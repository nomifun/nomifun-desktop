//! Shared application services for dependency injection.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nomifun_ai_agent::{
    AcpSessionSyncService, AcpSkillManager, AgentFactoryDeps, AgentRegistry, AgentRuntimeRegistry,
    InMemoryAgentRuntimeRegistry, build_agent_factory,
};
use nomifun_api_types::{GatewayMcpConfig, RequirementMcpConfig};
use nomifun_auth::{
    AuthPolicy, CompanionTokenValidator, CookieConfig, JwtService, QrTokenStore, resolve_jwt_secret,
};
use nomifun_common::OnConversationDelete;
use nomifun_conversation::runtime_state::ConversationRuntimeStateService;
use nomifun_conversation::{
    ExecutionConversationBoundary, RepositoryExecutionConversationBoundary,
};
use nomifun_db::{
    Database, IAcpSessionRepository, IAgentMetadataRepository, ICompanionTokenRepository,
    IConversationRepository, IMcpServerRepository, IProviderModelRepository, IProviderRepository,
    IUserRepository, SqliteAcpSessionRepository, SqliteAgentMetadataRepository,
    SqliteCompanionTokenRepository, SqliteConversationRepository, SqliteMcpServerRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, SqliteRemoteAgentRepository,
    SqliteTerminalRepository, SqliteUserRepository,
};
use nomifun_db::{IClientPreferenceRepository, SqliteClientPreferenceRepository};
use nomifun_realtime::{BroadcastEventBus, WebSocketManager};
use nomifun_terminal::{TerminalEventEmitter, TerminalLifecycleServer, TerminalService};

use crate::config::{AppConfig, load_or_create_data_encryption_key};

fn require_utf8_executable_path(path: &std::path::Path) -> anyhow::Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        anyhow::anyhow!(
            "backend executable path is not valid Unicode; refusing to configure child-process bridges or lifecycle hooks: {path:?}"
        )
    })
}

#[cfg(feature = "browser-use")]
struct BrowserPlatformTasks {
    sweep: tokio::task::JoinHandle<()>,
    events: tokio::task::JoinHandle<()>,
    telemetry: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "browser-use")]
impl Drop for BrowserPlatformTasks {
    fn drop(&mut self) {
        self.sweep.abort();
        self.events.abort();
        self.telemetry.abort();
    }
}

#[cfg(feature = "browser-use")]
fn browser_resource_telemetry_from_measurements(
    total_memory_bytes: u64,
    available_memory_bytes: u64,
    logical_cpus: Option<usize>,
    chromium_rss_bytes: Option<u64>,
    host_rss_by_process_id: std::collections::HashMap<u32, u64>,
    cpu_usage_percent: Option<f32>,
) -> nomifun_browser_platform::ResourceTelemetry {
    nomifun_browser_platform::ResourceTelemetry {
        total_memory_bytes,
        available_memory_bytes,
        logical_cpus: logical_cpus.unwrap_or(0),
        chromium_rss_bytes: chromium_rss_bytes.unwrap_or(0),
        cpu_pressure: cpu_usage_percent
            .map(browser_cpu_pressure_from_percent)
            .unwrap_or(0.0),
        // No cross-platform GPU collector is wired yet. Keep this explicitly
        // unknown instead of deriving a misleading approximation.
        gpu_pressure: None,
        host_rss_by_process_id,
    }
}

#[cfg(feature = "browser-use")]
fn browser_cpu_pressure_from_percent(cpu_usage_percent: f32) -> f64 {
    let pressure = f64::from(cpu_usage_percent) / 100.0;
    if pressure.is_finite() {
        pressure.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(feature = "browser-use")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct BrowserStartupPreferences {
    display_mode: &'static str,
    source: String,
    full_power: bool,
    persistent_login: bool,
}

#[cfg(feature = "browser-use")]
impl Default for BrowserStartupPreferences {
    fn default() -> Self {
        Self {
            // New installs default to truly silent Agent browsing: routine
            // Browser Use launches Chromium `--headless=new` and never opens
            // an operating-system window. A user may explicitly choose the
            // `external` default-visible policy in Settings; the removed
            // embedded viewer is never selected as a presentation surface.
            display_mode: "headless",
            source: "system".to_owned(),
            full_power: false,
            persistent_login: true,
        }
    }
}

/// Resolve the trusted application-level browser visibility policy.
///
/// The two supported values are user preferences, not Agent capabilities:
/// `headless` (default) keeps routine Primary work invisible; `external` is a
/// user's explicit choice to launch the Primary Host with a visible window.
/// Version 2 makes an external window an explicitly user-selected policy.
/// Any pre-versioned value (including a historical `external` value inferred
/// from the removed `silent=false` setting) is migrated once to `headless`.
/// Once the v2 marker is present, either valid user choice is preserved.
/// Missing or malformed v2 state fails closed to `headless` and is repaired.
#[cfg(feature = "browser-use")]
fn resolve_browser_display_mode(
    display_mode: Option<&str>,
    policy_version: Option<&str>,
) -> (&'static str, bool) {
    let is_current_version = policy_version
        .map(|value| value.trim().trim_matches('"') == BROWSER_DISPLAY_MODE_POLICY_VERSION)
        .unwrap_or(false);
    if !is_current_version {
        return ("headless", true);
    }

    match display_mode.map(|value| value.trim().trim_matches('"')) {
        Some("headless") => ("headless", false),
        Some("external") => ("external", false),
        _ => ("headless", true),
    }
}

#[cfg(feature = "browser-use")]
pub(crate) const BROWSER_DISPLAY_MODE_PREF_KEY: &str = "agent.browserUse.displayMode";
#[cfg(feature = "browser-use")]
pub(crate) const BROWSER_DISPLAY_MODE_VERSION_PREF_KEY: &str =
    "agent.browserUse.displayModeVersion";
#[cfg(feature = "browser-use")]
pub(crate) const BROWSER_DISPLAY_MODE_POLICY_VERSION: &str = "2";

#[cfg(feature = "browser-use")]
const BROWSER_STARTUP_PREFERENCE_KEYS: [&str; 5] = [
    BROWSER_DISPLAY_MODE_PREF_KEY,
    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
    "agent.browserUse.source",
    "agent.browserUse.fullPower",
    "agent.browserUse.persistentLogin",
];

#[cfg(feature = "browser-use")]
async fn load_browser_startup_preferences<R>(
    preference_repo: &R,
) -> BrowserStartupPreferences
where
    R: IClientPreferenceRepository + ?Sized,
{
    let preferences = match preference_repo
        .get_by_keys(&BROWSER_STARTUP_PREFERENCE_KEYS)
        .await
    {
        Ok(preferences) => preferences,
        Err(error) => {
            // A read failure is not the same as a fresh install. Keep the
            // fail-safe silent runtime default, but do not persist while the
            // authoritative preference store is unavailable.
            tracing::warn!(
                %error,
                "could not read persisted browser startup preferences; using fail-safe defaults without migration"
            );
            return BrowserStartupPreferences::default();
        }
    };

    let preference = |key: &str| {
        preferences
            .iter()
            .find(|row| row.key == key)
            .map(|row| row.value.as_str())
    };
    let (display_mode, persist_display_mode) = resolve_browser_display_mode(
        preference(BROWSER_DISPLAY_MODE_PREF_KEY),
        preference(BROWSER_DISPLAY_MODE_VERSION_PREF_KEY),
    );

    if persist_display_mode {
        // Persist only after a successful read. Mode plus marker form one
        // lineage boundary: a later explicit v2 choice is preserved, while
        // pre-v2 state cannot silently re-enable an operating-system window.
        let serialized =
            serde_json::to_string(display_mode).expect("browser display mode is static JSON");
        if let Err(error) = preference_repo
            .upsert_batch(&[
                (BROWSER_DISPLAY_MODE_PREF_KEY, serialized.as_str()),
                (
                    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                    BROWSER_DISPLAY_MODE_POLICY_VERSION,
                ),
            ])
            .await
        {
            tracing::warn!(
                %error,
                "could not persist migrated browser display mode"
            );
        }
    }

    BrowserStartupPreferences {
        display_mode,
        source: preference("agent.browserUse.source")
            .map(|value| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "system".to_owned()),
        full_power: preference("agent.browserUse.fullPower")
            .map(|value| value.trim().trim_matches('"') == "true")
            .unwrap_or(false),
        persistent_login: preference("agent.browserUse.persistentLogin")
            .map(|value| value.trim().trim_matches('"') != "false")
            .unwrap_or(true),
    }
}

#[cfg(feature = "browser-use")]
fn primary_host_is_headful(display_mode: &str) -> bool {
    // The trusted application-level preference is the only input that can
    // make the Primary Host launch visible; Agent tool JSON, lane names and
    // request parameters have no path into this policy. Non-Primary Hosts
    // stay headless regardless, and explicit foregrounding remains a separate
    // trusted Host transition owned by the Hub.
    display_mode == "external"
}

#[cfg(feature = "browser-use")]
fn browser_process_tree_rss<I>(
    root_pids: &[u32],
    processes: I,
) -> (
    Option<u64>,
    std::collections::HashMap<u32, u64>,
)
where
    I: IntoIterator<Item = (u32, Option<u32>, u64)>,
{
    use std::collections::{HashMap, HashSet};

    if root_pids.is_empty() {
        return (None, HashMap::new());
    }

    let mut rss_by_pid = HashMap::new();
    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, parent_pid, rss_bytes) in processes {
        rss_by_pid.insert(pid, rss_bytes);
        if let Some(parent_pid) = parent_pid {
            children_by_parent.entry(parent_pid).or_default().push(pid);
        }
    }

    let mut total_visited = HashSet::new();
    let mut host_rss_by_process_id = HashMap::new();
    let mut total_rss_bytes = 0_u64;
    for root_pid in root_pids.iter().copied().collect::<HashSet<_>>() {
        let mut pending = vec![root_pid];
        let mut host_visited = HashSet::new();
        let mut host_measured = false;
        let mut host_rss_bytes = 0_u64;
        while let Some(pid) = pending.pop() {
            if !host_visited.insert(pid) {
                continue;
            }
            if let Some(rss_bytes) = rss_by_pid.get(&pid) {
                host_measured = true;
                host_rss_bytes = host_rss_bytes.saturating_add(*rss_bytes);
                if total_visited.insert(pid) {
                    total_rss_bytes = total_rss_bytes.saturating_add(*rss_bytes);
                }
            }
            if let Some(children) = children_by_parent.get(&pid) {
                pending.extend(children.iter().copied());
            }
        }
        if host_measured {
            host_rss_by_process_id.insert(root_pid, host_rss_bytes);
        }
    }

    (
        (!total_visited.is_empty()).then_some(total_rss_bytes),
        host_rss_by_process_id,
    )
}

#[cfg(feature = "browser-use")]
const BROWSER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

type BrowserShutdownResult = Result<(), Arc<str>>;

struct BrowserShutdownFlight {
    result: tokio::sync::watch::Receiver<Option<BrowserShutdownResult>>,
}

impl BrowserShutdownFlight {
    fn new(result: tokio::sync::watch::Receiver<Option<BrowserShutdownResult>>) -> Self {
        Self { result }
    }

    async fn wait(&self) -> BrowserShutdownResult {
        let mut result = self.result.clone();
        loop {
            if let Some(result) = result.borrow().clone() {
                return result;
            }
            if result.changed().await.is_err() {
                return Err(Arc::from(
                    "managed browser platform shutdown worker ended without publishing a result",
                ));
            }
        }
    }
}

#[cfg(feature = "browser-use")]
#[derive(Default)]
struct BrowserShutdownCoordinatorState {
    flight: Option<Arc<BrowserShutdownFlight>>,
    succeeded: bool,
}

/// Process-wide Browser Hub shutdown authority.
///
/// The first caller starts a detached Hub-owned shutdown worker. Every
/// concurrent caller waits on that same flight, and timing out one waiter never
/// drops or cancels the real `hub.shutdown()` future. Success is cached;
/// failures clear the flight so a later caller can retry the Hub's retained
/// cleanup queues.
#[cfg(feature = "browser-use")]
#[derive(Clone)]
pub(crate) struct BrowserShutdownCoordinator {
    hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
    state: Arc<tokio::sync::Mutex<BrowserShutdownCoordinatorState>>,
}

#[cfg(feature = "browser-use")]
impl BrowserShutdownCoordinator {
    fn new(hub: Arc<nomifun_browser_platform::BrowserSessionHub>) -> Self {
        Self {
            hub,
            state: Arc::new(tokio::sync::Mutex::new(
                BrowserShutdownCoordinatorState::default(),
            )),
        }
    }

    pub(crate) async fn shutdown(&self) -> anyhow::Result<()> {
        self.shutdown_with_timeout(BROWSER_SHUTDOWN_TIMEOUT).await
    }

    async fn shutdown_with_timeout(&self, timeout: Duration) -> anyhow::Result<()> {
        let flight = self.current_or_start_flight().await;
        match tokio::time::timeout(timeout, flight.wait()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.clear_failed_flight(&flight).await;
                Err(anyhow::anyhow!("{error}"))
            }
            Err(_) => Err(anyhow::anyhow!(
                "managed browser platform shutdown timed out after {}",
                format_duration(timeout)
            )),
        }
    }

    async fn current_or_start_flight(&self) -> Arc<BrowserShutdownFlight> {
        let mut state = self.state.lock().await;
        if state.succeeded {
            let (_tx, rx) = tokio::sync::watch::channel(Some(Ok(())));
            return Arc::new(BrowserShutdownFlight::new(rx));
        }
        if let Some(flight) = state.flight.clone() {
            return flight;
        }

        let (result_tx, result_rx) = tokio::sync::watch::channel(None);
        let flight = Arc::new(BrowserShutdownFlight::new(result_rx));
        state.flight = Some(Arc::clone(&flight));

        let hub = Arc::clone(&self.hub);
        let coordinator_state = Arc::clone(&self.state);
        let active_flight = Arc::clone(&flight);
        tokio::spawn(async move {
            let result = match tokio::spawn(async move { hub.shutdown().await }).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(Arc::from(
                    format!(
                        "managed browser platform shutdown failed ({:?}): {}",
                        error.code, error.message
                    )
                    .into_boxed_str(),
                )),
                Err(error) => Err(Arc::from(
                    format!(
                        "managed browser platform shutdown worker failed (cancelled={}, panic={}): {error}",
                        error.is_cancelled(),
                        error.is_panic()
                    )
                    .into_boxed_str(),
                )),
            };

            // Publish the terminal result before mutating coordinator state.
            // Followers that already hold this flight must observe its exact
            // result even if a failed flight is cleared and a retry starts
            // immediately on another task.
            result_tx.send_replace(Some(result.clone()));
            {
                let mut state = coordinator_state.lock().await;
                if state
                    .flight
                    .as_ref()
                    .is_some_and(|flight| Arc::ptr_eq(flight, &active_flight))
                {
                    if result.is_ok() {
                        state.succeeded = true;
                    } else {
                        state.flight = None;
                    }
                }
            }
        });

        flight
    }

    async fn clear_failed_flight(&self, failed: &Arc<BrowserShutdownFlight>) {
        let mut state = self.state.lock().await;
        if state
            .flight
            .as_ref()
            .is_some_and(|flight| Arc::ptr_eq(flight, failed))
        {
            state.flight = None;
        }
    }
}

#[derive(Default)]
struct BrowserPlatformShutdownState {
    hub: Option<BrowserShutdownStep>,
    flight: Option<Arc<BrowserShutdownFlight>>,
    succeeded: bool,
}

type BrowserShutdownStepFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'static>,
>;

#[derive(Clone)]
struct BrowserShutdownStep {
    label: &'static str,
    run: Arc<dyn Fn() -> BrowserShutdownStepFuture + Send + Sync>,
}

impl BrowserShutdownStep {
    fn new<F, Fut>(label: &'static str, run: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        Self {
            label,
            run: Arc::new(move || Box::pin(run())),
        }
    }

    async fn execute(&self) -> Result<(), String> {
        (self.run)().await
    }
}

struct BrowserPlatformShutdownInner {
    gateway: Option<BrowserShutdownStep>,
    browser_mcp: tokio::sync::Mutex<Option<BrowserShutdownStep>>,
    state: tokio::sync::Mutex<BrowserPlatformShutdownState>,
}

/// Cloneable, process-wide authority for ordered Gateway/Browser shutdown.
///
/// Gateway is always present when startup succeeds, including builds without
/// `browser-use`. Browser-enabled builds add ACP Browser MCP and the Hub. One
/// shared flight first closes every configured ingress in parallel and waits
/// for authoritative quiescence/owner cleanup. Only a fully successful ingress
/// barrier may advance to Hub shutdown. This prevents Hub or DB teardown from
/// racing accepted requests, while a failed flight remains retryable.
#[derive(Clone)]
pub(crate) struct BrowserPlatformShutdown {
    inner: Arc<BrowserPlatformShutdownInner>,
}

impl Default for BrowserPlatformShutdown {
    fn default() -> Self {
        Self::from_steps(None, None)
    }
}

impl BrowserPlatformShutdown {
    #[cfg(not(feature = "browser-use"))]
    fn gateway_only(
        gateway: Option<Arc<nomifun_gateway::GatewayMcpServer>>,
    ) -> Self {
        Self::gateway_only_early(gateway)
    }

    fn gateway_only_early(
        gateway: Option<Arc<nomifun_gateway::GatewayMcpServer>>,
    ) -> Self {
        let gateway = gateway.map(|server| {
            BrowserShutdownStep::new("Gateway MCP ingress", move || {
                let server = Arc::clone(&server);
                async move { server.wait_for_shutdown().await }
            })
        });
        Self::from_steps(gateway, None)
    }

    #[cfg(feature = "browser-use")]
    async fn set_browser_mcp(
        &self,
        browser_mcp: Option<Arc<crate::browser_mcp_server::BrowserMcpServer>>,
    ) {
        let browser_mcp = browser_mcp.map(|server| {
            BrowserShutdownStep::new("ACP Browser MCP ingress", move || {
                let server = Arc::clone(&server);
                async move { server.stop_and_wait().await }
            })
        });
        self.set_browser_mcp_step(browser_mcp).await;
    }

    fn from_steps(
        gateway: Option<BrowserShutdownStep>,
        browser_mcp: Option<BrowserShutdownStep>,
    ) -> Self {
        Self {
            inner: Arc::new(BrowserPlatformShutdownInner {
                gateway,
                browser_mcp: tokio::sync::Mutex::new(browser_mcp),
                state: tokio::sync::Mutex::new(BrowserPlatformShutdownState::default()),
            }),
        }
    }

    /// Install the Hub shutdown authority before publishing the composed
    /// services. If an ingress-only shutdown raced composition, join that
    /// flight first and reopen the sequence so the newly installed Hub cannot
    /// be skipped by a cached success.
    #[cfg(feature = "browser-use")]
    async fn set_hub_coordinator(&self, coordinator: BrowserShutdownCoordinator) {
        let hub = BrowserShutdownStep::new("Browser Hub", move || {
            let coordinator = coordinator.clone();
            async move {
                coordinator
                    .shutdown()
                    .await
                    .map_err(|error| format!("{error:#}"))
            }
        });
        self.set_hub_step(hub).await;
    }

    #[cfg(feature = "browser-use")]
    async fn set_hub_step(&self, hub: BrowserShutdownStep) {
        loop {
            let active_flight = {
                let mut state = self.inner.state.lock().await;
                if state.succeeded {
                    state.succeeded = false;
                    state.flight = None;
                    state.hub = Some(hub.clone());
                    return;
                }
                match state.flight.clone() {
                    Some(flight) => Some(flight),
                    None => {
                        state.hub = Some(hub.clone());
                        return;
                    }
                }
            };

            let Some(active_flight) = active_flight else {
                unreachable!("Browser platform shutdown flight was checked above");
            };
            if active_flight.wait().await.is_err() {
                self.clear_failed_flight(&active_flight).await;
            }
            // The worker publishes before updating shared state. Yield once so
            // it can cache success or clear failure before this loop rechecks.
            tokio::task::yield_now().await;
        }
    }

    /// Register a newly started Browser MCP ingress without allowing an
    /// already-running or cached shutdown flight to omit it. Startup and
    /// shutdown normally run on one task, but fatal supervisors may initiate
    /// cleanup concurrently; use the same reopen/join protocol as Hub
    /// installation so cleanup success always covers every published ingress.
    async fn set_browser_mcp_step(&self, browser_mcp: Option<BrowserShutdownStep>) {
        loop {
            let active_flight = {
                let mut state = self.inner.state.lock().await;
                if state.succeeded {
                    state.succeeded = false;
                    state.flight = None;
                    *self.inner.browser_mcp.lock().await = browser_mcp.clone();
                    return;
                }
                if state.flight.is_none() {
                    *self.inner.browser_mcp.lock().await = browser_mcp.clone();
                    return;
                }
                state.flight.clone()
            };

            let Some(active_flight) = active_flight else {
                unreachable!("Browser platform shutdown flight was checked above");
            };
            if active_flight.wait().await.is_err() {
                self.clear_failed_flight(&active_flight).await;
            }
            tokio::task::yield_now().await;
        }
    }

    pub(crate) async fn shutdown(&self) -> anyhow::Result<()> {
        let flight = self.current_or_start_flight().await;
        match flight.wait().await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.clear_failed_flight(&flight).await;
                Err(anyhow::anyhow!("{error}"))
            }
        }
    }

    async fn current_or_start_flight(&self) -> Arc<BrowserShutdownFlight> {
        let mut state = self.inner.state.lock().await;
        if state.succeeded {
            let (_tx, rx) = tokio::sync::watch::channel(Some(Ok(())));
            return Arc::new(BrowserShutdownFlight::new(rx));
        }
        if let Some(flight) = state.flight.clone() {
            return flight;
        }

        let (result_tx, result_rx) = tokio::sync::watch::channel(None);
        let flight = Arc::new(BrowserShutdownFlight::new(result_rx));
        state.flight = Some(Arc::clone(&flight));

        let gateway = self.inner.gateway.clone();
        let browser_mcp = self.inner.browser_mcp.lock().await.clone();
        let hub = state.hub.clone();
        let inner = Arc::clone(&self.inner);
        let active_flight = Arc::clone(&flight);
        tokio::spawn(async move {
            let result = match tokio::spawn(run_browser_platform_shutdown(
                gateway,
                browser_mcp,
                hub,
            ))
            .await
            {
                Ok(result) => result,
                Err(error) => Err(Arc::from(
                    format!(
                        "managed browser platform shutdown worker failed (cancelled={}, panic={}): {error}",
                        error.is_cancelled(),
                        error.is_panic()
                    )
                    .into_boxed_str(),
                )),
            };

            // Publish first so every caller already attached to this exact
            // flight observes the same terminal result.
            result_tx.send_replace(Some(result.clone()));
            let mut state = inner.state.lock().await;
            if state
                .flight
                .as_ref()
                .is_some_and(|flight| Arc::ptr_eq(flight, &active_flight))
            {
                if result.is_ok() {
                    state.succeeded = true;
                } else {
                    state.flight = None;
                }
            }
        });

        flight
    }

    async fn clear_failed_flight(&self, failed: &Arc<BrowserShutdownFlight>) {
        let mut state = self.inner.state.lock().await;
        if state
            .flight
            .as_ref()
            .is_some_and(|flight| Arc::ptr_eq(flight, failed))
        {
            state.flight = None;
        }
    }
}

async fn run_browser_platform_shutdown(
    gateway: Option<BrowserShutdownStep>,
    browser_mcp: Option<BrowserShutdownStep>,
    hub: Option<BrowserShutdownStep>,
) -> BrowserShutdownResult {
    let (gateway_result, browser_mcp_result) = tokio::join!(
        await_browser_shutdown_step(gateway),
        await_browser_shutdown_step(browser_mcp)
    );

    let mut ingress_errors = Vec::new();
    if let Err(error) = gateway_result {
        ingress_errors.push(error);
    }
    if let Err(error) = browser_mcp_result {
        ingress_errors.push(error);
    }

    if !ingress_errors.is_empty() {
        // A failed or unconfirmed ingress barrier means an accepted request may
        // still own Hub work. Do not destroy that authority underneath it. The
        // composed flight is cleared by the caller so a later shutdown attempt
        // can rejoin/retry the ingress authorities and only then advance.
        return Err(Arc::from(ingress_errors.join("; ").into_boxed_str()));
    }

    await_browser_shutdown_step(hub)
        .await
        .map_err(|error| Arc::from(error.into_boxed_str()))
}

async fn await_browser_shutdown_step(step: Option<BrowserShutdownStep>) -> Result<(), String> {
    let Some(step) = step else {
        return Ok(());
    };
    let label = step.label;
    match tokio::spawn(async move { step.execute().await }).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("{label} shutdown failed: {error}")),
        Err(error) => Err(format!(
            "{label} shutdown task failed (cancelled={}, panic={}): {error}",
            error.is_cancelled(),
            error.is_panic()
        )),
    }
}

#[cfg(feature = "browser-use")]
fn format_duration(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        if duration.as_secs() > 0 {
            return format!("{} seconds", duration.as_secs());
        }
        return format!("{} milliseconds", duration.as_millis());
    }
    format!("{duration:?}")
}

#[cfg(feature = "browser-use")]
#[derive(Clone, Debug, PartialEq, Eq)]
enum BrowserOrphanRecoveryOutcome {
    Safe { summary: String },
    Degraded { reason: String },
}

#[cfg(feature = "browser-use")]
impl BrowserOrphanRecoveryOutcome {
    fn from_report(report: &nomi_browser_engine::profile::ProfileRecoveryReport) -> Self {
        let summary = report.safety_summary();
        // Resolved markers are never failures: profiles preserved for a live
        // verified owner, terminated orphan trees, removed ephemeral profiles,
        // and cleared stable markers all leave failures/profiles_preserved at
        // zero on every platform. Only genuinely unresolved state (scan or
        // identity-verification or termination or cleanup failures, each of
        // which also preserves the affected profile) degrades fail closed.
        if report.failures == 0 && report.profiles_preserved == 0 {
            Self::Safe { summary }
        } else {
            Self::Degraded {
                reason: format!(
                    "startup orphan recovery was not proven safe ({summary}); Browser functionality is disabled for this process"
                ),
            }
        }
    }

    fn from_join_error(error: &tokio::task::JoinError) -> Self {
        Self::Degraded {
            reason: format!(
                "startup orphan recovery worker failed (cancelled={}, panic={}): {error}; Browser functionality is disabled for this process",
                error.is_cancelled(),
                error.is_panic()
            ),
        }
    }

    fn permits_host_composition(&self) -> bool {
        matches!(self, Self::Safe { .. })
    }

    fn is_safe(&self) -> bool {
        self.permits_host_composition()
    }
}

#[cfg(feature = "browser-use")]
fn persisted_identity_seed_coverage() -> nomifun_browser_platform::SnapshotCoverage {
    nomifun_browser_platform::SnapshotCoverage::cookies_only()
}

#[cfg(feature = "browser-use")]
fn sample_browser_resources(
    system: &mut sysinfo::System,
    root_pids: &[u32],
    cpu_usage_percent: Option<f32>,
) -> nomifun_browser_platform::ResourceTelemetry {
    system.refresh_memory();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );
    let (chromium_rss_bytes, host_rss_by_process_id) = browser_process_tree_rss(
        root_pids,
        system.processes().values().map(|process| {
            (
                process.pid().as_u32(),
                process.parent().map(|pid| pid.as_u32()),
                process.memory(),
            )
        }),
    );
    browser_resource_telemetry_from_measurements(
        system.total_memory(),
        system.available_memory(),
        std::thread::available_parallelism()
            .ok()
            .map(std::num::NonZeroUsize::get),
        chromium_rss_bytes,
        host_rss_by_process_id,
        cpu_usage_percent,
    )

}

#[cfg(feature = "browser-use")]
const BROWSER_INVENTORY_EVENT_NAME: &str = "browser.inventory.changed";
#[cfg(feature = "browser-use")]
const BROWSER_INVENTORY_RESYNC_CHANGE_KIND: &str = "resync_required";

/// Build a protocol-compatible invalidation event for a lossy inventory hop.
///
/// This intentionally carries no synthetic `sequence`: only the Hub owns that
/// counter, and inventing one here could hide a real gap. Existing clients
/// already refresh on every inventory event; newer clients can use the
/// additive marker to explicitly classify the refresh as a full resync.
#[cfg(feature = "browser-use")]
fn browser_inventory_resync_event(
    skipped: u64,
) -> nomifun_api_types::WebSocketMessage<serde_json::Value> {
    nomifun_api_types::WebSocketMessage::new(
        BROWSER_INVENTORY_EVENT_NAME,
        serde_json::json!({
            "change_kind": BROWSER_INVENTORY_RESYNC_CHANGE_KIND,
            "resync_required": true,
            "skipped": skipped,
        }),
    )
}

#[cfg(feature = "browser-use")]
async fn forward_browser_inventory_events(
    mut receiver: tokio::sync::broadcast::Receiver<
        nomifun_browser_platform::BrowserInventoryEvent,
    >,
    event_bus: Arc<BroadcastEventBus>,
    ws_manager: Arc<WebSocketManager>,
    installation_owner: Arc<str>,
) {
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::UserEventSink;
    use tokio::sync::broadcast::error::RecvError;

    loop {
        let event = match receiver.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "browser inventory realtime forwarder lagged");
                // Hub events are user-scoped, but Tokio only reports a count
                // for discarded entries, not their audiences. The invalidation
                // contains no inventory data, so safely send it to every
                // connection; each client refreshes its own authenticated
                // snapshot. Deliver directly to the socket manager so the
                // signal cannot be dropped by the same intermediate bus.
                ws_manager.broadcast_all(browser_inventory_resync_event(skipped));
                continue;
            }
            Err(RecvError::Closed) => break,
        };
        let audience = event
            .user_id
            .as_deref()
            .unwrap_or(installation_owner.as_ref());
        let payload = serde_json::json!({
            "sequence": event.sequence,
            "change_kind": event.change_kind,
            "lane_id": event.lane_id,
            "conversation_id": event.conversation_id,
            "at_ms": event.at_ms,
        });
        event_bus.send_to_user(
            audience,
            WebSocketMessage::new(BROWSER_INVENTORY_EVENT_NAME, payload.clone()),
        );
        if matches!(
            event.change_kind.as_str(),
            "lane_created"
                | "lane_running"
                | "lane_failed"
                | "lane_stopping"
                | "lane_closed"
                | "platform_stopped"
        ) {
            event_bus.send_to_user(
                audience,
                WebSocketMessage::new("browser.lifecycle.changed", payload),
            );
        }
    }
}

pub struct AppServices {
    pub database: Database,
    /// Present only when the process owns the canonical OS server lock for the
    /// exact data directory backing `database`. Boot orphan reconciliation is
    /// forbidden without this retained authority.
    pub(crate) _boot_reconciliation_authority:
        Option<crate::bootstrap::BootServerLockAuthority>,
    /// Process-local barrier covering Provider lifecycle operations that span
    /// SQLite and JSON side stores (companions).
    pub provider_lifecycle: nomifun_common::SharedProviderLifecycleBarrier,
    /// Canonical owner of every installation-scoped resource. Resolved once
    /// through `installation_identity` at boot; usernames are mutable display
    /// data and must never be used as an authorization identity.
    pub authoritative_user_id: Arc<str>,
    pub jwt_service: Arc<JwtService>,
    pub user_repo: Arc<dyn IUserRepository>,
    /// Per-companion Remote front-door token store (SHA-256 hashes).
    pub companion_token_repo: Arc<dyn ICompanionTokenRepository>,
    /// In-memory validator mapping token -> companion_id (hot-swapped on mint/revoke).
    pub companion_token_validator: Arc<CompanionTokenValidator>,
    /// Provider repository (exposed for the mint-time model-availability guard).
    pub provider_repo: Arc<dyn IProviderRepository>,
    /// Unified loopback supply for NomiFun's managed free models.
    pub managed_model_service: Arc<nomifun_system::ManagedModelService>,
    /// Keeps the authenticated loopback OpenAI-compatible listener alive.
    pub(crate) _managed_model_server: nomifun_system::ManagedModelServer,
    /// Keeps the immediate + periodic managed catalog refresh loop alive.
    pub(crate) _managed_model_refresh_task: nomifun_system::ManagedModelRefreshTask,
    /// Authoritative per-model catalog rows (capability profiles + health;
    /// the multimodal model hub reads/writes these).
    pub provider_model_repo: Arc<dyn IProviderModelRepository>,
    pub cookie_config: Arc<CookieConfig>,
    pub qr_token_store: Arc<QrTokenStore>,
    pub ws_manager: Arc<WebSocketManager>,
    pub event_bus: Arc<BroadcastEventBus>,
    pub agent_runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    pub conversation_runtime_state: Arc<ConversationRuntimeStateService>,
    /// Same instance as `agent_runtime_registry`, exposed through the
    /// `OnConversationDelete` trait so `ConversationService::with_delete_hook`
    /// can wire it up. Optional because tests construct `AppServices` with a
    /// mock `agent_runtime_registry` that does not implement the trait.
    pub runtime_registry_delete_hook: Option<Arc<dyn OnConversationDelete>>,
    pub agent_registry: Arc<AgentRegistry>,
    pub conversation_repo: Arc<dyn IConversationRepository>,
    /// One mandatory Conversation↔Execution authority shared by every
    /// production ConversationService instance. Keeping it in AppServices
    /// makes incomplete module-specific assembly impossible.
    pub execution_conversation_boundary: Arc<dyn ExecutionConversationBoundary>,
    /// Singleton requirement service (shares its repo + WS emitter with the
    /// nomi native-tool sink). The router state attaches a `ConversationService`
    /// to a clone of this for AutoWork config persistence.
    pub requirement_service: Arc<nomifun_requirement::RequirementService>,
    /// Singleton terminal service: owns the live PTYs (one in-memory map). Shared
    /// so the AutoWork runner drives the SAME PTYs the terminal routes
    /// created (a fresh instance would have an empty live map).
    pub terminal_service: Arc<TerminalService>,
    pub acp_session_sync: Arc<AcpSessionSyncService>,
    /// Raw JWT secret string, used only for authentication/session signing.
    pub jwt_secret_raw: String,
    /// Persistent AES-256-GCM key for encrypted app data.
    pub encryption_key: [u8; 32],
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
    pub work_dir_is_cli_override: bool,
    /// Authentication policy (single source of truth, replaces `local: bool`).
    pub auth_policy: AuthPolicy,
    /// Per-boot secret the desktop's own webview presents to be trusted as the
    /// local client. Only `Some` under `AuthPolicy::TrustLocalToken`.
    pub local_trust_secret: Option<Arc<str>>,
    pub app_version: String,
    /// Resolved skill paths. Shared with the `ConversationService` for
    /// snapshot resolution at create time.
    pub skill_paths: Arc<nomifun_extension::SkillPaths>,
    /// Process-private Requirement MCP issuer (port, root secret, binary path).
    /// It is non-serializable; only per-session child capabilities leave the
    /// main process. `None` when the server failed to start. Its presence drives
    /// `AutoWorkRunnerDeps::requirement_mcp_enabled` so the ACP verdict gate stays
    /// in lock-step with whether the declaration tools are actually injected.
    pub requirement_mcp_config: Option<RequirementMcpConfig>,
    /// Requirement MCP server instance kept alive for the app lifetime.
    pub(crate) _requirement_mcp_server: Option<nomifun_requirement::RequirementMcpServer>,
    /// Process-private Platform Gateway issuer (port, root secret, binary path,
    /// installation owner). It is non-serializable; only short-lived signed
    /// child capabilities leave the main process. `None` when the server failed
    /// to start, so Agent sessions simply lack the `nomi_*` tools.
    pub gateway_mcp_config: Option<GatewayMcpConfig>,
    /// Platform Gateway MCP server instance kept alive for the app lifetime.
    /// Its deps are late-wired from `create_router` via
    /// [`AppServices::inject_gateway_deps`] once the module services exist.
    pub(crate) _gateway_mcp_server: Option<Arc<nomifun_gateway::GatewayMcpServer>>,
    /// Knowledge MCP server instance kept alive for the app lifetime. Its
    /// presence (surfaced to the agent factory as `knowledge_mcp_config`) gates
    /// scoped knowledge tool injection into ACP sessions that have bound bases.
    /// Its root issuer stays in-process; child capabilities independently scope
    /// search/read/write. `None` when startup fails (graceful degradation).
    pub(crate) _knowledge_mcp_server: Option<nomifun_knowledge::KnowledgeMcpServer>,
    /// Singleton companion service (nomi desktop companion). Built before the agent
    /// factory so the factory can register the companion memory tools for
    /// companion_session conversations; the router reuses this same instance.
    pub companion_service: Arc<nomifun_companion::CompanionService>,
    /// 客服独立域 CRUD service (agents / notes / bindings).
    pub customer_service_service: Arc<nomifun_customer_service::CustomerServiceService>,
    /// 客服无状态并发回合执行器 (channel seam target).
    pub cs_dialogue_engine: Arc<nomifun_customer_service::CsDialogueEngine>,
    /// Singleton 创意工坊 (Creative Workshop) service — canvas/asset CRUD +
    /// on-disk canvas docs / asset binaries under `{data_dir}/workshop/`. Shared
    /// by the `/api/workshop/*` routes.
    pub workshop_service: Arc<nomifun_workshop::WorkshopService>,
    /// Singleton 生成引擎 (creation) service — the media generation task queue
    /// behind the workshop canvas. Shared by the `/api/creation/*` routes.
    pub creation_service: Arc<nomifun_creation::CreationService>,
    /// Singleton unified multimodal invoke layer (P1 redesign): catalog
    /// resolution + protocol adapters over the shared proxy-aware HTTP client.
    /// Shared by `/api/tts` today; later tasks (media/probe rewiring) reuse it.
    pub model_invoke_service: Arc<nomifun_model_invoke::ModelInvokeService>,
    /// Singleton knowledge service (knowledge base platform). Shared between
    /// the `/api/knowledge/*` routes and the `ConversationService`, which
    /// mounts bound bases into session workspaces at task start.
    pub knowledge_service: Arc<nomifun_knowledge::KnowledgeService>,
    /// The process-wide browser authority. Browser-capable hosts inject one
    /// Hub at the composition root; routes and every agent transport reuse it.
    /// `None` is an explicit unsupported/degraded state and never triggers a
    /// handler-local Chromium launch.
    #[cfg(feature = "browser-use")]
    pub browser_session_hub: Option<Arc<nomifun_browser_platform::BrowserSessionHub>>,
    /// Ordered, cloneable Gateway/Browser shutdown authority used by every
    /// process entry point. It exists in builds without `browser-use` because
    /// the Platform Gateway is unconditional and must quiesce before the DB.
    pub(crate) browser_platform_shutdown: BrowserPlatformShutdown,
    /// One-shot bridge shared with the already-built Agent factory. Installing
    /// the Hub-backed provider here makes Native Nomi use the exact same
    /// process-wide Hub as HTTP management, Gateway, ACP, and knowledge fetching.
    #[cfg(feature = "browser-use")]
    _browser_lane_provider_slot:
        nomifun_ai_agent::BrowserLaneClientProviderSlot,
    /// Keeps the authenticated ACP browser loopback proxy alive. Its issuer is
    /// process-private; child runtimes receive only scoped capabilities.
    #[cfg(feature = "browser-use")]
    pub(crate) _browser_mcp_server: Option<Arc<crate::browser_mcp_server::BrowserMcpServer>>,
    /// Owns the Hub lifecycle sweep and user-scoped realtime forwarding loops.
    /// Dropping AppServices aborts both loops instead of detaching them.
    #[cfg(feature = "browser-use")]
    _browser_platform_tasks: Option<BrowserPlatformTasks>,
}

pub(crate) struct RetainedAppServicesStartupError {
    services: AppServices,
    error: anyhow::Error,
}

impl RetainedAppServicesStartupError {
    fn new(services: AppServices, error: anyhow::Error) -> Self {
        Self { services, error }
    }

    pub(crate) fn into_parts(self) -> (AppServices, anyhow::Error) {
        (self.services, self.error)
    }
}

pub(crate) struct RetainedAppServicesConstructionError {
    error: anyhow::Error,
    cleanup_error: Option<anyhow::Error>,
    authority: Option<Arc<StartupCleanupAuthority>>,
}

impl RetainedAppServicesConstructionError {
    fn verified(error: anyhow::Error) -> Self {
        Self {
            error,
            cleanup_error: None,
            authority: None,
        }
    }

    fn new(
        error: anyhow::Error,
        cleanup_error: anyhow::Error,
        authority: Arc<StartupCleanupAuthority>,
    ) -> Self {
        Self {
            error,
            cleanup_error: Some(cleanup_error),
            authority: Some(authority),
        }
    }

    pub(crate) async fn retry_cleanup(&self) -> anyhow::Result<()> {
        match &self.authority {
            Some(authority) => authority.cleanup().await,
            None => Ok(()),
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        anyhow::Error,
        Option<anyhow::Error>,
        Option<Arc<StartupCleanupAuthority>>,
    ) {
        (self.error, self.cleanup_error, self.authority)
    }
}

impl std::fmt::Debug for RetainedAppServicesConstructionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedAppServicesConstructionError")
            .field("error", &self.error)
            .field("cleanup_error", &self.cleanup_error)
            .field(
                "authority",
                &self.authority.as_ref().map(|_| "<retained>"),
            )
            .finish()
    }
}

impl std::fmt::Display for RetainedAppServicesConstructionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:#}{}",
            self.error,
            self.cleanup_error
                .as_ref()
                .map(|error| format!("; managed browser platform cleanup remains unverified: {error:#}"))
                .unwrap_or_default()
        )
    }
}

impl std::error::Error for RetainedAppServicesConstructionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

/// Startup-only cleanup authority used while `AppServices` is still being
/// composed.
///
/// `from_config_inner` starts loopback ingress servers before all of the
/// remaining fallible composition work has completed.  The ordinary Rust
/// drop path is not a proof that those ingresses have stopped, so the
/// database must stay open until the shared Browser/Gateway barrier confirms
/// quiescence.  This small authority is deliberately independent of
/// `AppServices`: it can outlive a failed composition and retain the exact
/// ingress/Hub handles needed for a later retry.
pub(crate) struct StartupCleanupAuthority {
    database: Database,
    browser_platform_shutdown: tokio::sync::Mutex<Option<BrowserPlatformShutdown>>,
    retry_worker_started: AtomicBool,
}

impl StartupCleanupAuthority {
    fn new(database: Database) -> Self {
        Self {
            database,
            browser_platform_shutdown: tokio::sync::Mutex::new(None),
            retry_worker_started: AtomicBool::new(false),
        }
    }

    async fn install_browser_platform(&self, shutdown: BrowserPlatformShutdown) {
        *self.browser_platform_shutdown.lock().await = Some(shutdown);
    }

    async fn browser_platform_shutdown(&self) -> Option<BrowserPlatformShutdown> {
        self.browser_platform_shutdown.lock().await.clone()
    }

    /// Close the managed ingress first and the database second.
    ///
    /// A failed ingress shutdown intentionally leaves the database open.  The
    /// caller retains this authority and can invoke the same method again;
    /// `BrowserPlatformShutdown` provides the single-flight/idempotent retry
    /// semantics for the actual Gateway, Browser MCP, and Hub owners.
    pub(crate) async fn cleanup(&self) -> anyhow::Result<()> {
        let browser_cleanup = match self.browser_platform_shutdown().await {
            Some(shutdown) => shutdown.shutdown().await,
            None => Ok(()),
        };
        close_database_after_browser_platform_cleanup(browser_cleanup, || self.database.close())
            .await
    }

    fn retain_retry_worker(self: &Arc<Self>) {
        if self
            .retry_worker_started
            .swap(true, Ordering::AcqRel)
        {
            return;
        }

        let authority = Arc::clone(self);
        tokio::spawn(async move {
            let mut delay = Duration::from_millis(250);
            loop {
                tokio::time::sleep(delay).await;
                match authority.cleanup().await {
                    Ok(()) => {
                        tracing::info!(
                            "retained startup cleanup authority completed after retry"
                        );
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "retained startup cleanup authority is still pending; retrying"
                        );
                        delay = (delay * 2).min(Duration::from_secs(5));
                    }
                }
            }
        });
    }
}

async fn finish_startup_cleanup_typed(
    authority: Arc<StartupCleanupAuthority>,
    error: anyhow::Error,
) -> Result<anyhow::Error, RetainedAppServicesConstructionError> {
    match authority.cleanup().await {
        Ok(()) => Ok(error),
        Err(cleanup_error) => Err(RetainedAppServicesConstructionError::new(
            error,
            cleanup_error,
            authority,
        )),
    }
}

async fn finish_startup_cleanup(
    authority: Arc<StartupCleanupAuthority>,
    error: anyhow::Error,
) -> anyhow::Error {
    match finish_startup_cleanup_typed(authority, error).await {
        Ok(error) => error,
        Err(retained) => {
            let (error, cleanup_error, authority) = retained.into_parts();
            // Compatibility callers may immediately drop the returned anyhow
            // error. Retain the exact ingress/Hub handles and open database in
            // a detached retry worker as well as in the downcastable error.
            match (cleanup_error, authority) {
                (Some(cleanup_error), Some(authority)) => {
                    authority.retain_retry_worker();
                    anyhow::Error::new(RetainedStartupCleanupError::new(
                        error,
                        cleanup_error,
                        authority,
                    ))
                }
                _ => anyhow::anyhow!(
                    "{error:#}; startup cleanup state was internally inconsistent"
                ),
            }
        }
    }
}

/// Error returned by `from_config` when startup cleanup could not yet be
/// verified.
///
/// The authority is retained both in this error (for a host that wants to
/// supervise retries) and by a bounded background retry worker.  This keeps
/// old callers safe even if they immediately propagate and drop the error:
/// neither the managed ingress handles nor the database are abandoned before
/// cleanup succeeds.
pub(crate) struct RetainedStartupCleanupError {
    error: anyhow::Error,
    cleanup_error: anyhow::Error,
    authority: Arc<StartupCleanupAuthority>,
}

impl RetainedStartupCleanupError {
    fn new(
        error: anyhow::Error,
        cleanup_error: anyhow::Error,
        authority: Arc<StartupCleanupAuthority>,
    ) -> Self {
        Self {
            error,
            cleanup_error,
            authority,
        }
    }

    pub(crate) fn authority(&self) -> Arc<StartupCleanupAuthority> {
        Arc::clone(&self.authority)
    }

    pub(crate) async fn retry_cleanup(&self) -> anyhow::Result<()> {
        self.authority.cleanup().await
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        anyhow::Error,
        anyhow::Error,
        Arc<StartupCleanupAuthority>,
    ) {
        (self.error, self.cleanup_error, self.authority)
    }
}

impl std::fmt::Debug for RetainedStartupCleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedStartupCleanupError")
            .field("error", &self.error)
            .field("cleanup_error", &self.cleanup_error)
            .field("authority", &"<retained>")
            .finish()
    }
}

impl std::fmt::Display for RetainedStartupCleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:#}; managed browser platform cleanup remains unverified: {:#}",
            self.error, self.cleanup_error
        )
    }
}

impl std::error::Error for RetainedStartupCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

impl AppServices {
    /// Bind the process server-lock authority to these exact services.
    ///
    /// Both the configured data directory and SQLite's live `main` database
    /// are compared by OS file identity before the authority is retained. This
    /// prevents a lock for directory A from authorizing the boot classification
    /// sweep in a database opened from directory B, without rejecting valid
    /// path aliases. The authority is not process-tree termination proof.
    pub async fn with_boot_reconciliation_authority(
        self,
        authority: crate::bootstrap::BootServerLockAuthority,
        config: &AppConfig,
    ) -> anyhow::Result<Self> {
        match self
            .try_with_boot_reconciliation_authority(authority, config)
            .await
        {
            Ok(services) => Ok(services),
            Err(failure) => {
                let (services, error) = failure.into_parts();
                Err(services.cleanup_after_startup_failure(error).await)
            }
        }
    }

    /// Desktop startup needs to retain the exact services that still own
    /// browser/Gateway cleanup when this late boot stage fails. Other entry
    /// points use [`Self::with_boot_reconciliation_authority`], which preserves
    /// the historical "cleanup before returning" behavior.
    pub(crate) async fn try_with_boot_reconciliation_authority(
        mut self,
        authority: crate::bootstrap::BootServerLockAuthority,
        config: &AppConfig,
    ) -> Result<Self, RetainedAppServicesStartupError> {
        let config_dir_protected = match authority.protects_data_dir(&config.data_dir) {
            Ok(value) => value,
            Err(error) => {
                return Err(RetainedAppServicesStartupError::new(self, error));
            }
        };
        let services_dir_protected = match authority.protects_data_dir(&self.data_dir) {
            Ok(value) => value,
            Err(error) => {
                return Err(RetainedAppServicesStartupError::new(self, error));
            }
        };
        if !config_dir_protected || !services_dir_protected {
            let data_dir = self.data_dir.display().to_string();
            return Err(RetainedAppServicesStartupError::new(
                self,
                anyhow::anyhow!(
                    "boot reconciliation authority does not protect AppServices data directory {}",
                    data_dir
                ),
            ));
        }

        let database_protected = match authority
            .protects_database(&self.database, &config.database_path())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                return Err(RetainedAppServicesStartupError::new(self, error));
            }
        };
        if !database_protected {
            return Err(RetainedAppServicesStartupError::new(
                self,
                anyhow::anyhow!(
                    "boot reconciliation authority/database mismatch for data directory {}",
                    config.data_dir.display()
                ),
            ));
        }

        self._boot_reconciliation_authority = Some(authority);
        if let Err(error) = self.requirement_service.recover_pending_attachment_deletes().await {
            return Err(RetainedAppServicesStartupError::new(
                self,
                anyhow::anyhow!(
                    "attachment delete-journal boot reconciliation failed: {error}"
                ),
            ));
        }
        Ok(self)
    }

    pub(crate) async fn has_valid_boot_reconciliation_authority(
        &self,
    ) -> anyhow::Result<bool> {
        let Some(authority) = self._boot_reconciliation_authority.as_ref() else {
            return Ok(false);
        };
        Ok(authority.protects_data_dir(&self.data_dir)?
            && authority
                .protects_database(
                    &self.database,
                    &self.data_dir.join("nomifun-backend.db"),
                )
                .await?)
    }

    /// Replace the process-local Agent runtime registry after construction.
    ///
    /// Primarily used by tests to inject mock implementations.
    pub fn with_agent_runtime_registry(mut self, runtime_registry: Arc<dyn AgentRuntimeRegistry>) -> Self {
        self.agent_runtime_registry = runtime_registry;
        self
    }

    /// Explicitly stop Gateway plus the optional managed Browser platform.
    /// Teardown is asynchronous and must be confirmed before an entry point
    /// closes the database or reports startup/shutdown success; relying on
    /// `Drop` is insufficient even when `browser-use` is disabled.
    pub async fn shutdown_browser_platform(&self) -> anyhow::Result<()> {
        self.browser_platform_shutdown.shutdown().await
    }

    /// Close browser resources and the database after a startup-stage failure,
    /// preserving the original failure as the primary error.
    pub async fn cleanup_after_startup_failure(&self, error: anyhow::Error) -> anyhow::Error {
        let authority = Arc::new(StartupCleanupAuthority::new(self.database.clone()));
        authority
            .install_browser_platform(self.browser_platform_shutdown.clone())
            .await;
        finish_startup_cleanup(authority, error).await
    }

    /// Inject the one main-process browser authority.
    ///
    /// Kept as a builder so the native host can compose the Chromium adapter
    /// after resolving its bundled executable and encrypted identity vault.
    #[cfg(feature = "browser-use")]
    pub(crate) async fn with_browser_session_hub(
        mut self,
        hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
    ) -> anyhow::Result<Self> {
        use tokio::time::Duration;

        let shutdown_coordinator = BrowserShutdownCoordinator::new(Arc::clone(&hub));
        self.browser_platform_shutdown
            .set_hub_coordinator(shutdown_coordinator)
            .await;
        if let Err(error) = self._browser_lane_provider_slot.install(Arc::new(
            crate::browser_lane_provider::HubBrowserLaneClientProvider::new(
                Arc::clone(&hub),
                Arc::clone(&self.execution_conversation_boundary),
                Duration::from_secs(60),
                Arc::clone(&self.authoritative_user_id),
            ),
        )) {
            let cleanup_error = self.browser_platform_shutdown.shutdown().await.err();
            if cleanup_error.is_none() {
                self.database.close().await;
            }
            let error = anyhow::Error::new(error);
            return Err(match cleanup_error {
                Some(cleanup_error) => anyhow::anyhow!(
                    "{error:#}; managed browser platform cleanup after composition failure also failed: {cleanup_error:#}"
                ),
                None => error,
            });
        }

        if let Some(server) = &self._browser_mcp_server {
            server.set_hub(Arc::downgrade(&hub));
        }

        let sweep_hub = Arc::clone(&hub);
        let sweep = tokio::spawn(async move {
            loop {
                // Read the live policy for every cycle. Resource-policy
                // updates may change the lifecycle cadence, and a fixed
                // application-level interval would silently ignore that
                // setting until restart.
                let sweep_period_ms = sweep_hub
                    .resource_policy()
                    .await
                    .lifecycle_sweep_period_ms
                    .max(1);
                // The first sleep intentionally avoids an eager startup sweep
                // before runtimes have finished attaching their owner leases.
                tokio::time::sleep(Duration::from_millis(sweep_period_ms)).await;
                if let Err(error) = sweep_hub.sweep().await {
                    tracing::warn!(
                        code = ?error.code,
                        retryable = error.retryable,
                        "browser lifecycle sweep failed"
                    );
                }
            }
        });

        let events_rx = hub.subscribe();
        let event_bus = self.event_bus.clone();
        let ws_manager = self.ws_manager.clone();
        let installation_owner = self.authoritative_user_id.clone();
        let events = tokio::spawn(forward_browser_inventory_events(
            events_rx,
            event_bus,
            ws_manager,
            installation_owner,
        ));

        let telemetry_hub = Arc::clone(&hub);
        let telemetry = tokio::spawn(async move {
            let mut system = sysinfo::System::new();
            // CPU usage is delta-based. This first refresh establishes the
            // baseline; the immediate startup sample deliberately leaves CPU
            // pressure unknown while still publishing memory and process RSS.
            system.refresh_cpu_usage();
            let root_pids = telemetry_hub.managed_host_process_ids().await;
            let initial_sample = sample_browser_resources(&mut system, &root_pids, None);
            telemetry_hub
                .update_resource_telemetry(initial_sample)
                .await;
            loop {
                let sample_period_ms = telemetry_hub.resource_policy().await.sample_period_ms;
                // Honor the policy period while preserving sysinfo's minimum
                // delta window for an accurate CPU sample.
                tokio::time::sleep(
                    Duration::from_millis(sample_period_ms.max(1))
                        .max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL),
                )
                .await;
                system.refresh_cpu_usage();
                let root_pids = telemetry_hub.managed_host_process_ids().await;
                let cpu_usage_percent = system.global_cpu_usage();
                let sample = sample_browser_resources(
                    &mut system,
                    &root_pids,
                    Some(cpu_usage_percent),
                );
                telemetry_hub.update_resource_telemetry(sample).await;
            }
        });

        self._browser_platform_tasks = Some(BrowserPlatformTasks {
            sweep,
            events,
            telemetry,
        });
        self.browser_session_hub = Some(hub);
        Ok(self)
    }

    /// Wire the dependency bundle into the Platform Gateway MCP server.
    /// Called from `create_router` after `build_module_states` (the
    /// `ConversationService` / `CronService` instances live there).
    pub(crate) async fn inject_gateway_deps(&self, deps: Arc<nomifun_gateway::GatewayDeps>) {
        if let Some(server) = &self._gateway_mcp_server {
            server.set_deps(deps).await;
        }
    }

    pub async fn from_config(database: Database, config: &AppConfig) -> anyhow::Result<Self> {
        match Self::try_from_config(database, config).await {
            Ok(services) => Ok(services),
            Err(failure) => {
                let (error, cleanup_error, authority) = failure.into_parts();
                match (cleanup_error, authority) {
                    (None, None) => Err(error),
                    (Some(cleanup_error), Some(authority)) => {
                        authority.retain_retry_worker();
                        Err(anyhow::Error::new(RetainedStartupCleanupError::new(
                            error,
                            cleanup_error,
                            authority,
                        )))
                    }
                    _ => Err(anyhow::anyhow!(
                        "{error:#}; startup cleanup state was internally inconsistent"
                    )),
                }
            }
        }
    }

    pub(crate) async fn try_from_config(
        database: Database,
        config: &AppConfig,
    ) -> Result<Self, RetainedAppServicesConstructionError> {
        let startup_cleanup_authority =
            Arc::new(StartupCleanupAuthority::new(database.clone()));
        match Self::from_config_inner(
            database,
            config,
            Arc::clone(&startup_cleanup_authority),
        )
        .await
        {
            Ok(services) => Ok(services),
            Err(error) => match finish_startup_cleanup_typed(startup_cleanup_authority, error).await {
                Ok(error) => Err(RetainedAppServicesConstructionError::verified(error)),
                Err(retained) => Err(retained),
            },
        }
    }

    async fn from_config_inner(
        database: Database,
        config: &AppConfig,
        startup_cleanup_authority: Arc<StartupCleanupAuthority>,
    ) -> anyhow::Result<Self> {
        // Brand computer-use permission-error guidance with the host app's name so
        // failures say "grant NomiFun … then quit and reopen NomiFun" instead of a
        // generic "this app" — which a model otherwise misreads as the terminal /
        // editor and sends the user to grant the wrong process. Set once, here, so
        // every later `observe` / screenshot / input failure carries the right name.
        #[cfg(feature = "computer-use")]
        nomi_computer::set_host_app_label("NomiFun");

        let data_dir = config.data_dir.clone();
        let work_dir = config.work_dir.clone();
        let work_dir_is_cli_override = config.work_dir_is_cli_override;
        // The on-device model feature has been retired. Remove its managed
        // models, runtimes, partial downloads, ASR jobs, and persisted state so
        // upgrades do not leave multi-gigabyte orphaned data behind.
        let retired_model_dir = data_dir.join("local-ai");
        if let Err(error) = std::fs::remove_dir_all(&retired_model_dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %retired_model_dir.display(), %error, "Could not remove retired on-device model data");
        }
        // Security hard-cut: older builds persisted live loopback root tokens in
        // this beacon. Scoped child capabilities make discovery without an
        // authoritative session impossible, so remove both the final and
        // interrupted-write files before any new loopback issuer starts.
        for obsolete in ["mcp-endpoints.json", "mcp-endpoints.json.tmp"] {
            let path = data_dir.join(obsolete);
            if let Err(error) = std::fs::remove_file(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), %error, "Could not remove obsolete MCP secret beacon");
            }
        }
        // Terminal MCP launch files are ephemeral. Older versions embedded a
        // process-wide token in these files; current versions keep even scoped
        // child credentials in the inherited process environment. Reset the
        // directory on every boot so neither historical nor stale session
        // configuration survives a backend restart.
        let terminal_mcp_dir = data_dir.join("terminal-mcp");
        if let Err(error) = std::fs::remove_dir_all(&terminal_mcp_dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %terminal_mcp_dir.display(), %error, "Could not reset ephemeral terminal MCP config directory");
        }
        let auth_policy = config.auth_policy;
        let local_trust_secret = config.local_trust_secret.clone();
        let app_version = config.app_version.clone();
        let user_repo: Arc<dyn IUserRepository> =
            Arc::new(SqliteUserRepository::new(database.pool().clone()));

        // Per-companion Remote front-door tokens: the repo persists each
        // companion's token hash; the validator caches `token -> companion_id`
        // in memory, hydrated from the DB at boot. An empty map means the front
        // door stays closed until a token is minted.
        let companion_token_repo: Arc<dyn ICompanionTokenRepository> =
            Arc::new(SqliteCompanionTokenRepository::new(database.pool().clone()));
        let initial_tokens = companion_token_repo.list_all().await.unwrap_or_else(|e| {
            tracing::warn!("failed to load companion access tokens at boot (Remote front door stays closed until a token is minted): {e}");
            Vec::new()
        });
        let companion_token_validator = Arc::new(CompanionTokenValidator::new(initial_tokens));

        // Resolve JWT secret: env var → installation-owner DB field → random generation
        let env_secret = std::env::var("JWT_SECRET").ok();
        let installation_owner = user_repo
            .get_system_user()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get installation owner: {e}"))?
            .ok_or_else(|| {
                anyhow::anyhow!("Database invariant violated: installation owner is missing")
            })?;
        let authoritative_user_id: Arc<str> = Arc::from(installation_owner.user_id.as_str());

        let db_secret = installation_owner.jwt_secret.as_deref().filter(|s| !s.is_empty());

        let (secret, is_new) = resolve_jwt_secret(env_secret.as_deref(), db_secret);

        // Persist newly generated secret to database
        if is_new {
            user_repo
                .update_jwt_secret(installation_owner.user_id.as_str(), &secret)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to persist JWT secret: {e}"))?;
            tracing::info!("Generated and persisted new JWT secret");
        }

        let encryption_key = load_or_create_data_encryption_key(&data_dir, &secret)
            .map_err(|e| anyhow::anyhow!("Failed to load data encryption key: {e}"))?;

        let remote_agent_repo = Arc::new(SqliteRemoteAgentRepository::new(database.pool().clone()));
        let provider_repo = Arc::new(SqliteProviderRepository::new(database.pool().clone()));
        let provider_model_repo: Arc<dyn IProviderModelRepository> =
            Arc::new(SqliteProviderModelRepository::new(database.pool().clone()));
        // Start the stable managed-model loopback supply and provision its
        // provider projection before any model-profile reconciliation or agent
        // factory construction. A seed catalog makes a fresh install usable
        // without blocking boot on third-party discovery.
        let (managed_model_service, managed_model_server) =
            nomifun_system::start_and_provision_free_model_with_preferences(
                provider_repo.clone(),
                provider_model_repo.clone(),
                Some(Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                    database.pool().clone(),
                ))),
                encryption_key,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to provision NomiFun free model service: {e}"))?;
        // Refresh immediately, then about every six hours with jitter. Failed
        // attempts retain the current catalog and use capped exponential
        // backoff. Successful refreshes atomically seed profiles for any newly
        // discovered models without overwriting concurrent user edits.
        let managed_model_refresh_task = {
            let profile_repo = provider_model_repo.clone();
            nomifun_system::ManagedModelRefreshTask::start_with_success_hook(
                managed_model_service.clone(),
                move |status| {
                    let profile_repo = profile_repo.clone();
                    async move {
                        let Some(provider_id) = status.provider_id.as_deref() else {
                            tracing::warn!("Managed free-model refresh returned no provider id");
                            return;
                        };
                        let models = status
                            .models
                            .iter()
                            .map(|model| model.id.as_str())
                            .collect::<Vec<_>>();
                        match nomifun_system::seed_missing_inferred_profiles(
                            profile_repo.as_ref(),
                            provider_id,
                            nomifun_system::FREE_MODEL_PLATFORM,
                            &models,
                        )
                        .await
                        {
                            Ok(seeded) if seeded > 0 => tracing::info!(
                                seeded,
                                "Managed free-model refresh seeded inferred model profiles"
                            ),
                            Ok(_) => {}
                            Err(error) => tracing::warn!(
                                error = %error,
                                "Managed free-model profile reconciliation failed"
                            ),
                        }
                    }
                },
            )
        };
        // User-configured MCP servers — injected into ACP `session/new`
        // so the agent gets the operator's tools (ELECTRON-1JG fix).
        let mcp_server_repo: Arc<dyn IMcpServerRepository> =
            Arc::new(SqliteMcpServerRepository::new(database.pool().clone()));

        let agent_metadata_repo: Arc<dyn IAgentMetadataRepository> =
            Arc::new(SqliteAgentMetadataRepository::new(database.pool().clone()));
        let agent_registry = AgentRegistry::new(agent_metadata_repo);
        agent_registry
            .hydrate()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to hydrate agent registry: {e}"))?;

        let acp_session_repo: Arc<dyn IAcpSessionRepository> =
            Arc::new(SqliteAcpSessionRepository::new(database.pool().clone()));
        let acp_agent_service = AcpSessionSyncService::new(acp_session_repo.clone());

        let conversation_repo: Arc<dyn IConversationRepository> =
            Arc::new(SqliteConversationRepository::new(database.pool().clone()));
        let execution_conversation_boundary: Arc<dyn ExecutionConversationBoundary> = Arc::new(
            RepositoryExecutionConversationBoundary::new(Arc::new(
                nomifun_db::SqliteAgentExecutionRepository::new(database.pool().clone()),
            )),
        );

        // Skill paths need app resource dir (for builtin rules) + data dir
        // (for user skills + materialized views). AcpSkillManager uses these
        // for first-message skill index/body loading.
        let app_resource_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok())
            .and_then(|p| p.parent().map(|pp| pp.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let skill_paths = Arc::new(nomifun_extension::resolve_skill_paths(
            &app_resource_dir,
            &data_dir,
        ));

        // Absolute path to this process's binary. Reused as the `command` for
        // stdio MCP bridges spawned by agent sessions.
        let backend_binary_path = Arc::new(
            std::env::current_exe()
                .map_err(|error| anyhow::anyhow!("failed to resolve backend executable path: {error}"))?,
        );
        let backend_binary_path_utf8 =
            require_utf8_executable_path(backend_binary_path.as_path())?;

        // Event bus is shared by every service that broadcasts WS events.
        // Constructed here (rather than inline in the returned struct) so the
        // requirement service + sink built below share the same bus.
        let event_bus = Arc::new(BroadcastEventBus::new(256));

        // Requirement service + sink. Built before the agent factory because the
        // factory needs the sink to register the nomi native requirement tools.
        let requirement_repo: Arc<dyn nomifun_db::IRequirementRepository> = Arc::new(
            nomifun_db::SqliteRequirementRepository::new(database.pool().clone()),
        );
        let requirement_emitter = nomifun_requirement::RequirementEventEmitter::new(
            event_bus.clone(),
            authoritative_user_id.clone(),
        );
        // Completion notifier: on a requirement reaching a terminal state, notify
        // its tag's bound webhook. Injected into the SINGLETON so it fires on BOTH
        // completion paths — the Agent self-report sink AND the AutoWork runner's
        // `finalize_if_needed` (both clone from this instance, propagating the
        // notifier field). The repos share the same pool as `build_webhook_state`,
        // so they read the same `webhooks` / `tag_settings` tables.
        let webhook_repo_for_notifier: Arc<dyn nomifun_db::IWebhookRepository> = Arc::new(
            nomifun_db::SqliteWebhookRepository::new(database.pool().clone()),
        );
        let tag_setting_repo_for_notifier: Arc<dyn nomifun_db::ITagSettingRepository> = Arc::new(
            nomifun_db::SqliteTagSettingRepository::new(database.pool().clone()),
        );
        let completion_notifier = nomifun_webhook::CompletionNotifierImpl::new(
            tag_setting_repo_for_notifier,
            webhook_repo_for_notifier,
            Arc::new(nomifun_webhook::DefaultWebhookSender::new()),
        )
        .into_arc();
        let attachment_repo: Arc<dyn nomifun_db::IAttachmentRepository> = Arc::new(
            nomifun_db::SqliteAttachmentRepository::new(database.pool().clone()),
        );
        let attachment_store = Arc::new(nomifun_requirement::AttachmentStore::new(
            data_dir.clone(),
            attachment_repo,
        ));
        let requirement_service = Arc::new(
            nomifun_requirement::RequirementService::new(requirement_repo, requirement_emitter)
                .with_completion_notifier(completion_notifier)
                .with_attachment_store(attachment_store),
        );
        let requirement_sink =
            nomifun_requirement::RequirementServiceSink::into_arc(requirement_service.clone());

        // Requirement MCP server: gives ACP AutoWork sessions the
        // `requirement_complete` / `requirement_update_status` declaration tools
        // over a stdio bridge (claude/codex/gemini are stdio-only for MCP).
        // Failure is non-fatal — ACP sessions then keep the tool-free contract
        // and `requirement_mcp_enabled` stays false. Wired to the SAME singleton
        // the sink/AutoWork runner use (held as a Weak).
        let (requirement_mcp_server, requirement_mcp_config) =
            match nomifun_requirement::RequirementMcpServer::start().await {
                Ok(srv) => {
                    srv.set_service(Arc::downgrade(&requirement_service)).await;
                    let config = srv.issuer_config(backend_binary_path_utf8.clone());
                    tracing::info!(port = config.port(), "Requirement MCP server started");
                    (Some(srv), Some(config))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Requirement MCP server failed to start; ACP AutoWork verdict tools disabled");
                    (None, None)
                }
            };

        // Platform Gateway MCP server: gives owner Agent sessions (Channel
        // Agent and companion conversations included) the `nomi_*` tools over
        // a stdio bridge. Started BEFORE the agent factory so the factory can
        // carry the connection config; the deps bundle is late-wired from
        // `create_router` (the conversation/cron services are built there).
        // Failure is non-fatal — flagged sessions then lack the desktop tools.
        let (gateway_mcp_server, gateway_mcp_config) =
            match nomifun_gateway::GatewayMcpServer::start().await {
                Ok(srv) => {
                    let srv = Arc::new(srv);
                    let config = srv.issuer_config(
                        backend_binary_path_utf8.clone(),
                        authoritative_user_id.to_string(),
                    );
                    tracing::info!(port = config.port(), "Gateway MCP server started");
                    (Some(srv), Some(config))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Gateway MCP server failed to start; platform tools disabled");
                    (None, None)
                }
            };
        // Register the first long-lived ingress immediately. Any later
        // composition failure must quiesce Gateway before the database closes;
        // waiting until the final AppServices struct exists leaves a large
        // unsafe window in which Drop is the only shutdown signal.
        let browser_platform_shutdown =
            BrowserPlatformShutdown::gateway_only_early(gateway_mcp_server.clone());
        startup_cleanup_authority
            .install_browser_platform(browser_platform_shutdown.clone())
            .await;

        // Reliable-launch (`open`) MCP config — Windows only. macOS/Linux already
        // launch URLs/apps reliably (`open`/`xdg-open`), so the agent needs no
        // nudging there; on Windows it stops the agent from using the fragile
        // `cmd /c start` (which mis-parses URLs as window titles and pops
        // "Windows cannot find '\\'" dialogs). Stateless — no server to start,
        // just the binary path so the assembler can spawn `mcp-open-stdio`.
        let open_mcp_config =
            cfg!(target_os = "windows").then(|| nomifun_api_types::OpenMcpConfig {
                binary_path: backend_binary_path_utf8.clone(),
            });

        // Computer-use discrete-tool MCP config — every desktop OS (macOS /
        // Windows / Linux), gated ONLY on the `computer-use` feature (else
        // `mcp-computer-stdio` is a stub, so we'd inject a bridge the binary
        // can't serve). Lets codex/ACP sessions drive the desktop (snapshot /
        // click / type / launch) via `nomicore mcp-computer-stdio`, mirroring the
        // in-process `ComputerTool` the nomi engine already gets on all platforms
        // (`nomi-a11y` implements macOS AX / Windows UIA / Linux AT-SPI backends).
        // Platform reality the bridge surfaces honestly: macOS needs the user to
        // grant TCC (Accessibility + Screen Recording) or ops error out; Linux
        // lacks OCR + cross-app window focus and degrades synthetic input on
        // Wayland. None of that warrants gating the bridge off — the tools simply
        // report `Unsupported` where the OS can't serve them.
        let computer_mcp_config =
            cfg!(feature = "computer-use").then(|| nomifun_api_types::ComputerMcpConfig {
                binary_path: backend_binary_path_utf8.clone(),
            });

        // Singleton knowledge service: knowledge base registry + workspace
        // mounting. Shared by the `/api/knowledge/*` routes and the
        // conversation service (mount-at-task-start).
        let knowledge_repo: Arc<dyn nomifun_db::IKnowledgeRepository> = Arc::new(
            nomifun_db::SqliteKnowledgeRepository::new(database.pool().clone()),
        );
        let knowledge_service = Arc::new(nomifun_knowledge::KnowledgeService::new(
            knowledge_repo,
            &data_dir,
            nomifun_knowledge::KnowledgeEventEmitter::new(
                event_bus.clone(),
                authoritative_user_id.clone(),
            ),
        ));
        // Late-wire the LLM seam for knowledge autogen / snapshot compression
        // (`LiveKnowledgeCompleter` resolves the first enabled provider/model
        // per call, so it tolerates providers configured after boot). NOTE:
        // `provider_repo` is moved into `build_agent_factory` below — clone.
        knowledge_service.set_completer(Arc::new(nomifun_ai_agent::LiveKnowledgeCompleter {
            provider_repo: provider_repo.clone() as Arc<dyn nomifun_db::IProviderRepository>,
            provider_model_repo: provider_model_repo.clone(),
            encryption_key,
            workspace: data_dir.clone(),
        }));
        // Recover profiles left by an interrupted earlier browser runtime
        // before constructing any Host authority. If ownership/termination
        // cannot be proven, Browser functionality remains degraded for this
        // process: no Hub/provider/MCP/fetcher wiring is published.
        #[cfg(feature = "browser-use")]
        let browser_orphan_recovery = {
            // Recover marker-owned browser processes before any managed Host
            // can launch. The engine validates app/browser PID + executable +
            // full platform creation identity and confirms the process tree is
            // gone before touching disk. Primary and legacy stable profiles
            // retain all data; only their completed ownership marker is
            // cleared. Explicit ephemeral roots may be deleted after proof.
            let browser_data = data_dir.join("browser-data");
            let platform_profiles = browser_data.join("platform-profiles");
            let recovery = tokio::task::spawn_blocking(move || {
                use nomi_browser_engine::profile::{
                    ProfileRecoveryMode, ProfileRecoveryReport, recover_owned_profiles,
                };

                let mut report = ProfileRecoveryReport::default();
                for profiles_root in [
                    browser_data.join("profiles"),
                    platform_profiles.join("anonymous"),
                    platform_profiles.join("replica"),
                    platform_profiles.join("isolated"),
                ] {
                    report.merge(recover_owned_profiles(
                        &profiles_root,
                        ProfileRecoveryMode::DeleteEphemeralProfile,
                    ));
                }
                for stable_root in [
                    browser_data.join("profile"),
                    platform_profiles.join("primary"),
                ] {
                    report.merge(recover_owned_profiles(
                        &stable_root,
                        ProfileRecoveryMode::PreserveStableProfile,
                    ));
                }
                report
            })
            .await;
            let outcome = match recovery {
                Ok(report) => BrowserOrphanRecoveryOutcome::from_report(&report),
                Err(error) => BrowserOrphanRecoveryOutcome::from_join_error(&error),
            };
            match &outcome {
                BrowserOrphanRecoveryOutcome::Safe { summary } => {
                    tracing::info!(
                        %summary,
                        "browser orphan recovery completed safely"
                    );
                }
                BrowserOrphanRecoveryOutcome::Degraded { reason } => {
                    tracing::error!(
                        %reason,
                        "browser startup degraded closed after unsafe orphan recovery"
                    );
                }
            }
            outcome
        };

        // Preference migration is independent from Chromium orphan recovery.
        // Even when Host composition must remain disabled for this process, a
        // successfully read unversioned/legacy display policy is still
        // migrated to the explicit v2 headless lineage. A read failure remains
        // fail-safe and never writes a replacement value.
        #[cfg(feature = "browser-use")]
        let browser_startup_preferences = {
            let preference_repo =
                SqliteClientPreferenceRepository::new(database.pool().clone());
            load_browser_startup_preferences(&preference_repo).await
        };

        // Browser-use MCP is a scoped proxy into the process-wide Hub. Start
        // its issuer only after orphan recovery proved safe. Failure or a
        // degraded recovery disables ACP browser tools without falling back to
        // child-owned Chromium.
        #[cfg(feature = "browser-use")]
        let (browser_mcp_server, browser_mcp_config) = if browser_orphan_recovery.is_safe() {
            match crate::browser_mcp_server::BrowserMcpServer::start().await {
                Ok(server) => {
                    let server = Arc::new(server);
                    let config = server.issuer_config(backend_binary_path_utf8.clone());
                    tracing::info!("Browser MCP scoped proxy started");
                    (Some(server), Some(config))
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Browser MCP scoped proxy failed to start; ACP browser tools disabled"
                    );
                    (None, None)
                }
            }
        } else {
            (None, None)
        };
        #[cfg(feature = "browser-use")]
        browser_platform_shutdown
            .set_browser_mcp(browser_mcp_server.clone())
            .await;
        #[cfg(not(feature = "browser-use"))]
        let browser_mcp_config = None;
        // Boot-resume: re-fetch snapshot-mode URL sources whose create-time
        // fetch never completed (the app exited mid-run — the source is
        // persisted unstamped before fetching). Spawned after the completer
        // wiring so the chained autogen works; never blocks startup.

        // Knowledge MCP server: gives ACP sessions with bound knowledge bases
        // search/read and policy-gated write tools over a stdio bridge. It owns
        // a domain-separated root issuer kept in this process; each managed
        // child receives only short-lived signed user/session/workspace/base/tool
        // claims. Wired to the SAME singleton KnowledgeService the routes use
        // (held as a Weak), mirroring the requirement server.
        // Failure is non-fatal — sessions then lack `knowledge_search` (graceful
        // degradation identical to having no mounted bases).
        let (knowledge_mcp_server, knowledge_mcp_config) =
            match nomifun_knowledge::KnowledgeMcpServer::start().await {
                Ok(mut srv) => {
                    srv.set_service(&knowledge_service).await;
                    let config = srv.issuer_config(backend_binary_path_utf8.clone());
                    if let Err(error) = srv
                        .start_external_broker(
                            config.clone(),
                            authoritative_user_id.to_string(),
                        )
                        .await
                    {
                        tracing::warn!(%error, "secure external knowledge MCP broker failed to start");
                    }
                    tracing::info!(port = config.port(), "Knowledge MCP server started");
                    (Some(srv), Some(config))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Knowledge MCP server failed to start; scoped knowledge_search tool disabled");
                    (None, None)
                }
            };

        // Singleton terminal service (owns the live PTY map). Shared between the
        // terminal routes and the AutoWork runner's terminal driver.
        let terminal_repo: Arc<dyn nomifun_db::ITerminalRepository> =
            Arc::new(SqliteTerminalRepository::new(database.pool().clone()));
        let terminal_service = Arc::new(TerminalService::new(
            terminal_repo,
            TerminalEventEmitter::new(event_bus.clone()),
            work_dir.clone(),
        ));
        // Wire the scoped knowledge-search MCP into terminal launches: a
        // terminal whose cwd has mounted bases gets the real knowledge_search
        // tool injected into the native CLI (claude/codex), same bridge as ACP.
        // Config dir is platform-private (under data_dir), never the user cwd.
        if let Some(cfg) = knowledge_mcp_config.clone() {
            terminal_service.with_knowledge_mcp_config(cfg, data_dir.join("terminal-mcp"));
        }
        // Wire the scoped requirement MCP into terminal launches: agent CLIs
        // (claude/codex) get the requirement_complete/requirement_update_status
        // tools injected as a stdio bridge, scoped to the terminal's own id +
        // owner_kind=terminal. Unknown CLIs/shell are unaffected (apply_enhancement
        // skips rendering for them). Mirrors the knowledge MCP wiring above.
        if let Some(cfg) = requirement_mcp_config.clone() {
            terminal_service.with_requirement_mcp_config(cfg);
        }
        // Wire the auto-title completer: a terminal session's first turn (agent
        // CLIs) is summarized into a short work-content title via the default
        // provider/model (same resolution as `LiveKnowledgeCompleter` above).
        // Shell sessions / no provider fall back to the first input line, so this
        // is best-effort and never blocks a launch.
        terminal_service.with_title_completer(Arc::new(nomifun_ai_agent::LiveTerminalTitleCompleter {
            provider_repo: provider_repo.clone(),
            provider_model_repo: provider_model_repo.clone(),
            encryption_key,
            workspace: data_dir.clone(),
        }));
        // Start the terminal lifecycle server (house pattern, 4th instance):
        // native CLI hooks (claude --settings / codex -c hooks) POST turn/tool/
        // notification events to it via the `nomicore terminal-hook` shim, and it
        // broadcasts them per terminal_id. Failure is non-fatal — terminals then
        // simply lack lifecycle events (graceful degradation). The backend binary
        // path is needed so injected hook commands invoke `<bin> terminal-hook`.
        match TerminalLifecycleServer::start().await {
            Ok(srv) => {
                tracing::info!(port = srv.http_port(), "Terminal lifecycle server started");
                terminal_service.with_terminal_lifecycle(
                    std::sync::Arc::new(srv),
                    backend_binary_path_utf8.clone(),
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "Terminal lifecycle server failed to start; terminal hooks disabled");
            }
        };

        // Boot reconciliation: flip ghost 'running' rows (PTYs that died with the
        // previous app run — `live` is empty here) to 'exited'. This makes the
        // state honest so the frontend shows the relaunch entry + replays
        // persisted scrollback instead of a black screen, and a cron-bound
        // terminal's fire-time `live` check takes the relaunch path rather than
        // writing to a dead handle. Runs before cron init (in build_module_states).
        if let Err(e) = terminal_service.reconcile_on_boot().await {
            tracing::warn!(error = %e, "terminal boot reconciliation failed");
        }
        // Debounced scrollback persistence loop so terminal output history
        // survives a restart (dirty live sessions only; never per chunk).
        // Start it only after all remaining fallible composition steps succeed.
        // The task owns a clone of TerminalService, which in turn owns the
        // TerminalLifecycleServer Arc; starting it here would keep the
        // lifecycle listener alive if a later startup step returned an error.

        // Companion service (nomi companion): built BEFORE the agent factory so the
        // factory gets the companion memory sink (recall/save memory tools for
        // companion_session conversations). The companion router state reuses this same
        // instance via `services.companion_service`.
        let companion_completer: Arc<dyn nomifun_companion::learner::CompanionCompleter> =
            Arc::new(nomifun_companion::learner::LiveCompanionCompleter {
                provider_repo: provider_repo.clone() as Arc<dyn nomifun_db::IProviderRepository>,
                provider_model_repo: provider_model_repo.clone(),
                encryption_key,
                workspace: data_dir.clone(),
            });
        let provider_lifecycle = Arc::new(nomifun_common::ProviderLifecycleBarrier::new());
        let companion_service = nomifun_companion::CompanionService::start_with_provider_lifecycle(
            &data_dir,
            event_bus.clone(),
            authoritative_user_id.as_ref(),
            companion_completer,
            skill_paths.clone(),
            Some(provider_repo.clone() as Arc<dyn nomifun_db::IProviderRepository>),
            Some(provider_lifecycle.clone()),
        )
        .await
        .map_err(|e| anyhow::anyhow!("companion service start failed: {e}"))?;

        // 客服独立域 (customer-service domain): agents/notes/bindings CRUD
        // service + the stateless concurrent dialogue engine. The engine's
        // LLM turns go through the generic one-shot entry whose tool table is
        // fixed at construction to three read-only tools — no workspace mount,
        // no runtime registry, no Conversation.
        let customer_service_repo: Arc<dyn nomifun_db::ICustomerServiceRepository> =
            Arc::new(nomifun_db::SqliteCustomerServiceRepository::new(
                database.pool().clone(),
            ));
        let customer_service_service = Arc::new(
            nomifun_customer_service::CustomerServiceService::new(customer_service_repo.clone()),
        );
        let cs_dialogue_engine = Arc::new(nomifun_customer_service::CsDialogueEngine::new(
            customer_service_repo,
            knowledge_service.clone(),
            Arc::new(nomifun_customer_service::LiveTurnRunner {
                deps: nomifun_ai_agent::OneShotDeps {
                    provider_repo: provider_repo.clone()
                        as Arc<dyn nomifun_db::IProviderRepository>,
                    provider_model_repo: provider_model_repo.clone(),
                    encryption_key,
                    workspace: data_dir.clone(),
                },
            }),
        ));

        // 创意工坊 (Creative Workshop) + 生成引擎 (creation): the workshop service
        // owns canvas/asset index rows + on-disk docs/binaries; the creation
        // service owns the media generation task queue. Both are plain repo-backed
        // services (no agent-factory dependency), constructed here alongside the
        // other singletons and reused by the router states.
        let workshop_service = nomifun_workshop::WorkshopService::start_with_provider_lifecycle(
            &data_dir,
            Arc::new(nomifun_db::SqliteWorkshopRepository::new(database.pool().clone())),
            provider_lifecycle.clone(),
        );
        if let Err(error) = workshop_service.audit_managed_data_on_boot().await {
            anyhow::bail!(
                "managed Workshop data failed its startup integrity audit; \
                 the existing dataset has been preserved; request an explicit factory reset \
                 to replace it: {error}"
            );
        }
        // The generation engine delegates model execution to the unified
        // invoke layer (provider/model/protocol resolution + adapters live
        // there), runs over a proxy-aware HTTP client, and reads/writes canvas
        // assets through the workshop bridge (AssetSource/AssetSink — no crate
        // cycle). `reconcile_on_boot` (running-with-remote resume / else
        // fail-interrupted) is driven from `build_creation_state` at router
        // assembly.
        let creation_http = nomifun_net::http_client();
        // Unified multimodal invoke layer (P1): one process-wide singleton over
        // the catalog repos + the same proxy-aware HTTP client. The creation
        // engine and `/api/tts` consume it; later tasks (health probes) reuse
        // this exact instance.
        let model_invoke_service = Arc::new(nomifun_model_invoke::ModelInvokeService::new(
            Arc::new(nomifun_db::SqliteProviderRepository::new(database.pool().clone())),
            Arc::new(nomifun_db::SqliteProviderModelRepository::new(database.pool().clone())),
            Arc::new(nomifun_db::SqliteProviderConnectionRepository::new(database.pool().clone())),
            encryption_key,
            creation_http.clone(),
            nomifun_model_invoke::AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
        ));
        let creation_asset_bridge = Arc::new(crate::workshop_bridge::WorkshopAssetBridge::new(
            data_dir.clone(),
            Arc::new(nomifun_db::SqliteWorkshopRepository::new(database.pool().clone())),
        ));
        let creation_service = nomifun_creation::CreationService::builder(Arc::new(
            nomifun_db::SqliteCreationTaskRepository::new(database.pool().clone()),
        ))
        .with_http(creation_http.clone())
        .with_invoke(model_invoke_service.clone())
        .with_asset_source(creation_asset_bridge.clone())
        .with_asset_sink(creation_asset_bridge)
        .build();
        // Complete task/asset reconciliation before AppServices is published.
        // Running this synchronously closes the race where a newly-created task
        // could persist an asset between the task snapshot and Workshop scan
        // and be mistaken for an orphan by detached boot cleanup.
        if let Err(error) = creation_service.audit_managed_data_on_boot().await {
            anyhow::bail!(
                "managed creation data failed its startup integrity audit; \
                 the existing dataset has been preserved; request an explicit factory reset \
                 to replace it: {error}"
            );
        }
        creation_service
            .reconcile_on_boot()
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "creation startup reconciliation failed without changing dataset lineage: {error}"
                )
            })?;

        // Headless seed: bind a Remote access token to the default companion so an
        // operator can configure the front door via env on a headless server.
        // (Desktop mints per-companion tokens via /api/webui/companions/{id}/access-token.)
        if let Ok(seed) = std::env::var("NOMIFUN_COMPANION_TOKEN") {
            let seed = seed.trim();
            if !seed.is_empty() && companion_token_validator.resolve(seed).is_none() {
                match companion_service.default_companion_id().await {
                    Some(default_id) => {
                        let default_id = nomifun_common::CompanionId::parse(default_id)
                            .map_err(|error| anyhow::anyhow!("default companion has invalid id: {error}"))?;
                        let hash = nomifun_auth::token_sha256_hex(seed);
                        if let Err(e) = companion_token_repo
                            .upsert_for_companion(&default_id, &hash)
                            .await
                        {
                            tracing::warn!("failed to persist NOMIFUN_COMPANION_TOKEN seed: {e}");
                        }
                        companion_token_validator.insert_token(default_id.clone(), hash);
                        tracing::info!(
                            "Remote access token seeded from NOMIFUN_COMPANION_TOKEN, bound to default companion {default_id}"
                        );
                    }
                    None => tracing::warn!(
                        "NOMIFUN_COMPANION_TOKEN set but no companion exists to bind it to; create a companion first"
                    ),
                }
            }
        }

        // Expose the provider repo on AppServices (mint-time model guard reads it)
        // before it is moved into the agent factory below.
        let provider_repo_for_services: Arc<dyn IProviderRepository> =
            provider_repo.clone() as Arc<dyn nomifun_db::IProviderRepository>;

        // Seed authoritative capability profiles for any provider models that
        // lack one (multimodal model hub). Best-effort: never blocks boot on error.
        reconcile_model_profiles(&provider_repo_for_services, &provider_model_repo).await;

        // One-time legacy speech-preference migration: pre-provider-catalog
        // configs that still embed a raw openai/deepgram credential (and have
        // no provider_id) are disabled and de-credentialed. Best-effort:
        // never blocks boot on error.
        {
            let preference_repo =
                SqliteClientPreferenceRepository::new(database.pool().clone());
            migrate_legacy_speech_preference(&preference_repo).await;
        }

        #[cfg(feature = "browser-use")]
        let browser_lane_provider_slot =
            nomifun_ai_agent::BrowserLaneClientProviderSlot::new();

        let factory = build_agent_factory(AgentFactoryDeps {
            authoritative_user_id: authoritative_user_id.clone(),
            skill_manager: AcpSkillManager::new(skill_paths.clone()),
            remote_agent_repo,
            provider_repo,
            provider_model_repo: provider_model_repo.clone(),
            encryption_key,
            agent_registry: agent_registry.clone(),
            acp_agent_service: acp_agent_service.clone(),
            data_dir: data_dir.clone(),
            work_dir: work_dir.clone(),
            backend_binary_path: backend_binary_path.clone(),
            requirement_mcp_config: requirement_mcp_config.clone(),
            // Scoped knowledge-search MCP. Populated only when the server started
            // above; the assembler further gates injection on bound bases, so a
            // session without mounts never sees the tool. Independent of the
            // gateway config — this token never grants gateway reach.
            knowledge_mcp_config: knowledge_mcp_config.clone(),
            gateway_mcp_config: gateway_mcp_config.clone(),
            open_mcp_config: open_mcp_config.clone(),
            computer_mcp_config: computer_mcp_config.clone(),
            browser_mcp_config: browser_mcp_config.clone(),
            #[cfg(feature = "browser-use")]
            browser_lane_provider: Some(browser_lane_provider_slot.clone()),
            client_prefs: Some(Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                database.pool().clone(),
            ))
                as Arc<dyn nomifun_db::IClientPreferenceRepository>),
            // System settings repo: lets the nomi factory read the app UI language
            // live per build so every nomi session thinks and replies in the app's
            // language instead of the old hardcoded Chinese (mirrors client_prefs).
            settings_repo: Some(Arc::new(nomifun_db::SqliteSettingsRepository::new(
                database.pool().clone(),
            )) as Arc<dyn nomifun_db::ISettingsRepository>),
            mcp_server_repo: Some(mcp_server_repo),
            requirement_sink: Some(requirement_sink),
            // Native cron tools: agent schedules/lists/deletes its own recurring
            // prompts. The closure resolves the process CronService lazily (it is
            // registered at startup in router/state.rs, after this factory is
            // built), so by the time a conversation runs the agent the service is
            // present. (Phase 4 platform synergy)
            cron_sink_factory: Some(Arc::new(|user_id: &str, conversation_id: &str| {
                nomifun_cron::sink::cron_sink_for(
                    user_id.to_string(),
                    conversation_id.to_string(),
                )
            })),
            companion_sink: Some(companion_service.memory_sink()),
            // Companion self-evolved skill auto-use (`companion_skill` tool + per-turn
            // when_to_use injection). Only registered for companion sessions (factory gates).
            companion_skill_sink: Some(companion_service.skill_sink()),
            // Live knowledge_search sink: registers the retrieval tool over the
            // shared KnowledgeService. The field's declared type
            // `Option<Arc<dyn KnowledgeRetrievalSink>>` drives the unsized
            // coercion, so no explicit `dyn` annotation is needed here.
            knowledge_retrieval: Some(Arc::new(nomifun_ai_agent::LiveKnowledgeRetrievalSink {
                service: knowledge_service.clone(),
            })),
            // Live knowledge_write (回血) sink: registers the native write-back
            // tool over the same KnowledgeService. Gated downstream on bound
            // bases + write-back enabled, so a read-only session never sees it.
            knowledge_writeback: Some(Arc::new(nomifun_ai_agent::LiveKnowledgeWritebackSink {
                service: knowledge_service.clone(),
            })),
            companion_prompt: Some(
                companion_service.clone() as Arc<dyn nomifun_ai_agent::CompanionPromptProvider>
            ),
            // In-session companion summon (spec §设计 B): skills + selected
            // memories of one companion loaded read-only into work sessions
            // whose `extra.summon` is present (factory gates authority).
            companion_summon: Some(
                companion_service.clone() as Arc<dyn nomifun_ai_agent::CompanionSummonProvider>
            ),
        });

        // Agent factory is now wired. Future extension/custom agents
        // that get written to `agent_metadata` will show up after the
        // relevant service calls `AgentRegistry::hydrate`.
        let runtime_registry_concrete = Arc::new(
            InMemoryAgentRuntimeRegistry::new(factory)
                .with_nomi_session_directory(data_dir.join("nomi-sessions")),
        );
        let agent_runtime_registry: Arc<dyn AgentRuntimeRegistry> = runtime_registry_concrete.clone();
        let runtime_registry_delete_hook: Arc<dyn OnConversationDelete> = runtime_registry_concrete;
        let conversation_runtime_state = Arc::new(ConversationRuntimeStateService::default());

        let services = Self {
            database,
            _boot_reconciliation_authority: None,
            provider_lifecycle,
            authoritative_user_id,
            jwt_service: Arc::new(JwtService::new(secret.clone())),
            user_repo,
            companion_token_repo,
            companion_token_validator,
            provider_repo: provider_repo_for_services,
            managed_model_service,
            _managed_model_server: managed_model_server,
            _managed_model_refresh_task: managed_model_refresh_task,
            provider_model_repo: provider_model_repo.clone(),
            cookie_config: Arc::new(CookieConfig::from_env()),
            qr_token_store: Arc::new(QrTokenStore::new()),
            ws_manager: Arc::new(WebSocketManager::new()),
            event_bus,
            agent_runtime_registry,
            conversation_runtime_state,
            runtime_registry_delete_hook: Some(runtime_registry_delete_hook),
            agent_registry,
            conversation_repo,
            execution_conversation_boundary,
            requirement_service,
            terminal_service,
            acp_session_sync: acp_agent_service,
            jwt_secret_raw: secret,
            encryption_key,
            data_dir,
            work_dir,
            work_dir_is_cli_override,
            auth_policy,
            local_trust_secret,
            app_version,
            skill_paths,
            requirement_mcp_config,
            _requirement_mcp_server: requirement_mcp_server,
            gateway_mcp_config,
            _gateway_mcp_server: gateway_mcp_server,
            _knowledge_mcp_server: knowledge_mcp_server,
            companion_service,
            customer_service_service,
            cs_dialogue_engine,
            workshop_service,
            creation_service,
            model_invoke_service,
            knowledge_service,
            #[cfg(feature = "browser-use")]
            browser_session_hub: None,
            browser_platform_shutdown,
            #[cfg(feature = "browser-use")]
            _browser_lane_provider_slot: browser_lane_provider_slot,
            #[cfg(feature = "browser-use")]
            _browser_mcp_server: browser_mcp_server,
            #[cfg(feature = "browser-use")]
            _browser_platform_tasks: None,
        };

        #[cfg(feature = "browser-use")]
        {
            if !browser_orphan_recovery.permits_host_composition() {
                let BrowserOrphanRecoveryOutcome::Degraded { reason } =
                    &browser_orphan_recovery
                else {
                    unreachable!("unsafe Browser recovery outcome must be degraded");
                };
                tracing::error!(
                    %reason,
                    "Browser Hub composition skipped; all Browser entry points remain fail-closed"
                );
                services.terminal_service.spawn_scrollback_flusher();
                tokio::spawn(
                    Arc::clone(&services.knowledge_service).resume_pending_source_fetches(),
                );
                return Ok(services);
            }

            let BrowserStartupPreferences {
                display_mode,
                source,
                full_power,
                persistent_login,
            } = browser_startup_preferences;
            let storage_state = nomi_browser_engine::load_storage_state(
                &nomi_browser_engine::shared_storage_state_path(&services.data_dir),
                &services.encryption_key,
            )
            .and_then(|state| state.to_json().ok());
            let startup_identity_snapshot = storage_state.clone();
            let engine_config = nomi_browser_engine::EngineConfig {
                data_dir: services.data_dir.join("browser-data"),
                bundled_dir: crate::browser_resource::bundled_chrome_dir(),
                // Host visibility is assigned by the Hub's identity-aware
                // HostLaunchRequest. Keep the template headless so a future
                // non-Primary launch cannot inherit external-window state.
                headful: false,
                chrome_source: nomi_browser_engine::ChromeSource::from_source_str(&source),
                workspace_dir: Some(services.work_dir.clone()),
                evaluate_full_power: full_power,
                evaluate_persistent_login: persistent_login,
                storage_state,
                ..Default::default()
            };
            let secret_source = nomi_browser::BrowserSecretSource {
                vault_path: nomifun_secret::shared_vault_path(&services.data_dir),
                key: services.encryption_key,
            };
            let factory = nomi_browser::ManagedEngineHostFactory::new(engine_config)
                .with_identity_vault(
                    nomi_browser_engine::shared_storage_state_path(&services.data_dir),
                    services.encryption_key,
                )
                // F6 (裁决⑤): the same vault also feeds the HOST egress
                // allowlist, so managed lanes enforce the allow_etld1 the
                // standalone path enforces (and secret injection stays gated
                // on that enforced list).
                .with_secret_source(secret_source.clone())
                .with_lane_policy(Arc::new(move |tool| {
                    tool.secret_source(secret_source.clone())
                }));
            let mut hub_config = nomifun_browser_platform::HubConfig {
                // Hub applies this preference only to Primary identity when
                // constructing HostLaunchRequest; Anonymous/Replica/Isolated
                // Hosts remain headless even in external display mode.
                headful: primary_host_is_headful(display_mode),
                ..Default::default()
            };
            // Restore policy before Hub construction so admission and operation
            // limits are correct before any runtime can open its first Lane.
            let browser_preferences =
                nomifun_db::SqliteClientPreferenceRepository::new(services.database.pool().clone());
            hub_config.resource_policy =
                crate::router::browser_management::restore_persisted_resource_policy(
                    &browser_preferences,
                    hub_config.resource_policy,
                )
                .await;
            let hub = Arc::new(nomifun_browser_platform::BrowserSessionHub::new(
                Arc::new(factory),
                hub_config,
            ));
            if let Some(payload) = startup_identity_snapshot {
                if let Err(error) = hub.publish_identity_snapshot(
                    nomifun_browser_platform::IdentitySnapshotPayload::from_json(payload),
                    persisted_identity_seed_coverage(),
                ) {
                    tracing::warn!(
                        code = ?error.code,
                        "persisted canonical browser identity could not be seeded into the Hub"
                    );
                }
            }
            let browser_fetcher = Arc::new(nomifun_ai_agent::BrowserFetcher::new(
                Arc::clone(&hub),
                services.authoritative_user_id.to_string(),
            ));
            services
                .knowledge_service
                .set_render_fetcher(browser_fetcher);
            let services = services.with_browser_session_hub(hub).await?;
            services.terminal_service.spawn_scrollback_flusher();
            tokio::spawn(
                Arc::clone(&services.knowledge_service).resume_pending_source_fetches(),
            );
            return Ok(services);
        }

        #[cfg(not(feature = "browser-use"))]
        {
            services.terminal_service.spawn_scrollback_flusher();
            tokio::spawn(
                Arc::clone(&services.knowledge_service).resume_pending_source_fetches(),
            );
            Ok(services)
        }
    }
}

async fn close_database_after_browser_platform_cleanup<F, Fut>(
    browser_cleanup: anyhow::Result<()>,
    close_database: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    browser_cleanup?;
    close_database().await;
    Ok(())
}

/// Ensure every provider catalog model has an authoritative capability
/// profile on its [`nomifun_db::ProviderModelRow`]. Since migration 016 the
/// rows ARE the catalog, so this is a pure backfill pass: unprofiled
/// membership rows (`tasks == "[]"`, `source == "inferred"`) get tasks/traits
/// from the name/platform heuristic; existing profiles (incl. user overrides)
/// are left untouched. Best-effort — logs and returns on any error so boot
/// never fails on profile reconciliation.
async fn reconcile_model_profiles(
    provider_repo: &Arc<dyn IProviderRepository>,
    provider_model_repo: &Arc<dyn IProviderModelRepository>,
) {
    let providers = match provider_repo.list().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("model-profile reconcile: failed to list providers: {e}");
            return;
        }
    };
    let mut seeded = 0usize;
    for provider in &providers {
        match nomifun_system::seed_inferred_provider_models(
            provider_model_repo.as_ref(),
            &provider.provider_id,
            &provider.platform,
        )
        .await
        {
            Ok(count) => seeded += count,
            Err(error) => tracing::warn!(
                provider_id = %provider.provider_id,
                error = %error,
                "model-profile reconcile failed"
            ),
        }
    }
    if seeded > 0 {
        tracing::info!("model-profile reconcile: seeded {seeded} inferred profile(s)");
    }
}

/// Preference keys holding the speech-to-text tool config, in the order the
/// shell reads them (`nomifun-shell` STT route: namespaced key first, then
/// the pre-namespacing legacy fallback key).
const SPEECH_PREFERENCE_KEYS: [&str; 2] = ["tools.speechToText", "speechToText"];

/// One-time boot migration for pre-provider-catalog speech configs.
///
/// Legacy speech preferences embedded raw `openai`/`deepgram` credential
/// blocks instead of referencing a catalog provider (`provider_id`). The
/// invoke layer only executes catalog-backed models — the shell STT route
/// already rejects such configs with a "re-select your speech provider"
/// error — so a stored config that still carries an embedded credential but
/// no `provider_id` is rewritten here: `enabled` is forced to `false` and the
/// embedded blocks are removed. All other fields (`model`, `language`,
/// `auto_send`, ...) are preserved so the user only has to re-select a
/// provider in Settings.
///
/// Idempotent: after the rewrite no embedded credential remains, so the next
/// boot leaves the value untouched. Credential-less `openai`/`deepgram`
/// shells (the frontend historically persisted empty-key blocks for
/// unconfigured providers) are NOT legacy and keep their existing
/// "not configured" behavior; configs that already carry a `provider_id` are
/// never touched. Best-effort — logs and returns on any error so boot never
/// fails on this migration.
async fn migrate_legacy_speech_preference<R>(preference_repo: &R)
where
    R: IClientPreferenceRepository + ?Sized,
{
    let rows = match preference_repo.get_by_keys(&SPEECH_PREFERENCE_KEYS).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                %error,
                "legacy speech preference migration: could not read preferences; skipping"
            );
            return;
        }
    };
    for row in rows {
        let Some(rewritten) = rewrite_legacy_speech_preference(&row.value) else {
            continue;
        };
        match preference_repo
            .upsert_batch(&[(row.key.as_str(), rewritten.as_str())])
            .await
        {
            Ok(()) => tracing::info!(
                key = row.key.as_str(),
                "legacy speech config disabled and its embedded credential removed; \
                 re-select the speech provider in Settings to re-enable speech recognition"
            ),
            Err(error) => tracing::warn!(
                key = row.key.as_str(),
                %error,
                "legacy speech preference migration: rewrite failed; value left untouched"
            ),
        }
    }
}

/// Pure rewrite rule for [`migrate_legacy_speech_preference`].
///
/// Returns the replacement JSON when `value` is a legacy embedded-credential
/// speech config — an object with no `provider_id` (absent or `null`) whose
/// `openai` or `deepgram` block carries a non-empty `api_key` — and `None`
/// when the stored value must be left untouched (already-migrated, catalog
/// mode, credential-less shells, or anything unparseable).
fn rewrite_legacy_speech_preference(value: &str) -> Option<String> {
    let mut parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    let object = parsed.as_object_mut()?;
    if object.get("provider_id").is_some_and(|id| !id.is_null()) {
        return None;
    }
    let has_embedded_credential = ["openai", "deepgram"].iter().any(|block| {
        object
            .get(*block)
            .and_then(|block| block.get("api_key"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|api_key| !api_key.trim().is_empty())
    });
    if !has_embedded_credential {
        return None;
    }
    object.remove("openai");
    object.remove("deepgram");
    object.insert("enabled".to_owned(), serde_json::Value::Bool(false));
    Some(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[cfg(feature = "browser-use")]
    use std::sync::atomic::AtomicBool;
    #[cfg(feature = "browser-use")]
    use std::time::Duration;

    #[cfg(feature = "browser-use")]
    use async_trait::async_trait;
    #[cfg(feature = "browser-use")]
    use nomifun_db::{DbError, models::ClientPreference};
    #[cfg(feature = "browser-use")]
    use nomifun_browser_platform::{
        BrowserErrorCode, BrowserHostDriver, BrowserHostFactory, BrowserHostId,
        BrowserIdentityMode, BrowserLaneDriver, BrowserOperation,
        BrowserOperationResult, BrowserPlatformError, DriverOperationContext, HostLaunchRequest,
        HostLifecycleState, HubConfig, LaneFreezeOutcome, LaneLaunchRequest,
        SnapshotComponentCoverage,
    };
    #[cfg(feature = "browser-use")]
    use tokio::sync::{Notify, Semaphore};

    #[cfg(feature = "browser-use")]
    #[derive(Default)]
    struct BrowserPreferenceTestRepository {
        rows: std::sync::Mutex<Vec<ClientPreference>>,
        writes: std::sync::Mutex<Vec<(String, String)>>,
        fail_reads: AtomicBool,
    }

    #[cfg(feature = "browser-use")]
    impl BrowserPreferenceTestRepository {
        fn with_rows(rows: &[(&str, &str)]) -> Self {
            Self {
                rows: std::sync::Mutex::new(
                    rows.iter()
                        .enumerate()
                        .map(|(index, (key, value))| ClientPreference {
                            id: index as i64 + 1,
                            key: (*key).to_owned(),
                            value: (*value).to_owned(),
                            updated_at: 0,
                        })
                        .collect(),
                ),
                ..Default::default()
            }
        }

        fn failing_reads() -> Self {
            Self {
                fail_reads: AtomicBool::new(true),
                ..Default::default()
            }
        }

        fn writes(&self) -> Vec<(String, String)> {
            self.writes
                .lock()
                .expect("browser preference write probe poisoned")
                .clone()
        }
    }

    #[cfg(feature = "browser-use")]
    #[async_trait]
    impl IClientPreferenceRepository for BrowserPreferenceTestRepository {
        async fn get_all(&self) -> Result<Vec<ClientPreference>, DbError> {
            self.get_by_keys(&[]).await
        }

        async fn get_by_keys(&self, keys: &[&str]) -> Result<Vec<ClientPreference>, DbError> {
            if self.fail_reads.load(Ordering::Acquire) {
                return Err(DbError::Init("synthetic preference read failure".to_owned()));
            }
            let rows = self
                .rows
                .lock()
                .expect("browser preference row probe poisoned");
            if keys.is_empty() {
                return Ok(rows.clone());
            }
            Ok(rows
                .iter()
                .filter(|row| keys.contains(&row.key.as_str()))
                .cloned()
                .collect())
        }

        async fn upsert_batch(&self, entries: &[(&str, &str)]) -> Result<(), DbError> {
            self.writes
                .lock()
                .expect("browser preference write probe poisoned")
                .extend(
                    entries
                        .iter()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
                );
            Ok(())
        }

        async fn delete_keys(&self, _keys: &[&str]) -> Result<(), DbError> {
            Ok(())
        }
    }

    #[cfg(feature = "browser-use")]
    struct ShutdownProbe {
        launches: AtomicUsize,
        shutdown_calls: AtomicUsize,
        fail_shutdowns_remaining: AtomicUsize,
        block_shutdown: AtomicBool,
        shutdown_release: Semaphore,
        shutdown_changed: Notify,
    }

    #[cfg(feature = "browser-use")]
    impl ShutdownProbe {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                launches: AtomicUsize::new(0),
                shutdown_calls: AtomicUsize::new(0),
                fail_shutdowns_remaining: AtomicUsize::new(0),
                block_shutdown: AtomicBool::new(false),
                shutdown_release: Semaphore::new(0),
                shutdown_changed: Notify::new(),
            })
        }

        async fn wait_for_shutdown_calls(&self, expected: usize) {
            loop {
                if self.shutdown_calls.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.shutdown_changed.notified().await;
            }
        }
    }

    #[cfg(feature = "browser-use")]
    struct PlatformShutdownStepProbe {
        label: &'static str,
        failure_message: &'static str,
        calls: AtomicUsize,
        completions: AtomicUsize,
        fail_calls_remaining: AtomicUsize,
        block: AtomicBool,
        release: Semaphore,
        changed: Notify,
        events: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[cfg(feature = "browser-use")]
    impl PlatformShutdownStepProbe {
        fn new(
            label: &'static str,
            failure_message: &'static str,
            events: Arc<std::sync::Mutex<Vec<String>>>,
        ) -> Arc<Self> {
            Arc::new(Self {
                label,
                failure_message,
                calls: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                fail_calls_remaining: AtomicUsize::new(0),
                block: AtomicBool::new(false),
                release: Semaphore::new(0),
                changed: Notify::new(),
                events,
            })
        }

        fn step(self: &Arc<Self>) -> BrowserShutdownStep {
            let probe = Arc::clone(self);
            BrowserShutdownStep::new(self.label, move || {
                let probe = Arc::clone(&probe);
                async move { probe.run().await }
            })
        }

        async fn run(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.record("start");
            self.changed.notify_waiters();
            if self.block.load(Ordering::Acquire) {
                let permit = self
                    .release
                    .acquire()
                    .await
                    .map_err(|_| format!("{} test release closed", self.label))?;
                permit.forget();
            }
            self.record("finish");
            self.completions.fetch_add(1, Ordering::AcqRel);
            self.changed.notify_waiters();

            if self
                .fail_calls_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(self.failure_message.to_owned());
            }
            Ok(())
        }

        fn record(&self, phase: &str) {
            self.events
                .lock()
                .expect("platform shutdown event log poisoned")
                .push(format!("{}:{phase}", self.label));
        }

        async fn wait_for_calls(&self, expected: usize) {
            self.wait_for_counter(&self.calls, expected).await;
        }

        async fn wait_for_completions(&self, expected: usize) {
            self.wait_for_counter(&self.completions, expected).await;
        }

        async fn wait_for_counter(&self, counter: &AtomicUsize, expected: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let changed = self.changed.notified();
                    if counter.load(Ordering::Acquire) >= expected {
                        return;
                    }
                    changed.await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "{} did not reach expected count {expected}; calls={}, completions={}",
                    self.label,
                    self.calls.load(Ordering::Acquire),
                    self.completions.load(Ordering::Acquire)
                )
            });
        }

        fn release_one(&self) {
            self.release.add_permits(1);
        }
    }

    #[cfg(feature = "browser-use")]
    struct ShutdownTestLane;

    #[cfg(feature = "browser-use")]
    #[async_trait]
    impl BrowserLaneDriver for ShutdownTestLane {
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

        async fn freeze(&self) -> Result<LaneFreezeOutcome, BrowserPlatformError> {
            Ok(LaneFreezeOutcome::Unsupported)
        }
    }

    #[cfg(feature = "browser-use")]
    struct ShutdownTestHost {
        host_id: BrowserHostId,
        epoch: u64,
        probe: Arc<ShutdownProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait]
    impl BrowserHostDriver for ShutdownTestHost {
        fn host_id(&self) -> BrowserHostId {
            self.host_id.clone()
        }

        fn epoch(&self) -> u64 {
            self.epoch
        }

        fn state(&self) -> HostLifecycleState {
            HostLifecycleState::Running
        }

        async fn open_lane(
            &self,
            _request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            Ok(Arc::new(ShutdownTestLane))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            self.probe.shutdown_calls.fetch_add(1, Ordering::AcqRel);
            self.probe.shutdown_changed.notify_waiters();
            if self.probe.block_shutdown.load(Ordering::Acquire) {
                let permit = self
                    .probe
                    .shutdown_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
            }
            if self
                .probe
                .fail_shutdowns_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "synthetic shutdown failure",
                    true,
                    "retry",
                ));
            }
            Ok(())
        }
    }

    #[cfg(feature = "browser-use")]
    struct ShutdownTestFactory {
        probe: Arc<ShutdownProbe>,
    }

    #[cfg(feature = "browser-use")]
    #[async_trait]
    impl BrowserHostFactory for ShutdownTestFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            self.probe.launches.fetch_add(1, Ordering::AcqRel);
            Ok(Arc::new(ShutdownTestHost {
                host_id: request.host_id,
                epoch: request.browser_epoch,
                probe: Arc::clone(&self.probe),
            }))
        }
    }

    #[cfg(feature = "browser-use")]
    async fn shutdown_coordinator_fixture(
    ) -> (
        BrowserShutdownCoordinator,
        Arc<ShutdownProbe>,
        Arc<nomifun_browser_platform::BrowserSessionHub>,
    ) {
        use std::collections::BTreeSet;

        let probe = ShutdownProbe::new();
        let hub = Arc::new(nomifun_browser_platform::BrowserSessionHub::new(
            Arc::new(ShutdownTestFactory {
                probe: Arc::clone(&probe),
            }),
            HubConfig::default(),
        ));
        let owner = hub
            .issue_owner_lease("user", Some("conversation".to_owned()), "runtime")
            .unwrap();
        let client = hub
            .bind(nomifun_browser_platform::CallerIdentity {
                user_id: "user".to_owned(),
                conversation_id: Some("conversation".to_owned()),
                runtime_instance_id: "runtime".to_owned(),
                agent_id: Some("fixture".to_owned()),
                companion_id: None,
                execution_id: None,
                step_id: None,
                attempt_id: None,
                remote_connection_id: None,
                surface: nomifun_browser_platform::BrowserSurface::Native,
                owner_lease_id: owner.lease_id,
                capability_expires_at_ms: u64::MAX,
                allowed_operations: BTreeSet::from([
                    nomifun_browser_platform::BrowserOperationKind::Manage,
                ]),
            })
            .unwrap();
        client
            .open(
                Some("shutdown-fixture"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();

        (
            BrowserShutdownCoordinator::new(Arc::clone(&hub)),
            probe,
            hub,
        )
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_display_mode_migration_is_authoritative_and_persistable() {
        // Only a versioned explicit user choice is preserved.
        assert_eq!(
            resolve_browser_display_mode(Some("headless"), Some("2")),
            ("headless", false)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("external"), Some("\"2\"")),
            ("external", false)
        );
        // Every unversioned historical value converges once to the silent
        // default, including the previous inferred external setting.
        assert_eq!(
            resolve_browser_display_mode(Some("external"), None),
            ("headless", true)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("headless"), Some("1")),
            ("headless", true)
        );
        assert_eq!(
            resolve_browser_display_mode(None, None),
            ("headless", true)
        );
        // Missing or invalid mode under the current marker is repaired.
        assert_eq!(
            resolve_browser_display_mode(Some("embedded"), Some("2")),
            ("headless", true)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("invalid"), Some("2")),
            ("headless", true)
        );
        assert_eq!(
            resolve_browser_display_mode(None, Some("2")),
            ("headless", true),
            "a marker without a valid mode fails safe to silent headless"
        );
        assert_eq!(
            resolve_browser_display_mode(Some("  \"headless\"  "), Some("  \"2\"  ")),
            ("headless", false)
        );
        // Only the user's explicit external policy launches a visible
        // Primary Host; everything else stays truly headless.
        assert!(primary_host_is_headful("external"));
        assert!(!primary_host_is_headful("headless"));
        assert!(!primary_host_is_headful("embedded"));
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_display_mode_read_failure_never_persists_a_default() {
        let repo = BrowserPreferenceTestRepository::failing_reads();

        assert_eq!(
            load_browser_startup_preferences(&repo).await,
            BrowserStartupPreferences::default()
        );
        assert!(
            repo.writes().is_empty(),
            "a repository read failure must not be treated as missing preferences"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_display_mode_migrates_unversioned_external_to_headless_once() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"external\""),
            ("agent.browserUse.source", "\"system\""),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(
            preferences.display_mode, "headless",
            "unversioned external state must not keep opening an operating-system window"
        );
        assert_eq!(
            repo.writes(),
            vec![
                (
                    BROWSER_DISPLAY_MODE_PREF_KEY.to_owned(),
                    "\"headless\"".to_owned()
                ),
                (
                    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY.to_owned(),
                    BROWSER_DISPLAY_MODE_POLICY_VERSION.to_owned()
                ),
            ]
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_display_mode_preserves_versioned_explicit_external() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"external\""),
            (
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                BROWSER_DISPLAY_MODE_POLICY_VERSION,
            ),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "external");
        assert!(repo.writes().is_empty());
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn fresh_install_persists_headless_display_mode() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "headless");
        assert_eq!(
            repo.writes(),
            vec![
                (
                    BROWSER_DISPLAY_MODE_PREF_KEY.to_owned(),
                    "\"headless\"".to_owned()
                ),
                (
                    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY.to_owned(),
                    BROWSER_DISPLAY_MODE_POLICY_VERSION.to_owned()
                ),
            ]
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn invalid_display_mode_is_repaired_to_headless() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"visible\""),
            (
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                BROWSER_DISPLAY_MODE_POLICY_VERSION,
            ),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "headless");
        assert_eq!(
            repo.writes(),
            vec![
                (
                    BROWSER_DISPLAY_MODE_PREF_KEY.to_owned(),
                    "\"headless\"".to_owned()
                ),
                (
                    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY.to_owned(),
                    BROWSER_DISPLAY_MODE_POLICY_VERSION.to_owned()
                ),
            ],
            "malformed configuration must converge to the silent headless default"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_inventory_lag_emits_resync_then_forwards_newest_event() {
        let (source_tx, source_rx) = tokio::sync::broadcast::channel(1);
        source_tx
            .send(nomifun_browser_platform::BrowserInventoryEvent {
                sequence: 1,
                change_kind: "lane_created".to_owned(),
                lane_id: None,
                user_id: Some("owner-a".to_owned()),
                conversation_id: None,
                at_ms: 10,
            })
            .unwrap();
        source_tx
            .send(nomifun_browser_platform::BrowserInventoryEvent {
                sequence: 2,
                change_kind: "lane_running".to_owned(),
                lane_id: None,
                user_id: Some("owner-a".to_owned()),
                conversation_id: None,
                at_ms: 20,
            })
            .unwrap();
        drop(source_tx);

        let event_bus = Arc::new(BroadcastEventBus::new(4));
        let mut user_events = event_bus.subscribe_user();
        let manager = Arc::new(WebSocketManager::new());
        let (owner_tx, mut owner_rx) = tokio::sync::mpsc::channel(4);
        let (other_tx, mut other_rx) = tokio::sync::mpsc::channel(4);
        manager.add_client("owner-a".into(), "owner-token".into(), owner_tx);
        manager.add_client("owner-b".into(), "other-token".into(), other_tx);

        let task = tokio::spawn(forward_browser_inventory_events(
            source_rx,
            event_bus,
            manager,
            Arc::from("installation-owner"),
        ));

        for receiver in [&mut owner_rx, &mut other_rx] {
            let outbound = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
                .await
                .expect("resync delivery must not stall")
                .expect("resync must be delivered");
            let nomifun_realtime::WsOutbound::Text(text) = outbound else {
                panic!("expected resync text event")
            };
            let event: nomifun_api_types::WebSocketMessage<serde_json::Value> =
                serde_json::from_str(&text).unwrap();
            assert_eq!(event.name, BROWSER_INVENTORY_EVENT_NAME);
            assert_eq!(event.data["change_kind"], BROWSER_INVENTORY_RESYNC_CHANGE_KIND);
            assert_eq!(event.data["resync_required"], true);
            assert_eq!(event.data["skipped"], 1);
            assert!(event.data.get("sequence").is_none());
        }

        let newest = tokio::time::timeout(Duration::from_secs(1), user_events.recv())
            .await
            .expect("newest inventory event delivery must not stall")
            .expect("newest inventory event must be delivered");
        assert_eq!(newest.user_id, "owner-a");
        assert_eq!(newest.event.name, BROWSER_INVENTORY_EVENT_NAME);
        assert_eq!(newest.event.data["sequence"], 2);
        assert_eq!(newest.event.data["change_kind"], "lane_running");
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("closed source must stop the forwarder")
            .unwrap();
    }

    #[cfg(not(feature = "browser-use"))]
    #[tokio::test]
    async fn default_platform_shutdown_waits_for_gateway_only() {
        let server = Arc::new(
            nomifun_gateway::GatewayMcpServer::start()
                .await
                .expect("Gateway MCP server must start for the default shutdown test"),
        );
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], server.http_port()));
        let shutdown = BrowserPlatformShutdown::gateway_only(Some(Arc::clone(&server)));

        shutdown
            .shutdown()
            .await
            .expect("default shutdown must await Gateway quiescence");
        assert!(
            tokio::net::TcpListener::bind(address).await.is_ok(),
            "Gateway listener must be closed before the shutdown barrier resolves"
        );

        // The composed authority is idempotent after a successful flight.
        shutdown
            .shutdown()
            .await
            .expect("repeating default Gateway shutdown must be a no-op");
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_shutdown_coordinator_joins_concurrent_callers_and_caches_success() {
        let (coordinator, probe, _hub) = shutdown_coordinator_fixture().await;
        probe.block_shutdown.store(true, Ordering::Release);

        let first = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.shutdown().await })
        };
        probe.wait_for_shutdown_calls(1).await;
        let second = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.shutdown().await })
        };
        tokio::task::yield_now().await;
        assert_eq!(probe.shutdown_calls.load(Ordering::Acquire), 1);

        probe.shutdown_release.add_permits(1);
        assert!(first.await.unwrap().is_ok());
        assert!(second.await.unwrap().is_ok());
        assert!(coordinator.shutdown().await.is_ok());
        assert_eq!(
            probe.shutdown_calls.load(Ordering::Acquire),
            1,
            "successful shutdown must be cached"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_shutdown_waiter_timeout_does_not_cancel_real_shutdown() {
        let (coordinator, probe, _hub) = shutdown_coordinator_fixture().await;
        probe.block_shutdown.store(true, Ordering::Release);

        let timed_out = coordinator
            .shutdown_with_timeout(Duration::from_millis(20))
            .await;
        assert!(timed_out.is_err());
        probe.wait_for_shutdown_calls(1).await;

        let waiter = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.shutdown().await })
        };
        tokio::task::yield_now().await;
        assert_eq!(
            probe.shutdown_calls.load(Ordering::Acquire),
            1,
            "a timed-out waiter must leave the original Hub shutdown flight running"
        );

        probe.shutdown_release.add_permits(1);
        assert!(waiter.await.unwrap().is_ok());
        assert!(coordinator.shutdown().await.is_ok());
        assert_eq!(probe.shutdown_calls.load(Ordering::Acquire), 1);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_shutdown_failure_clears_flight_for_retry() {
        let (coordinator, probe, _hub) = shutdown_coordinator_fixture().await;
        probe
            .fail_shutdowns_remaining
            .store(2, Ordering::Release);

        let first = coordinator.shutdown().await;
        assert!(first.is_err());
        assert_eq!(
            probe.shutdown_calls.load(Ordering::Acquire),
            2,
            "one Hub shutdown pass retries retained Host authority before returning its first error"
        );

        assert!(coordinator.shutdown().await.is_ok());
        assert_eq!(
            probe.shutdown_calls.load(Ordering::Acquire),
            3,
            "failed flight must be cleared so retained Hub cleanup can retry"
        );
        assert!(coordinator.shutdown().await.is_ok());
        assert_eq!(probe.shutdown_calls.load(Ordering::Acquire), 3);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_shutdown_failure_is_shared_before_a_retry_starts() {
        let (coordinator, probe, _hub) = shutdown_coordinator_fixture().await;
        probe
            .fail_shutdowns_remaining
            .store(2, Ordering::Release);
        probe.block_shutdown.store(true, Ordering::Release);

        let first = coordinator.current_or_start_flight().await;
        probe.wait_for_shutdown_calls(1).await;
        let follower = coordinator.current_or_start_flight().await;
        assert!(Arc::ptr_eq(&first, &follower));
        probe.shutdown_release.add_permits(2);

        assert!(first.wait().await.is_err());
        assert!(follower.wait().await.is_err());
        assert_eq!(
            probe.shutdown_calls.load(Ordering::Acquire),
            2,
            "followers of the failed flight must receive that failure rather than silently starting a retry"
        );

        probe.block_shutdown.store(false, Ordering::Release);
        assert!(coordinator.shutdown().await.is_ok());
        assert_eq!(probe.shutdown_calls.load(Ordering::Acquire), 3);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_shutdown_concurrent_callers_share_one_ordered_flight() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gateway =
            PlatformShutdownStepProbe::new("gateway", "gateway failure", Arc::clone(&events));
        let browser_mcp =
            PlatformShutdownStepProbe::new("browser-mcp", "browser MCP failure", Arc::clone(&events));
        let hub = PlatformShutdownStepProbe::new("hub", "hub failure", Arc::clone(&events));
        gateway.block.store(true, Ordering::Release);
        browser_mcp.block.store(true, Ordering::Release);
        hub.block.store(true, Ordering::Release);

        let shutdown =
            BrowserPlatformShutdown::from_steps(Some(gateway.step()), Some(browser_mcp.step()));
        shutdown.set_hub_step(hub.step()).await;

        let first_flight = shutdown.current_or_start_flight().await;
        gateway.wait_for_calls(1).await;
        browser_mcp.wait_for_calls(1).await;
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            0,
            "Hub shutdown must not begin while either ingress is still draining"
        );

        let follower_flight = shutdown.current_or_start_flight().await;
        assert!(
            Arc::ptr_eq(&first_flight, &follower_flight),
            "concurrent callers must join the exact same ordered shutdown flight"
        );
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 1);

        gateway.release_one();
        gateway.wait_for_completions(1).await;
        tokio::task::yield_now().await;
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            0,
            "one completed ingress is insufficient to start Hub shutdown"
        );

        browser_mcp.release_one();
        hub.wait_for_calls(1).await;
        let events = events
            .lock()
            .expect("platform shutdown event log poisoned")
            .clone();
        let hub_start = events
            .iter()
            .position(|event| event == "hub:start")
            .expect("Hub start event");
        for ingress_finish in ["gateway:finish", "browser-mcp:finish"] {
            assert!(
                events
                    .iter()
                    .position(|event| event == ingress_finish)
                    .is_some_and(|position| position < hub_start),
                "{ingress_finish} must precede Hub shutdown: {events:?}"
            );
        }

        hub.release_one();
        let (first_result, follower_result) =
            tokio::join!(first_flight.wait(), follower_flight.wait());
        assert!(first_result.is_ok());
        assert!(follower_result.is_ok());

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 1);
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            1,
            "successful ordered shutdown must be cached"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_shutdown_without_hub_still_stops_both_ingresses() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gateway =
            PlatformShutdownStepProbe::new("gateway", "gateway failure", Arc::clone(&events));
        let browser_mcp =
            PlatformShutdownStepProbe::new("browser-mcp", "browser MCP failure", events);
        gateway.block.store(true, Ordering::Release);
        browser_mcp.block.store(true, Ordering::Release);

        let shutdown =
            BrowserPlatformShutdown::from_steps(Some(gateway.step()), Some(browser_mcp.step()));
        let waiter = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move { shutdown.shutdown().await })
        };

        gateway.wait_for_calls(1).await;
        browser_mcp.wait_for_calls(1).await;
        assert!(
            !waiter.is_finished(),
            "ingress-only shutdown must wait for both ingress cleanup flights"
        );

        gateway.release_one();
        browser_mcp.release_one();
        assert!(waiter.await.unwrap().is_ok());
        assert_eq!(gateway.completions.load(Ordering::Acquire), 1);
        assert_eq!(browser_mcp.completions.load(Ordering::Acquire), 1);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_shutdown_ingress_failures_block_hub_and_retry_in_order() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gateway = PlatformShutdownStepProbe::new(
            "Gateway MCP ingress",
            "synthetic ingress error",
            Arc::clone(&events),
        );
        let browser_mcp = PlatformShutdownStepProbe::new(
            "ACP Browser MCP ingress",
            "synthetic cleanup timed out",
            Arc::clone(&events),
        );
        let hub =
            PlatformShutdownStepProbe::new("Browser Hub", "synthetic Hub error", Arc::clone(&events));
        gateway
            .fail_calls_remaining
            .store(1, Ordering::Release);
        browser_mcp
            .fail_calls_remaining
            .store(1, Ordering::Release);

        let shutdown =
            BrowserPlatformShutdown::from_steps(Some(gateway.step()), Some(browser_mcp.step()));
        shutdown.set_hub_step(hub.step()).await;

        let error = shutdown.shutdown().await.unwrap_err().to_string();
        assert!(error.contains(
            "Gateway MCP ingress shutdown failed: synthetic ingress error"
        ));
        assert!(error.contains(
            "ACP Browser MCP ingress shutdown failed: synthetic cleanup timed out"
        ));
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 1);
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            0,
            "unconfirmed ingress quiescence must block Hub shutdown"
        );

        assert!(
            shutdown.shutdown().await.is_ok(),
            "a failed ordered flight must remain retryable"
        );
        assert_eq!(gateway.calls.load(Ordering::Acquire), 2);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 2);
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            1,
            "Hub shutdown may begin only after the retry confirms both ingress barriers"
        );

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(gateway.calls.load(Ordering::Acquire), 2);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 2);
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            1,
            "the first successful retry must be cached"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_shutdown_waiter_timeout_does_not_cancel_ordered_authority() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gateway =
            PlatformShutdownStepProbe::new("gateway", "gateway failure", Arc::clone(&events));
        let browser_mcp =
            PlatformShutdownStepProbe::new("browser-mcp", "browser MCP failure", Arc::clone(&events));
        let hub = PlatformShutdownStepProbe::new("hub", "hub failure", events);
        gateway.block.store(true, Ordering::Release);
        hub.block.store(true, Ordering::Release);

        let shutdown =
            BrowserPlatformShutdown::from_steps(Some(gateway.step()), Some(browser_mcp.step()));
        shutdown.set_hub_step(hub.step()).await;

        let timed_out_waiter = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                tokio::time::timeout(Duration::from_millis(20), shutdown.shutdown()).await
            })
        };
        gateway.wait_for_calls(1).await;
        browser_mcp.wait_for_calls(1).await;
        assert!(
            timed_out_waiter.await.unwrap().is_err(),
            "the caller-local timeout should expire while ingress remains blocked"
        );
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            0,
            "timing out a waiter must not skip the ingress barrier"
        );

        gateway.release_one();
        hub.wait_for_calls(1).await;
        let follower = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move { shutdown.shutdown().await })
        };
        tokio::task::yield_now().await;
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 1);
        assert_eq!(
            hub.calls.load(Ordering::Acquire),
            1,
            "a later waiter must join the original ordered authority"
        );

        hub.release_one();
        assert!(follower.await.unwrap().is_ok());
        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 1);
        assert_eq!(hub.calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn failed_browser_cleanup_keeps_database_close_barrier_closed() {
        let close_calls = AtomicUsize::new(0);

        let error = close_database_after_browser_platform_cleanup(
            Err(anyhow::anyhow!("ingress quiescence not confirmed")),
            || async {
                close_calls.fetch_add(1, Ordering::AcqRel);
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("ingress quiescence not confirmed"));
        assert_eq!(
            close_calls.load(Ordering::Acquire),
            0,
            "the database must remain available for a retry while browser ingress is unconfirmed"
        );
    }

    #[tokio::test]
    async fn successful_browser_cleanup_allows_database_close_once() {
        let close_calls = AtomicUsize::new(0);

        close_database_after_browser_platform_cleanup(Ok(()), || async {
            close_calls.fetch_add(1, Ordering::AcqRel);
        })
        .await
        .unwrap();

        assert_eq!(close_calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn failed_browser_cleanup_can_be_retried_before_database_close() {
        let close_calls = AtomicUsize::new(0);

        assert!(
            close_database_after_browser_platform_cleanup(
                Err(anyhow::anyhow!("transient ingress failure")),
                || async {
                    close_calls.fetch_add(1, Ordering::AcqRel);
                },
            )
            .await
            .is_err()
        );
        assert_eq!(close_calls.load(Ordering::Acquire), 0);

        close_database_after_browser_platform_cleanup(Ok(()), || async {
            close_calls.fetch_add(1, Ordering::AcqRel);
        })
        .await
        .unwrap();
        assert_eq!(
            close_calls.load(Ordering::Acquire),
            1,
            "a successful retry must close the database exactly once"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_shutdown_installing_hub_reopens_ingress_only_success() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gateway =
            PlatformShutdownStepProbe::new("gateway", "gateway failure", Arc::clone(&events));
        let browser_mcp =
            PlatformShutdownStepProbe::new("browser-mcp", "browser MCP failure", events);
        let shutdown =
            BrowserPlatformShutdown::from_steps(Some(gateway.step()), Some(browser_mcp.step()));

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 1);

        let (coordinator, hub_probe, _hub) = shutdown_coordinator_fixture().await;
        shutdown.set_hub_coordinator(coordinator).await;
        assert!(
            shutdown.shutdown().await.is_ok(),
            "installing a Hub after ingress-only success must reopen the ordered sequence"
        );
        assert_eq!(
            hub_probe.shutdown_calls.load(Ordering::Acquire),
            1,
            "the earlier ingress-only cached success must not permanently skip Hub shutdown"
        );
        assert_eq!(gateway.calls.load(Ordering::Acquire), 2);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 2);

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(hub_probe.shutdown_calls.load(Ordering::Acquire), 1);
        assert_eq!(gateway.calls.load(Ordering::Acquire), 2);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 2);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_shutdown_installing_browser_mcp_reopens_cached_success() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gateway =
            PlatformShutdownStepProbe::new("gateway", "gateway failure", Arc::clone(&events));
        let browser_mcp =
            PlatformShutdownStepProbe::new("browser-mcp", "browser MCP failure", events);
        let shutdown = BrowserPlatformShutdown::from_steps(Some(gateway.step()), None);

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);

        shutdown
            .set_browser_mcp_step(Some(browser_mcp.step()))
            .await;
        assert!(
            shutdown.shutdown().await.is_ok(),
            "registering Browser MCP after cached success must reopen shutdown"
        );
        assert_eq!(gateway.calls.load(Ordering::Acquire), 2);
        assert_eq!(
            browser_mcp.calls.load(Ordering::Acquire),
            1,
            "the newly registered ingress must be included in verified cleanup"
        );

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(gateway.calls.load(Ordering::Acquire), 2);
        assert_eq!(browser_mcp.calls.load(Ordering::Acquire), 1);
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn unsafe_orphan_recovery_degrades_browser_functionality() {
        let safe = nomi_browser_engine::profile::ProfileRecoveryReport::default();
        assert!(BrowserOrphanRecoveryOutcome::from_report(&safe).is_safe());

        // Resolved markers are not failures: startup recovery that verified a
        // live owner, terminated an orphan tree, removed ephemeral profiles,
        // or cleared stable markers must keep the browser feature enabled.
        let resolved = nomi_browser_engine::profile::ProfileRecoveryReport {
            markers_scanned: 4,
            process_trees_terminated: 1,
            ephemeral_profiles_removed: 2,
            stable_markers_cleared: 1,
            live_owners_preserved: 1,
            ..Default::default()
        };
        assert!(
            BrowserOrphanRecoveryOutcome::from_report(&resolved)
                .permits_host_composition(),
            "resolved markers must not degrade browser startup"
        );

        let with_failure = nomi_browser_engine::profile::ProfileRecoveryReport {
            failures: 1,
            ..Default::default()
        };
        assert!(matches!(
            BrowserOrphanRecoveryOutcome::from_report(&with_failure),
            BrowserOrphanRecoveryOutcome::Degraded { .. }
        ));
        assert!(
            !BrowserOrphanRecoveryOutcome::from_report(&with_failure)
                .permits_host_composition()
        );

        let with_preserved_profile = nomi_browser_engine::profile::ProfileRecoveryReport {
            profiles_preserved: 1,
            ..Default::default()
        };
        assert!(matches!(
            BrowserOrphanRecoveryOutcome::from_report(&with_preserved_profile),
            BrowserOrphanRecoveryOutcome::Degraded { .. }
        ));
        assert!(
            !BrowserOrphanRecoveryOutcome::from_report(&with_preserved_profile)
                .permits_host_composition()
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn orphan_recovery_worker_join_failure_degrades_browser_functionality() {
        let join_error = tokio::spawn(async {
            panic!("synthetic orphan recovery panic");
        })
        .await
        .unwrap_err();

        let outcome = BrowserOrphanRecoveryOutcome::from_join_error(&join_error);
        assert!(!outcome.permits_host_composition());
        assert!(matches!(
            outcome,
            BrowserOrphanRecoveryOutcome::Degraded { .. }
        ));
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn persisted_identity_startup_seed_declares_cookie_only_coverage() {
        let coverage = persisted_identity_seed_coverage();
        assert_eq!(coverage.current_origin, None);
        assert_eq!(
            coverage.cookies,
            SnapshotComponentCoverage::AllOrigins
        );
        assert_eq!(
            coverage.local_storage,
            SnapshotComponentCoverage::NotIncluded
        );
        assert_eq!(
            coverage.indexed_db,
            SnapshotComponentCoverage::NotIncluded
        );
    }

    fn test_config(data_dir: &Path) -> AppConfig {
        AppConfig {
            data_dir: data_dir.to_path_buf(),
            work_dir: data_dir.to_path_buf(),
            ..Default::default()
        }
    }


    #[test]
    fn executable_path_preserves_valid_unicode_exactly() {
        let path = Path::new("C:/Nomi $() `%TEMP%` & Friends/nomicore.exe");
        assert_eq!(
            require_utf8_executable_path(path).unwrap(),
            path.to_str().unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn executable_path_rejects_non_utf8_instead_of_replacing_bytes() {
        use std::os::unix::ffi::OsStringExt as _;

        let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![
            b'/', b't', b'm', b'p', b'/', b'n', b'o', b'm', b'i', 0xff,
        ]));
        assert!(require_utf8_executable_path(&path).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn executable_path_rejects_unpaired_utf16_instead_of_replacing_it() {
        use std::os::windows::ffi::OsStringExt as _;

        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            b'n' as u16,
            b'o' as u16,
            b'm' as u16,
            b'i' as u16,
            0xd800,
        ]));
        assert!(require_utf8_executable_path(&path).is_err());
    }


    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_telemetry_maps_only_real_measurements() {
        let telemetry = browser_resource_telemetry_from_measurements(
            16_000,
            7_500,
            Some(12),
            Some(2_048),
            std::collections::HashMap::from([(4_242, 2_048)]),
            Some(37.5),
        );

        assert_eq!(telemetry.total_memory_bytes, 16_000);
        assert_eq!(telemetry.available_memory_bytes, 7_500);
        assert_eq!(telemetry.logical_cpus, 12);
        assert_eq!(telemetry.chromium_rss_bytes, 2_048);
        assert_eq!(telemetry.host_rss_by_process_id.get(&4_242), Some(&2_048));
        assert!((telemetry.cpu_pressure - 0.375).abs() < f64::EPSILON);
        assert_eq!(telemetry.gpu_pressure, None);

        let unknown = browser_resource_telemetry_from_measurements(
            16_000,
            7_500,
            None,
            None,
            std::collections::HashMap::new(),
            None,
        );
        assert_eq!(unknown.logical_cpus, 0);
        assert_eq!(unknown.chromium_rss_bytes, 0);
        assert_eq!(unknown.cpu_pressure, 0.0);
        assert_eq!(unknown.gpu_pressure, None);
        assert_eq!(browser_cpu_pressure_from_percent(250.0), 1.0);
        assert_eq!(browser_cpu_pressure_from_percent(f32::NAN), 0.0);
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_process_tree_rss_counts_roots_and_descendants_once() {
        let (rss, hosts) = browser_process_tree_rss(
            &[10, 20, 10],
            [
                (10, Some(1), 100),
                (11, Some(10), 50),
                (12, Some(11), 25),
                (20, Some(1), 200),
                (99, Some(1), 9_999),
            ],
        );
        assert_eq!(rss, Some(375));
        assert_eq!(hosts.get(&10), Some(&175));
        assert_eq!(hosts.get(&20), Some(&200));
        assert_eq!(
            browser_process_tree_rss(&[10], [(11, Some(10), 50)]),
            (Some(50), std::collections::HashMap::from([(10, 50)]))
        );
        assert_eq!(
            browser_process_tree_rss(&[10], [(99, Some(1), 9_999)]),
            (None, std::collections::HashMap::new())
        );
        assert_eq!(
            browser_process_tree_rss(&[], [(10, Some(1), 100)]),
            (None, std::collections::HashMap::new())
        );
    }


    #[tokio::test]
    async fn test_app_services_from_memory_db() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let retired_data = tmp.path().join("local-ai/models/retired-model");
        std::fs::create_dir_all(&retired_data).unwrap();
        std::fs::write(retired_data.join("model.bin"), b"retired").unwrap();
        let config = test_config(tmp.path());
        let services = AppServices::from_config(db, &config).await.unwrap();

        assert!(!tmp.path().join("local-ai").exists());
        assert_eq!(
            services
                .agent_runtime_registry
                .reset_persisted_nomi_session(
                    &nomifun_common::ConversationId::new().into_string(),
                    nomifun_common::now_ms(),
                )
                .await
                .unwrap(),
            nomifun_ai_agent::NomiSessionResetOutcome::AlreadyAbsent,
            "product composition must configure the exact Nomi session directory used by the factory"
        );

        // JWT service should be functional
        let test_user_id = "0190f5fe-7c00-7a00-8000-000000000001";
        let token = services.jwt_service.sign(test_user_id, "testuser").unwrap();
        let payload = services.jwt_service.verify(&token).unwrap();
        assert_eq!(payload.user_id.as_str(), test_user_id);

        // The installation owner exists but has no login credential yet.
        let has_users = services.user_repo.has_users().await.unwrap();
        assert!(!has_users); // empty owner password → not counted

        services.database.close().await;
    }

    #[tokio::test]
    async fn from_config_error_closes_supplied_database() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let database_observer = db.clone();
        sqlx::query("DELETE FROM users")
            .execute(db.pool())
            .await
            .unwrap();
        let tmp = tempfile::TempDir::new().unwrap();

        let error = match AppServices::from_config(db, &test_config(tmp.path())).await {
            Ok(_) => panic!("missing installation owner must fail startup"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("installation owner is missing"),
            "unexpected startup error: {error:#}"
        );
        assert!(
            database_observer.pool().is_closed(),
            "from_config must close the supplied database after dropping partial startup resources"
        );
    }

    #[tokio::test]
    async fn test_jwt_secret_persisted_to_db() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let config = test_config(tmp.path());
        let services = AppServices::from_config(db, &config).await.unwrap();

        // The installation owner should now have a persisted jwt_secret.
        let installation_owner = services.user_repo.get_system_user().await.unwrap();
        let jwt_secret = installation_owner.unwrap().jwt_secret;
        assert!(jwt_secret.is_some());
        assert!(!jwt_secret.unwrap().is_empty());

        services.database.close().await;
    }

    #[tokio::test]
    async fn test_app_services_uses_supplied_app_version() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let config = AppConfig {
            app_version: "9.9.9".to_string(),
            ..test_config(tmp.path())
        };
        let services = AppServices::from_config(db, &config).await.unwrap();

        assert_eq!(services.app_version, "9.9.9");

        services.database.close().await;
    }

    // -- legacy speech preference migration --

    fn legacy_speech_preference() -> serde_json::Value {
        serde_json::json!({
            "enabled": true,
            "provider": "openai",
            "model": "whisper-1",
            "language": "zh",
            "auto_send": true,
            "openai": {
                "api_key": "sk-legacy-secret",
                "base_url": "https://api.openai.com/v1",
                "model": "whisper-1"
            }
        })
    }

    #[test]
    fn rewrite_legacy_speech_preference_disables_and_strips_credentials() {
        let rewritten =
            rewrite_legacy_speech_preference(&legacy_speech_preference().to_string())
                .expect("embedded-credential config without provider_id must be rewritten");
        let rewritten: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(
            rewritten,
            serde_json::json!({
                "enabled": false,
                "provider": "openai",
                "model": "whisper-1",
                "language": "zh",
                "auto_send": true
            }),
            "non-credential fields must be preserved verbatim"
        );

        // Idempotent: the rewritten value no longer matches the legacy shape.
        assert_eq!(rewrite_legacy_speech_preference(&rewritten.to_string()), None);
    }

    #[test]
    fn rewrite_legacy_speech_preference_handles_deepgram_and_null_provider_id() {
        let value = serde_json::json!({
            "enabled": true,
            "provider": "deepgram",
            "provider_id": null,
            "deepgram": {"api_key": "dg-secret", "model": "nova-2"}
        });
        let rewritten = rewrite_legacy_speech_preference(&value.to_string())
            .expect("null provider_id counts as absent");
        let rewritten: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(rewritten["enabled"], serde_json::json!(false));
        assert!(rewritten.get("deepgram").is_none());
        assert!(rewritten.get("openai").is_none());
    }

    #[test]
    fn rewrite_legacy_speech_preference_leaves_non_legacy_values_untouched() {
        for value in [
            // Catalog mode: provider_id present (even with a stale embedded block).
            serde_json::json!({
                "enabled": true,
                "provider": "openai",
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000001",
                "model": "whisper-1",
                "openai": {"api_key": "sk-stale", "model": "whisper-1"}
            })
            .to_string(),
            // Credential-less shell (frontend historically persisted empty keys).
            serde_json::json!({
                "enabled": true,
                "provider": "openai",
                "openai": {"api_key": "  ", "model": "whisper-1"}
            })
            .to_string(),
            // No embedded blocks at all.
            serde_json::json!({"enabled": false, "provider": "openai"}).to_string(),
            // Unparseable / non-object values are never touched.
            "not-json".to_string(),
            serde_json::json!(["enabled"]).to_string(),
        ] {
            assert_eq!(
                rewrite_legacy_speech_preference(&value),
                None,
                "value must be left untouched: {value}"
            );
        }
    }

    #[tokio::test]
    async fn migrate_legacy_speech_preference_rewrites_both_keys_and_is_idempotent() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo = SqliteClientPreferenceRepository::new(db.pool().clone());

        let legacy = legacy_speech_preference().to_string();
        let unrelated = "\"dark\"";
        repo.upsert_batch(&[
            ("tools.speechToText", legacy.as_str()),
            ("speechToText", legacy.as_str()),
            ("theme", unrelated),
        ])
        .await
        .unwrap();

        migrate_legacy_speech_preference(&repo).await;

        let read_value = |rows: &[nomifun_db::models::ClientPreference], key: &str| {
            rows.iter()
                .find(|row| row.key == key)
                .map(|row| row.value.clone())
                .unwrap_or_else(|| panic!("preference '{key}' must survive the migration"))
        };
        let rows = repo.get_all().await.unwrap();
        for key in ["tools.speechToText", "speechToText"] {
            let migrated: serde_json::Value =
                serde_json::from_str(&read_value(&rows, key)).unwrap();
            assert_eq!(migrated["enabled"], serde_json::json!(false), "{key}");
            assert!(migrated.get("openai").is_none(), "{key} keeps no credential");
            assert_eq!(migrated["model"], serde_json::json!("whisper-1"), "{key}");
        }
        assert_eq!(read_value(&rows, "theme"), unrelated);

        // Second boot: no-op (values byte-identical after another pass).
        migrate_legacy_speech_preference(&repo).await;
        let rows_after = repo.get_all().await.unwrap();
        for key in ["tools.speechToText", "speechToText", "theme"] {
            assert_eq!(
                read_value(&rows_after, key),
                read_value(&rows, key),
                "second migration pass must not rewrite '{key}'"
            );
        }

        db.close().await;
    }

    #[tokio::test]
    async fn migrate_legacy_speech_preference_is_a_noop_without_speech_keys() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo = SqliteClientPreferenceRepository::new(db.pool().clone());
        repo.upsert_batch(&[("theme", "\"light\"")]).await.unwrap();

        migrate_legacy_speech_preference(&repo).await;

        let rows = repo.get_all().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "theme");
        db.close().await;
    }
}
