//! Shared application services for dependency injection.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nomifun_ai_agent::{
    AgentFactoryDeps, AgentRegistry, AgentRuntimeRegistry,
    InMemoryAgentRuntimeRegistry, build_agent_factory, build_agent_model_config_resolver,
};
use nomifun_api_types::{GatewayMcpConfig, RequirementMcpConfig};
use nomifun_auth::{
    AuthPolicy, CookieConfig, InstanceTokenValidator, JwtService, QrTokenStore, resolve_jwt_secret,
};
use nomifun_common::OnConversationDelete;
use nomifun_conversation::runtime_state::ConversationRuntimeStateService;
use nomifun_conversation::{
    ExecutionConversationBoundary, RepositoryExecutionConversationBoundary,
};
use nomifun_db::{
    Database, IAgentMetadataRepository, IInstanceTokenRepository,
    IConversationRepository, IMcpServerRepository, IProviderModelCapabilityRepository,
    IProviderModelRepository, IProviderRepository,
    IUserRepository, SqliteAgentMetadataRepository,
    SqliteConversationRepository, SqliteInstanceTokenRepository, SqliteMcpServerRepository,
    SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
    SqliteProviderRepository,
    SqliteTerminalRepository, SqliteUserRepository,
};
#[cfg(feature = "browser-use")]
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

/// Stateless Creative Studio Template draft bridge.
///
/// This intentionally uses the provider factory's stateless completion
/// surface: one exact managed Chat config, one user message, and a
/// construction-time empty tool table. It creates no Conversation/Skill/MCP
/// state and schedules no product-level retry or model failover. The selected
/// provider may still perform its existing bounded transport negotiation while
/// the downstream receiver remains live.
pub(crate) struct AgentTemplateDraftRunner {
    pub model_invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    pub workspace: PathBuf,
}

#[async_trait::async_trait]
impl nomifun_workshop::TemplateDraftRunner for AgentTemplateDraftRunner {
    async fn run(
        &self,
        request: nomifun_workshop::TemplateDraftRunRequest,
    ) -> Result<String, nomifun_common::AppError> {
        let completion = async {
            let config = nomifun_ai_agent::resolve_provider_config(
                self.model_invoke.as_ref(),
                &request.provider_id,
                &request.model,
                &self.workspace,
            )
            .await?;
            nomifun_ai_agent::one_shot_completion_bounded(
                &config,
                request.system_prompt,
                vec![nomifun_ai_agent::user_message(request.user_text)],
                nomifun_workshop::TEMPLATE_DRAFT_MAX_TOKENS,
                nomifun_workshop::MAX_TEMPLATE_DRAFT_RESPONSE_BYTES,
            )
            .await
        };

        tokio::time::timeout(
            Duration::from_secs(nomifun_workshop::TEMPLATE_DRAFT_TIMEOUT_SECS),
            completion,
        )
        .await
        .map_err(|_| {
            nomifun_common::AppError::Timeout(
                "Creative Studio template draft generation timed out".into(),
            )
        })?
    }
}

/// Workshop text-node executor over the production Agent Chat stack.
///
/// The selected model's persisted Chat capability is resolved by
/// `ModelInvokeService`, then the shared Agent provider factory performs the
/// completion. Consequently OpenAI-compatible, Anthropic Messages, Gemini and
/// Bedrock text calls use the same serializer/auth rules as live conversations;
/// this bridge contains no platform-name routing table.
struct AgentCreationTextExecutor {
    model_invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    workspace: PathBuf,
}

#[async_trait::async_trait]
impl nomifun_creation::CreationTextExecutor for AgentCreationTextExecutor {
    async fn complete(
        &self,
        request: nomifun_creation::CreationTextRequest,
    ) -> Result<String, nomifun_creation::CreationError> {
        let config = nomifun_ai_agent::factory::provider_config::resolve_provider_config(
            self.model_invoke.as_ref(),
            &request.provider_id,
            &request.model,
            &self.workspace,
        )
        .await
        .map_err(|error| nomifun_creation::CreationError::config(error.to_string()))?;
        nomifun_ai_agent::factory::provider_config::one_shot_completion(
            &config,
            &request.system,
            vec![nomifun_ai_agent::factory::provider_config::user_message(
                request.prompt,
            )],
            request.max_tokens,
        )
        .await
        .map_err(|error| nomifun_creation::CreationError::provider_error(error.to_string()))
    }
}

#[cfg(feature = "browser-use")]
struct BrowserPlatformTasks {
    shutdown: BrowserPlatformTaskShutdown,
    sweep: tokio::task::JoinHandle<()>,
    events: tokio::task::JoinHandle<()>,
    telemetry: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "browser-use")]
impl Drop for BrowserPlatformTasks {
    fn drop(&mut self) {
        // Cancellation makes every supervised loop return cooperatively. Abort
        // as well so AppServices Drop never detaches a loop that is currently
        // inside Hub I/O; there is no separately spawned inner worker.
        self.shutdown.cancel();
        self.sweep.abort();
        self.events.abort();
        self.telemetry.abort();
    }
}

#[cfg(feature = "browser-use")]
#[derive(Clone)]
struct BrowserPlatformTaskShutdown {
    state: tokio::sync::watch::Sender<bool>,
}

#[cfg(feature = "browser-use")]
impl Default for BrowserPlatformTaskShutdown {
    fn default() -> Self {
        let (state, _receiver) = tokio::sync::watch::channel(false);
        Self { state }
    }
}

#[cfg(feature = "browser-use")]
impl BrowserPlatformTaskShutdown {
    fn cancel(&self) {
        self.state.send_replace(true);
    }

    fn is_cancelled(&self) -> bool {
        *self.state.borrow()
    }

    async fn cancelled(&self) {
        let mut state = self.state.subscribe();
        loop {
            if *state.borrow() {
                return;
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    }
}

#[cfg(feature = "browser-use")]
const BROWSER_PLATFORM_LOOP_RESTART_INITIAL: Duration = Duration::from_millis(100);
#[cfg(feature = "browser-use")]
const BROWSER_PLATFORM_LOOP_RESTART_MAX: Duration = Duration::from_secs(5);

#[cfg(feature = "browser-use")]
fn browser_platform_loop_restart_delay(
    failure_count: u32,
    initial: Duration,
    maximum: Duration,
) -> Duration {
    let multiplier = 1_u32
        .checked_shl(failure_count.min(31))
        .unwrap_or(u32::MAX);
    initial.saturating_mul(multiplier).min(maximum)
}

/// Run exactly one instance of a Browser background loop at a time.
///
/// The loop future executes inline in this supervisor task. This is important:
/// aborting the retained supervisor handle drops the active future instead of
/// detaching an untracked inner Tokio task. Both synchronous factory panics and
/// asynchronous loop panics are contained, then retried with capped backoff.
#[cfg(feature = "browser-use")]
async fn supervise_browser_platform_loop<F, Fut>(
    loop_name: &'static str,
    shutdown: BrowserPlatformTaskShutdown,
    mut loop_factory: F,
    restart_initial: Duration,
    restart_maximum: Duration,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use futures_util::FutureExt as _;

    let mut failure_count = 0_u32;
    loop {
        if shutdown.is_cancelled() {
            return;
        }

        let loop_future = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            loop_factory()
        })) {
            Ok(loop_future) => loop_future,
            Err(_) => {
                let restart_delay = browser_platform_loop_restart_delay(
                    failure_count,
                    restart_initial,
                    restart_maximum,
                );
                failure_count = failure_count.saturating_add(1);
                tracing::error!(
                    loop_name,
                    restart_delay_ms = restart_delay.as_millis(),
                    "browser platform loop factory panicked; restarting"
                );
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(restart_delay) => {}
                }
                continue;
            }
        };

        let termination = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            outcome = std::panic::AssertUnwindSafe(loop_future).catch_unwind() => outcome,
        };
        if shutdown.is_cancelled() {
            return;
        }

        let restart_delay = browser_platform_loop_restart_delay(
            failure_count,
            restart_initial,
            restart_maximum,
        );
        failure_count = failure_count.saturating_add(1);
        match termination {
            Ok(()) => tracing::warn!(
                loop_name,
                restart_delay_ms = restart_delay.as_millis(),
                "browser platform loop returned unexpectedly; restarting"
            ),
            Err(_) => tracing::error!(
                loop_name,
                restart_delay_ms = restart_delay.as_millis(),
                "browser platform loop panicked; restarting"
            ),
        }
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(restart_delay) => {}
        }
    }
}

#[cfg(feature = "browser-use")]
fn spawn_supervised_browser_platform_loop<F, Fut>(
    loop_name: &'static str,
    shutdown: BrowserPlatformTaskShutdown,
    loop_factory: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(supervise_browser_platform_loop(
        loop_name,
        shutdown,
        loop_factory,
        BROWSER_PLATFORM_LOOP_RESTART_INITIAL,
        BROWSER_PLATFORM_LOOP_RESTART_MAX,
    ))
}

#[cfg(feature = "browser-use")]
fn browser_resource_telemetry_from_measurements(
    total_memory_bytes: u64,
    available_memory_bytes: u64,
    logical_cpus: Option<usize>,
    chromium_rss_bytes: Option<u64>,
    host_rss_by_process_id: std::collections::HashMap<u32, u64>,
    host_cpu_pressure_by_process_id: std::collections::HashMap<u32, f64>,
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
        host_cpu_pressure_by_process_id,
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
const BROWSER_IDLE_TELEMETRY_PERIOD: Duration = Duration::from_secs(30);

#[cfg(feature = "browser-use")]
fn browser_telemetry_needs_process_scan(
    root_identities: &[nomifun_browser_platform::BrowserProcessIdentity],
) -> bool {
    !root_identities.is_empty()
}

#[cfg(feature = "browser-use")]
fn browser_resource_sample_period(sample_period_ms: u64, has_managed_hosts: bool) -> Duration {
    let normal_period = Duration::from_millis(sample_period_ms.max(1))
        .max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    if has_managed_hosts {
        normal_period
    } else {
        normal_period.max(BROWSER_IDLE_TELEMETRY_PERIOD)
    }
}

#[cfg(feature = "browser-use")]
async fn wait_for_browser_resource_sample(
    sample_period_ms: u64,
    has_managed_hosts: bool,
    events: &mut tokio::sync::broadcast::Receiver<
        nomifun_browser_platform::BrowserInventoryEvent,
    >,
) {
    let normal_period = browser_resource_sample_period(sample_period_ms, true);
    if has_managed_hosts {
        tokio::time::sleep(normal_period).await;
        return;
    }

    let idle_period = browser_resource_sample_period(sample_period_ms, false);
    tokio::select! {
        _ = tokio::time::sleep(idle_period) => {}
        result = events.recv() => {
            // A Host/Lane lifecycle event wakes the idle collector
            // immediately. A closed channel must not turn teardown into a
            // busy loop while the owning task is being aborted.
            if matches!(result, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                tokio::time::sleep(idle_period).await;
            }
        }
    }
}

#[cfg(feature = "browser-use")]
fn browser_startup_resource_policy(
    total_memory_bytes: u64,
    logical_cpus: Option<usize>,
) -> nomifun_browser_platform::ResourcePolicy {
    match (total_memory_bytes, logical_cpus.filter(|value| *value > 0)) {
        (total_memory_bytes, Some(logical_cpus)) if total_memory_bytes > 0 => {
            nomifun_browser_platform::ResourcePolicy::automatic(
                total_memory_bytes,
                logical_cpus,
            )
        }
        // Hardware discovery is expected to succeed during normal startup.
        // Retain the validated conservative policy only when the operating
        // system cannot provide an authoritative memory/CPU baseline.
        _ => {
            tracing::warn!(
                total_memory_bytes,
                logical_cpus = logical_cpus.unwrap_or(0),
                fallback_total_memory_bytes = 8_u64 * 1024 * 1024 * 1024,
                fallback_logical_cpus = 4,
                "browser hardware capacity discovery was unavailable; using the conservative fallback policy"
            );
            nomifun_browser_platform::ResourcePolicy::default()
        }
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
            // New installs let the trusted host choose per Lane: routine Agent
            // browsing launches Chromium `--headless=new` and never opens an
            // operating-system window, but a moment that needs the user's
            // supervision may surface one. A user may still pin `headless`
            // (never visible) or `external` (always visible) in Settings; the
            // removed embedded viewer is never selected as a presentation
            // surface.
            display_mode: "auto",
            source: "system".to_owned(),
            full_power: false,
            persistent_login: true,
        }
    }
}

/// Resolve the trusted application-level browser visibility policy.
///
/// The three supported values are user preferences, not Agent capabilities:
/// - `headless` keeps every Primary launch invisible, and forbids the Agent from
///   surfacing a window even at an attended moment.
/// - `auto` (default) lets the trusted host decide per Lane from the Agent's
///   declared intent and the action's risk tier: routine work stays silent, and
///   a moment that needs supervision (a login wall, an irreversible action) may
///   open a window. The Agent proposes; the host decides.
/// - `external` launches the Primary Host with a visible window unconditionally.
///
/// Version 3 introduces `auto` and makes it the default. Migration is lineage
/// aware, because not every stored `external` is a trustworthy user choice:
/// - **Unversioned** (pre-v2) state migrates to `auto` and never preserves
///   `external`. A pre-v2 `external` may have been *inferred* from the removed
///   `silent=false` setting rather than chosen, and version 2 deliberately
///   stopped such state from opening an operating-system window. That decision
///   is preserved here.
/// - **Version 2** state carries a real user choice, so an explicit `external`
///   is preserved — a user who opted into a visible window never silently loses
///   it — while `headless`, which was version 2's default for every
///   installation, moves to `auto`. That direction is safe: `auto` still keeps
///   routine browsing silent and only adds the ability to surface a window when
///   the user genuinely needs to intervene. Anyone who wants "never visible" can
///   still select `headless` explicitly.
///
/// Malformed or missing state at the current version fails closed to `auto` and
/// is repaired.
#[cfg(feature = "browser-use")]
fn resolve_browser_display_mode(
    display_mode: Option<&str>,
    policy_version: Option<&str>,
) -> (&'static str, bool) {
    let version = policy_version.map(|value| value.trim().trim_matches('"'));
    let stored = display_mode.map(|value| value.trim().trim_matches('"'));
    let is_current_version = version == Some(BROWSER_DISPLAY_MODE_POLICY_VERSION);
    if !is_current_version {
        // Only a version-2 marker proves the stored value was an explicit user
        // choice. Anything older is not trustworthy as an opt-in to a visible
        // window and must not resurrect one.
        return match (version, stored) {
            (Some(BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION), Some("external")) => {
                ("external", true)
            }
            _ => ("auto", true),
        };
    }

    match stored {
        Some("headless") => ("headless", false),
        Some("external") => ("external", false),
        Some("auto") => ("auto", false),
        _ => ("auto", true),
    }
}

#[cfg(feature = "browser-use")]
pub(crate) const BROWSER_DISPLAY_MODE_PREF_KEY: &str = "agent.browserUse.displayMode";
#[cfg(feature = "browser-use")]
pub(crate) const BROWSER_DISPLAY_MODE_VERSION_PREF_KEY: &str =
    "agent.browserUse.displayModeVersion";
#[cfg(feature = "browser-use")]
pub(crate) const BROWSER_DISPLAY_MODE_POLICY_VERSION: &str = "3";
/// The previous policy lineage. A marker of this version proves the stored mode
/// was an explicit version-2 user choice, which migration is allowed to preserve.
#[cfg(feature = "browser-use")]
pub(crate) const BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION: &str = "2";

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

/// Map the stored display-mode preference onto the Hub's visibility policy.
///
/// `external`/`headless` pin the mechanism and forbid the Hub from resolving it;
/// `auto` delegates. Anything unrecognized resolves to `auto`, matching
/// [`resolve_browser_display_mode`]'s fail-closed direction — which is silent,
/// because `auto` still launches headless and only escalates for a moment that
/// needs the user.
#[cfg(feature = "browser-use")]
fn browser_visibility_policy(
    display_mode: &str,
) -> nomifun_browser_platform::BrowserVisibilityPolicy {
    use nomifun_browser_platform::BrowserVisibilityPolicy;
    match display_mode {
        "external" => BrowserVisibilityPolicy::AlwaysHeadful,
        "headless" => BrowserVisibilityPolicy::AlwaysHeadless,
        _ => BrowserVisibilityPolicy::Auto,
    }
}

#[cfg(feature = "browser-use")]
fn primary_host_is_headful(display_mode: &str) -> bool {
    // The trusted application-level preference is the only input that can
    // make the Primary Host launch visible; Agent tool JSON, lane names and
    // request parameters have no path into this policy. Non-Primary Hosts
    // stay headless regardless, and explicit foregrounding remains a separate
    // trusted Host transition owned by the Hub.
    //
    // `auto` is deliberately *not* headful at startup: it starts silent and lets
    // the Hub surface a window only when a Lane reaches a moment that needs the
    // user's supervision. Only an explicit `external` preference launches
    // visible unconditionally.
    display_mode == "external"
}

/// Per-process byte count used to attribute a managed Chromium process tree.
///
/// Chromium is deliberately multi-process: one browser process plus GPU,
/// network/storage utilities, a crash handler, and one renderer per site
/// instance. Attribution sums this value across the whole tree, so the metric
/// has to be *additive* — a value that can be summed across sibling processes
/// without counting the same physical memory twice.
///
/// On Windows the working set fails that test. `WorkingSetSize` counts every
/// resident page a process maps, including pages **shared** with its siblings:
/// `chrome.dll` and the other shared images are mapped into every child, so
/// summing working sets charges those pages once per process. Measured on a
/// nine-process Chromium tree, 41% of the summed working set was shared pages
/// counted repeatedly (696 MiB summed working set against 413 MiB of private
/// bytes). Attributing that inflated total to one task made an ordinary
/// browsing session look like a leak and got its Lane reclaimed.
///
/// `sysinfo` exposes the private commit charge on Windows as
/// `Process::virtual_memory` (it maps to `PROCESS_MEMORY_COUNTERS_EX::
/// PrivateUsage`, not to an address-space size). Private commit is
/// per-process-exclusive, so it is safe to sum.
///
/// On Linux and macOS `virtual_memory` really is the virtual address-space size
/// (VSZ / `vsize`), which is meaningless here, and `sysinfo` exposes no
/// proportional set size. Those platforms therefore keep the resident-set
/// value. Summing RSS still over-counts shared pages, so tree totals there
/// remain an upper bound rather than an exact figure; the per-task budget is
/// sized to tolerate that.
#[cfg(feature = "browser-use")]
fn process_tree_attributable_bytes(process: &sysinfo::Process) -> u64 {
    #[cfg(windows)]
    {
        // Windows: private commit charge (PrivateUsage). Additive across the
        // tree because it excludes shared pages.
        process.virtual_memory()
    }
    #[cfg(not(windows))]
    {
        // Unix: resident set size. `virtual_memory` is VSZ here and must not be
        // substituted.
        process.memory()
    }
}

#[cfg(feature = "browser-use")]
fn browser_process_tree_rss<I>(
    root_identities: &[nomifun_browser_platform::BrowserProcessIdentity],
    processes: I,
) -> (
    Option<u64>,
    std::collections::HashMap<u32, u64>,
)
where
    I: IntoIterator<Item = (u32, Option<u32>, u64, u64)>,
{
    use std::collections::{HashMap, HashSet};

    if root_identities.is_empty() {
        return (None, HashMap::new());
    }

    let mut process_by_pid = HashMap::new();
    for (pid, parent_pid, rss_bytes, started_at_secs) in processes {
        process_by_pid.insert(pid, (parent_pid, rss_bytes, started_at_secs));
    }
    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &(parent_pid, _, child_started_at_secs)) in &process_by_pid {
        if let Some(parent_pid) = parent_pid {
            // Windows retains an orphan's original parent PID after the
            // parent exits. If that numeric PID is later reused by Chromium,
            // a parent-only walk attributes the old unrelated process (and
            // all of its children) to Browser Use. A real child cannot start
            // before its current parent, so reject that stale PID-reuse edge.
            let parent_is_not_newer = process_by_pid
                .get(&parent_pid)
                .is_none_or(|(_, _, parent_started_at_secs)| {
                    child_started_at_secs >= *parent_started_at_secs
                });
            if parent_is_not_newer {
                children_by_parent.entry(parent_pid).or_default().push(pid);
            }
        }
    }

    let mut total_visited = HashSet::new();
    let mut host_rss_by_process_id = HashMap::new();
    let mut total_rss_bytes = 0_u64;
    for root_identity in root_identities.iter().copied().collect::<HashSet<_>>() {
        let root_pid = root_identity.process_id;
        // Legacy fakes may only expose a PID. Production drivers always carry
        // the captured start time, and a mismatched or vanished root must not
        // be charged to Browser Use after PID reuse.
        let root_matches = match process_by_pid.get(&root_pid) {
            Some((_, _, observed_start)) => {
                root_identity.started_at_epoch_seconds == 0
                    || *observed_start == root_identity.started_at_epoch_seconds
            }
            None => root_identity.started_at_epoch_seconds == 0,
        };
        if !root_matches {
            continue;
        }
        let mut pending = vec![root_pid];
        let mut host_visited = HashSet::new();
        let mut host_measured = false;
        let mut host_rss_bytes = 0_u64;
        while let Some(pid) = pending.pop() {
            if !host_visited.insert(pid) {
                continue;
            }
            if let Some((_, rss_bytes, _)) = process_by_pid.get(&pid) {
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
fn browser_process_tree_cpu_pressure<I>(
    root_identities: &[nomifun_browser_platform::BrowserProcessIdentity],
    logical_cpus: usize,
    processes: I,
) -> std::collections::HashMap<u32, f64>
where
    I: IntoIterator<Item = (u32, Option<u32>, f32, u64)>,
{
    use std::collections::{HashMap, HashSet};

    if root_identities.is_empty() || logical_cpus == 0 {
        return HashMap::new();
    }

    let mut process_by_pid = HashMap::new();
    for (pid, parent_pid, cpu_usage_percent, started_at_secs) in processes {
        process_by_pid.insert(pid, (parent_pid, cpu_usage_percent, started_at_secs));
    }
    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &(parent_pid, _, child_started_at_secs)) in &process_by_pid {
        if let Some(parent_pid) = parent_pid {
            // Apply the same start-time edge validation as RSS attribution.
            // Otherwise a Chromium root PID reused after an unrelated orphan
            // was created could make Browser Use claim that process's CPU.
            let parent_is_not_newer = process_by_pid
                .get(&parent_pid)
                .is_none_or(|(_, _, parent_started_at_secs)| {
                    child_started_at_secs >= *parent_started_at_secs
                });
            if parent_is_not_newer {
                children_by_parent.entry(parent_pid).or_default().push(pid);
            }
        }
    }

    let machine_capacity_percent = (logical_cpus as f64) * 100.0;
    let mut host_cpu_pressure_by_process_id = HashMap::new();
    for root_identity in root_identities.iter().copied().collect::<HashSet<_>>() {
        let root_pid = root_identity.process_id;
        let root_matches = match process_by_pid.get(&root_pid) {
            Some((_, _, observed_start)) => {
                root_identity.started_at_epoch_seconds == 0
                    || *observed_start == root_identity.started_at_epoch_seconds
            }
            None => root_identity.started_at_epoch_seconds == 0,
        };
        if !root_matches {
            continue;
        }

        let mut pending = vec![root_pid];
        let mut host_visited = HashSet::new();
        let mut host_measured = false;
        let mut host_cpu_percent = 0.0_f64;
        while let Some(pid) = pending.pop() {
            if !host_visited.insert(pid) {
                continue;
            }
            if let Some((_, cpu_usage_percent, _)) = process_by_pid.get(&pid) {
                host_measured = true;
                let usage = f64::from(*cpu_usage_percent);
                if usage.is_finite() && usage > 0.0 {
                    host_cpu_percent += usage;
                }
            }
            if let Some(children) = children_by_parent.get(&pid) {
                pending.extend(children.iter().copied());
            }
        }
        if host_measured {
            host_cpu_pressure_by_process_id.insert(
                root_pid,
                (host_cpu_percent / machine_capacity_percent).clamp(0.0, 1.0),
            );
        }
    }

    host_cpu_pressure_by_process_id
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
    platform_tasks_shutdown: Option<BrowserPlatformTaskShutdown>,
}

#[cfg(feature = "browser-use")]
impl BrowserShutdownCoordinator {
    #[cfg(test)]
    fn new(hub: Arc<nomifun_browser_platform::BrowserSessionHub>) -> Self {
        Self {
            hub,
            state: Arc::new(tokio::sync::Mutex::new(
                BrowserShutdownCoordinatorState::default(),
            )),
            platform_tasks_shutdown: None,
        }
    }

    fn with_platform_tasks(
        hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
        platform_tasks_shutdown: BrowserPlatformTaskShutdown,
    ) -> Self {
        Self {
            hub,
            state: Arc::new(tokio::sync::Mutex::new(
                BrowserShutdownCoordinatorState::default(),
            )),
            platform_tasks_shutdown: Some(platform_tasks_shutdown),
        }
    }

    pub(crate) async fn shutdown(&self) -> anyhow::Result<()> {
        // Once ordered ingress shutdown reaches the Hub step, no telemetry,
        // sweep, or realtime loop may be restarted against a closing Hub.
        // Cancellation is idempotent, so concurrent shutdown callers share the
        // same no-restart boundary before joining the Hub shutdown flight.
        if let Some(shutdown) = &self.platform_tasks_shutdown {
            shutdown.cancel();
        }
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
    state: tokio::sync::Mutex<BrowserPlatformShutdownState>,
}

/// Cloneable, process-wide authority for ordered Gateway/Browser shutdown.
///
/// Gateway is always present when startup succeeds, including builds without
/// `browser-use`. Browser-enabled builds additionally install the Hub. One
/// shared flight first closes Gateway and waits for authoritative quiescence;
/// only then may Hub shutdown begin. This prevents Hub or DB teardown from
/// racing accepted requests, while a failed flight remains retryable.
#[derive(Clone)]
pub(crate) struct BrowserPlatformShutdown {
    inner: Arc<BrowserPlatformShutdownInner>,
}

impl Default for BrowserPlatformShutdown {
    fn default() -> Self {
        Self::from_steps(None)
    }
}

impl BrowserPlatformShutdown {
    #[cfg(not(feature = "browser-use"))]
    #[cfg(test)]
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
        Self::from_steps(gateway)
    }

    fn from_steps(gateway: Option<BrowserShutdownStep>) -> Self {
        Self {
            inner: Arc::new(BrowserPlatformShutdownInner {
                gateway,
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
        let hub = state.hub.clone();
        let inner = Arc::clone(&self.inner);
        let active_flight = Arc::clone(&flight);
        tokio::spawn(async move {
            let result = match tokio::spawn(run_browser_platform_shutdown(
                gateway,
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
    hub: Option<BrowserShutdownStep>,
) -> BrowserShutdownResult {
    // Gateway is the only ingress that can still own work against the Hub.
    // Keep the ordering explicit: a failed or unconfirmed Gateway barrier
    // leaves the Hub available for a later retry.
    await_browser_shutdown_step(gateway).await?;
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

}

#[cfg(feature = "browser-use")]
fn persisted_identity_seed_coverage() -> nomifun_browser_platform::SnapshotCoverage {
    nomifun_browser_platform::SnapshotCoverage::cookies_only()
}

#[cfg(feature = "browser-use")]
fn sample_browser_resources(
    system: &mut sysinfo::System,
    root_identities: &[nomifun_browser_platform::BrowserProcessIdentity],
    cpu_usage_percent: Option<f32>,
) -> nomifun_browser_platform::ResourceTelemetry {
    system.refresh_memory();
    let logical_cpus = std::thread::available_parallelism()
        .ok()
        .map(std::num::NonZeroUsize::get);
    if !browser_telemetry_needs_process_scan(root_identities) {
        return browser_resource_telemetry_from_measurements(
            system.total_memory(),
            system.available_memory(),
            logical_cpus,
            None,
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            cpu_usage_percent,
        );
    }
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu(),
    );
    // Re-probe the platform-native creation key at sample time. The Hub may
    // retain cleanup authority after Chromium has exited; a numeric PID can
    // then belong to an unrelated process. Probe failures fail closed by
    // omitting that Host from this sample rather than inventing memory use.
    let verified_identities = root_identities
        .iter()
        .copied()
        .filter(|identity| {
            if identity.platform_start_key == 0 {
                return identity.started_at_epoch_seconds == 0;
            }
            nomi_process_runtime::probe_process_identity(identity.process_id)
                .ok()
                .flatten()
                .is_some_and(|live| {
                    live.platform_start_key == identity.platform_start_key
                })
        })
        .collect::<Vec<_>>();
    let (chromium_rss_bytes, host_rss_by_process_id) = browser_process_tree_rss(
        &verified_identities,
        system.processes().values().map(|process| {
            (
                process.pid().as_u32(),
                process.parent().map(|pid| pid.as_u32()),
                process_tree_attributable_bytes(process),
                process.start_time(),
            )
        }),
    );
    let host_cpu_pressure_by_process_id = browser_process_tree_cpu_pressure(
        &verified_identities,
        logical_cpus.unwrap_or(0),
        system.processes().values().map(|process| {
            (
                process.pid().as_u32(),
                process.parent().map(|pid| pid.as_u32()),
                process.cpu_usage(),
                process.start_time(),
            )
        }),
    );
    browser_resource_telemetry_from_measurements(
        system.total_memory(),
        system.available_memory(),
        logical_cpus,
        chromium_rss_bytes,
        host_rss_by_process_id,
        host_cpu_pressure_by_process_id,
        cpu_usage_percent,
    )
}

#[cfg(feature = "browser-use")]
async fn run_browser_lifecycle_sweep_loop(
    hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
) {
    loop {
        // Read the live policy for every cycle. Resource-policy updates may
        // change the lifecycle cadence, and a fixed application-level interval
        // would silently ignore that setting until restart.
        let sweep_period_ms = hub
            .resource_policy()
            .await
            .lifecycle_sweep_period_ms
            .max(1);
        // The first sleep intentionally avoids an eager startup sweep before
        // runtimes have finished attaching their owner leases.
        tokio::time::sleep(Duration::from_millis(sweep_period_ms)).await;
        if let Err(error) = hub.sweep().await {
            tracing::warn!(
                code = ?error.code,
                retryable = error.retryable,
                "browser lifecycle sweep failed"
            );
        }
    }
}

#[cfg(feature = "browser-use")]
async fn run_browser_resource_telemetry_loop(
    hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
    mut system: sysinfo::System,
    mut inventory_events: tokio::sync::broadcast::Receiver<
        nomifun_browser_platform::BrowserInventoryEvent,
    >,
) {
    // CPU usage is delta-based. This first refresh establishes the baseline;
    // the immediate startup sample deliberately leaves CPU pressure unknown
    // while still publishing memory and process RSS.
    system.refresh_cpu_usage();
    let root_identities = hub.managed_host_process_identities().await;
    let initial_sample = sample_browser_resources(&mut system, &root_identities, None);
    hub.update_resource_telemetry(initial_sample).await;
    let mut has_managed_hosts = !root_identities.is_empty();
    loop {
        let sample_period_ms = hub.resource_policy().await.sample_period_ms;
        // Idle Browser Use does not need to scan the operating system's full
        // process table every five seconds. Inventory changes wake this wait
        // immediately, so the first managed Host restores the configured
        // sampling cadence without waiting for the idle period.
        wait_for_browser_resource_sample(
            sample_period_ms,
            has_managed_hosts,
            &mut inventory_events,
        )
        .await;
        system.refresh_cpu_usage();
        let root_identities = hub.managed_host_process_identities().await;
        let cpu_usage_percent = system.global_cpu_usage();
        let next_has_managed_hosts = !root_identities.is_empty();
        if has_managed_hosts && !next_has_managed_hosts {
            // `sysinfo::System` retains its discovered process table. Once the
            // last managed Host is gone, replace the collector so Browser Use
            // does not retain unrelated OS process metadata while idle.
            system = sysinfo::System::new();
            system.refresh_cpu_usage();
        }
        has_managed_hosts = next_has_managed_hosts;
        let sample = sample_browser_resources(
            &mut system,
            &root_identities,
            Some(cpu_usage_percent),
        );
        hub.update_resource_telemetry(sample).await;
    }
}

#[cfg(feature = "browser-use")]
use crate::browser_inventory_events::{BROWSER_INVENTORY_EVENT_NAME, browser_inventory_resync_event};

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
    /// Installation-scoped Remote front-door token store (SHA-256 hash only).
    pub instance_token_repo: Arc<dyn IInstanceTokenRepository>,
    /// In-memory validator for the single installation token.
    pub instance_token_validator: Arc<InstanceTokenValidator>,
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
    /// Task-scoped transport and health authority for provider models.
    pub provider_model_capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
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
    /// The one SSH connection pool: every live remote session in the process.
    /// Shared (not rebuilt) by the host-book routes, the agent factory and the
    /// conversation-delete cascade — a per-consumer pool would report status about
    /// sockets the agent is not using. `clone()` is a handle to the same pool.
    pub ssh_pool: nomifun_ssh::SshConnectionPool,
    /// LAN robot gateway: device registry, live status, tool registry, loopback
    /// MCP front and the speech stack. `None` when the registry could not be
    /// loaded — every robot entry point is then simply absent, which is a better
    /// failure than refusing to boot the desktop over a robot file. The accept
    /// loop is attached during router assembly, where the `ConversationService`
    /// the sessions dispatch through exists.
    pub robot: Option<Arc<crate::robot_wiring::RobotServices>>,
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
    /// Singleton computer-history service (privacy-filtered activity
    /// observation). `None` when the feature-local store failed to open this
    /// boot — every `computer_history_*` surface then reports unavailable
    /// instead of refusing the boot. The observer loop is only running while
    /// the user has explicitly enabled capture; its cancellation token is
    /// cancelled in the shutdown sequence so the open segment flushes.
    pub computer_history: Option<Arc<nomifun_computer_history::ComputerHistoryService>>,
    /// 客服独立域 CRUD service (agents / notes / bindings).
    pub customer_service_service: Arc<nomifun_customer_service::CustomerServiceService>,
    /// 客服无状态并发回合执行器 (channel seam target).
    pub cs_dialogue_engine: Arc<nomifun_customer_service::CsDialogueEngine>,
    /// Singleton Creative Studio service — canonical projects, assets,
    /// templates, and archives. Asset binaries live under
    /// `{data_dir}/workshop/`; project documents live in SQLite. Shared by the
    /// `/api/creative-studio/*` routes and Gateway capabilities.
    pub workshop_service: Arc<nomifun_workshop::WorkshopService>,
    /// Singleton 小程序 (mini-app) service — owner-scoped CRUD over the
    /// `miniapps` table, the document read the auth-exempt serve route uses, and
    /// the per-app working copy under `{work_dir}/miniapps/{miniapp_id}/`.
    /// Shared so the serve route and the publish route cannot disagree about
    /// which document a given id names.
    pub miniapp_service: Arc<nomifun_miniapp::MiniAppService>,
    /// Singleton generation service — the Creative Studio media task queue.
    /// Shared by the `/api/creative-studio/tasks*` routes and Gateway tools.
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
    /// semantics for the actual Gateway and Hub owners.
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
    /// Held only for RAII retention: keeps the cleanup authority (and the
    /// resources it guards) alive while this error propagates.
    _authority: Arc<StartupCleanupAuthority>,
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
            _authority: authority,
        }
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
        if let Err(error) = self
            .knowledge_service
            .recover_pending_tree_operations()
            .await
        {
            return Err(RetainedAppServicesStartupError::new(
                self,
                anyhow::anyhow!(
                    "knowledge-tree mutation-journal boot reconciliation failed: {error}"
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

    /// Stop the computer-history observer (if running) so the open activity
    /// segment is flushed before the feature-local SQLite file closes with the
    /// database. Idempotent; a service that failed to open this boot is a no-op.
    pub async fn shutdown_computer_history(&self) {
        if let Some(service) = &self.computer_history {
            if let Err(error) = service.stop().await {
                tracing::warn!(%error, "computer history shutdown failed");
            }
        }
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

        // Subscribe before publishing the Hub to any provider or MCP entry
        // point, so an immediate first Host launch cannot race past the idle
        // telemetry wake-up channel or the realtime inventory forwarder.
        let telemetry_events = hub.subscribe();
        let realtime_events = hub.subscribe();
        let platform_tasks_shutdown = BrowserPlatformTaskShutdown::default();
        let shutdown_coordinator = BrowserShutdownCoordinator::with_platform_tasks(
            Arc::clone(&hub),
            platform_tasks_shutdown.clone(),
        );
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

        let sweep_hub = Arc::clone(&hub);
        let sweep = spawn_supervised_browser_platform_loop(
            "lifecycle_sweep",
            platform_tasks_shutdown.clone(),
            move || {
                let hub = Arc::clone(&sweep_hub);
                async move { run_browser_lifecycle_sweep_loop(hub).await }
            },
        );

        let events_hub = Arc::clone(&hub);
        let event_bus = self.event_bus.clone();
        let ws_manager = self.ws_manager.clone();
        let installation_owner = self.authoritative_user_id.clone();
        let mut first_realtime_events = Some(realtime_events);
        let events = spawn_supervised_browser_platform_loop(
            "inventory_events",
            platform_tasks_shutdown.clone(),
            move || {
                let receiver = first_realtime_events
                    .take()
                    .unwrap_or_else(|| events_hub.subscribe());
                let event_bus = Arc::clone(&event_bus);
                let ws_manager = Arc::clone(&ws_manager);
                let installation_owner = Arc::clone(&installation_owner);
                async move {
                    forward_browser_inventory_events(
                        receiver,
                        event_bus,
                        ws_manager,
                        installation_owner,
                    )
                    .await
                }
            },
        );

        let telemetry_hub = Arc::clone(&hub);
        let mut first_telemetry_events = Some(telemetry_events);
        let telemetry = spawn_supervised_browser_platform_loop(
            "resource_telemetry",
            platform_tasks_shutdown.clone(),
            move || {
                // Collector and receiver are attempt-local. A panic cannot
                // poison or retain sysinfo's process table; the replacement
                // loop starts from a clean CPU baseline and a fresh receiver.
                let system = sysinfo::System::new();
                let inventory_events = first_telemetry_events
                    .take()
                    .unwrap_or_else(|| telemetry_hub.subscribe());
                let hub = Arc::clone(&telemetry_hub);
                async move {
                    run_browser_resource_telemetry_loop(hub, system, inventory_events).await
                }
            },
        );

        self._browser_platform_tasks = Some(BrowserPlatformTasks {
            shutdown: platform_tasks_shutdown,
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

        // The Remote front door belongs to this installation. It has one token
        // hash and never resolves a token to a companion identity.
        let instance_token_repo: Arc<dyn IInstanceTokenRepository> =
            Arc::new(SqliteInstanceTokenRepository::new(database.pool().clone()));
        let initial_token = instance_token_repo.get().await.unwrap_or_else(|error| {
            tracing::warn!(
                "failed to load the installation access token at boot (Remote front door stays closed until a token is minted): {error}"
            );
            None
        });
        let instance_token_validator = Arc::new(InstanceTokenValidator::new(initial_token));

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

        let provider_repo = Arc::new(SqliteProviderRepository::new(database.pool().clone()));
        let provider_model_repo: Arc<dyn IProviderModelRepository> =
            Arc::new(SqliteProviderModelRepository::new(database.pool().clone()));
        let provider_model_capability_repo: Arc<dyn IProviderModelCapabilityRepository> = Arc::new(
            SqliteProviderModelCapabilityRepository::new(database.pool().clone()),
        );
        let provider_connection_repo: Arc<dyn nomifun_db::IProviderConnectionRepository> =
            Arc::new(nomifun_db::SqliteProviderConnectionRepository::new(
                database.pool().clone(),
            ));
        let model_invoke_http = nomifun_net::http_client();
        let model_invoke_service = Arc::new(nomifun_model_invoke::ModelInvokeService::new(
            provider_repo.clone(),
            provider_model_repo.clone(),
            provider_model_capability_repo.clone(),
            provider_connection_repo,
            encryption_key,
            model_invoke_http.clone(),
            nomifun_model_invoke::AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
        ));
        // Start the stable managed-model loopback supply and provision its
        // provider/model capability graph before agent factory construction.
        // A seed catalog makes a fresh install usable without blocking boot on
        // third-party discovery.
        let (managed_model_service, managed_model_server) =
            nomifun_system::start_and_provision_free_model_with_preferences(
                provider_repo.clone(),
                provider_model_repo.clone(),
                provider_model_capability_repo.clone(),
                Some(Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                    database.pool().clone(),
                ))),
                encryption_key,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to provision NomiFun free model service: {e}"))?;
        // Refresh immediately, then about every six hours with jitter. Failed
        // attempts retain the current catalog and use capped exponential
        // backoff. ManagedModelService owns the single transactional graph
        // write; there is deliberately no second profile/backfill writer.
        let managed_model_refresh_task =
            nomifun_system::ManagedModelRefreshTask::start(managed_model_service.clone());
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
        // 1024: shared by every domain incl. per-chunk terminal output; lag
        // now also emits sync.resync-required, but a larger buffer keeps
        // drops rare in the first place.
        let event_bus = Arc::new(BroadcastEventBus::new(1024));

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

        // Singleton knowledge service: knowledge base registry + workspace
        // mounting. Shared by the `/api/knowledge/*` routes and the
        // conversation service (mount-at-task-start).
        let sqlite_knowledge_repo = Arc::new(
            nomifun_db::SqliteKnowledgeRepository::new(database.pool().clone()),
        );
        let knowledge_repo: Arc<dyn nomifun_db::IKnowledgeRepository> =
            sqlite_knowledge_repo.clone();
        let knowledge_service = Arc::new(nomifun_knowledge::KnowledgeService::new(
            knowledge_repo,
            &data_dir,
            nomifun_knowledge::KnowledgeEventEmitter::new(
                event_bus.clone(),
                authoritative_user_id.clone(),
            ),
        ));
        knowledge_service.set_entry_repository(
            sqlite_knowledge_repo.clone() as Arc<dyn nomifun_db::IKnowledgeEntryRepository>,
        );
        knowledge_service.set_source_repository(
            sqlite_knowledge_repo as Arc<dyn nomifun_db::IKnowledgeSourceRepository>,
        );
        knowledge_service.set_tree_operation_repository(Arc::new(
            nomifun_db::SqliteKnowledgeTreeOperationRepository::new(
                database.pool().clone(),
            ),
        ));
        knowledge_service.set_retrieval_runtime(
            Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                database.pool().clone(),
            )),
            model_invoke_service.clone(),
        );
        // Late-wire the LLM seam for knowledge autogen / snapshot compression
        // (`LiveKnowledgeCompleter` resolves the first enabled provider/model
        // per call, so it tolerates providers configured after boot).
        knowledge_service.set_completer(Arc::new(nomifun_ai_agent::LiveKnowledgeCompleter {
            provider_repo: provider_repo.clone() as Arc<dyn nomifun_db::IProviderRepository>,
            provider_model_repo: provider_model_repo.clone(),
            model_invoke: model_invoke_service.clone(),
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
            model_invoke: model_invoke_service.clone(),
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
                model_invoke: model_invoke_service.clone(),
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

        // Computer history (privacy-filtered activity observation). The
        // feature ships DISABLED — the service is always assembled so the
        // agent sink + gateway capabilities exist, but nothing is sampled
        // until the user explicitly enables it (set_enabled → start). A
        // domain-local failure (disk, sqlite) must not refuse the boot: the
        // handle goes `None` and every computer-history surface then reports
        // itself unavailable, the same degradation the robot gateway uses.
        let computer_history = match nomifun_computer_history::ComputerHistoryService::open(
            nomifun_computer_history::ComputerHistoryConfig {
                data_dir: data_dir.join("computer-history"),
                ..Default::default()
            },
            nomifun_computer_history::observer::default_backend(),
            authoritative_user_id.to_string(),
        )
        .await
        {
            Ok(service) => Some(Arc::new(service)),
            Err(error) => {
                tracing::error!(%error, "computer history service unavailable this boot");
                None
            }
        };

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
                    model_invoke: model_invoke_service.clone(),
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
        // there), and reads/writes canvas assets through the workshop bridge
        // (AssetSource/AssetSink — no crate cycle). Untrusted provider-returned
        // artifact URLs use the creation engine's proxy-free, DNS-pinned safe
        // downloader. `reconcile_on_boot` (running-with-remote resume / else
        // fail-interrupted) is driven from `build_creation_state` at router
        // assembly.
        // Unified multimodal invoke layer (P1): one process-wide singleton over
        // the catalog repos + the same proxy-aware HTTP client. The creation
        // engine and `/api/tts` consume it; later tasks (health probes) reuse
        // this exact instance.
        let creation_asset_bridge = Arc::new(crate::workshop_bridge::WorkshopAssetBridge::new(
            data_dir.clone(),
            Arc::new(nomifun_db::SqliteWorkshopRepository::new(database.pool().clone())),
        ));
        let creation_service = nomifun_creation::CreationService::builder(Arc::new(
            nomifun_db::SqliteCreationTaskRepository::new(database.pool().clone()),
        ))
        .with_invoke(model_invoke_service.clone())
        .with_text_executor(Arc::new(AgentCreationTextExecutor {
            model_invoke: model_invoke_service.clone(),
            workspace: data_dir.clone(),
        }))
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

        // Headless seed for the installation-scoped Remote front door. It is
        // independent of companion creation and model/profile configuration.
        if let Ok(seed) = std::env::var("NOMIFUN_ACCESS_TOKEN") {
            let seed = seed.trim();
            if !seed.is_empty() && !instance_token_validator.validate(seed) {
                let hash = nomifun_auth::token_sha256_hex(seed);
                if let Err(error) = instance_token_repo.set(&hash).await {
                    tracing::warn!("failed to persist NOMIFUN_ACCESS_TOKEN seed: {error}");
                } else {
                    instance_token_validator.set_token(hash);
                    tracing::info!(
                        "Remote access token seeded from NOMIFUN_ACCESS_TOKEN for this NomiFun Desktop installation"
                    );
                }
            }
        }

        // Expose the provider repo on AppServices (mint-time model guard reads it)
        // before it is moved into the agent factory below.
        let provider_repo_for_services: Arc<dyn IProviderRepository> =
            provider_repo.clone() as Arc<dyn nomifun_db::IProviderRepository>;

        #[cfg(feature = "browser-use")]
        let browser_lane_provider_slot =
            nomifun_ai_agent::BrowserLaneClientProviderSlot::new();

        // 小程序 (mini-apps): the published snapshot lives in SQLite (that is what
        // the serve route streams into an iframe), while each app's working copy
        // lives on disk under `{work_dir}/miniapps/{miniapp_id}/`. `work_dir` is
        // the SAME resolved value `ConversationService` uses as its workspace
        // root — passed in, never re-resolved, or the absolute path this service
        // hands 「继续迭代」 and the directory it materializes into could name
        // different places. No background task: the working copy is materialized
        // lazily on first provision/publish, so a user who never iterates pays
        // nothing.
        let miniapp_service = Arc::new(nomifun_miniapp::MiniAppService::new(
            work_dir.clone(),
            Arc::new(nomifun_db::SqliteMiniAppRepository::new(
                database.pool().clone(),
            )),
        ));

        // SSH remote sessions: ONE process-level connection pool, built here
        // because the agent factory below is its first consumer and the host-book
        // routes plus the conversation-delete cascade must receive this very
        // handle. A second pool would supervise sockets nobody is talking to while
        // reporting status for the ones the operator can see. Host keys are learned
        // into the operator's own ~/.ssh/known_hosts.
        let ssh_pool = {
            let repo = Arc::new(nomifun_db::SqliteSshHostRepository::new(
                database.pool().clone(),
            )) as Arc<dyn nomifun_db::ISshHostRepository>;
            let known_hosts = dirs::home_dir()
                .unwrap_or_else(|| data_dir.clone())
                .join(".ssh")
                .join("known_hosts");
            nomifun_ssh::SshConnectionPool::new(
                nomifun_ssh::SshHostService::new(repo, encryption_key),
                known_hosts,
                nomifun_ssh::SshEventEmitter::new(event_bus.clone()),
            )
        };

        // LAN robot gateway. Everything that does not need a
        // `ConversationService` is built here so the device face and the OTA
        // response are live the moment a listener comes up; the accept loop is
        // attached during router assembly. A failure is domain-local: the
        // desktop boots without robot support rather than not at all.
        let robot = match crate::robot_wiring::RobotServices::build(
            &data_dir,
            authoritative_user_id.as_ref(),
            event_bus.clone(),
            model_invoke_service.clone(),
            companion_service.clone(),
            Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(
                database.pool().clone(),
            )),
        )
        .await
        {
            Ok(services) => Some(Arc::new(services)),
            Err(error) => {
                tracing::error!(%error, "robot: gateway unavailable this boot");
                None
            }
        };

        let factory = build_agent_factory(AgentFactoryDeps {
            authoritative_user_id: authoritative_user_id.clone(),
            model_invoke: model_invoke_service.clone(),
            model_invoke_service: Some(model_invoke_service.clone()),
            encryption_key,
            data_dir: data_dir.clone(),
            work_dir: work_dir.clone(),
            gateway_mcp_config: gateway_mcp_config.clone(),
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
            // Computer-history native tools (`computer_history_*`). The sink
            // wraps the SAME service the gateway capabilities use, so a
            // `computer_history_status` call and the settings page can never
            // disagree about recorder state. `None` (service failed to open)
            // keeps every tool unregistered.
            computer_history_sink: computer_history.clone().map(|service| {
                Arc::new(nomifun_ai_agent::computer_history_sink::LiveComputerHistorySink {
                    service,
                }) as Arc<dyn nomifun_ai_agent::ComputerHistorySink>
            }),
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
            // SSH remote sessions: the factory dials through the one process pool,
            // so a runtime rebuilt by a model switch rejoins the conversation's
            // existing link instead of opening (and abandoning) a second one.
            ssh_provider: Some(Arc::new(ssh_pool.clone())
                as Arc<dyn nomifun_ai_agent::SshBackendProvider>),
        });

        // Agent factory is now wired. Future extension/custom agents
        // that get written to `agent_metadata` will show up after the
        // relevant service calls `AgentRegistry::hydrate`.
        let runtime_registry_concrete = Arc::new(
            InMemoryAgentRuntimeRegistry::new(factory)
                .with_model_config_resolver(build_agent_model_config_resolver(
                    model_invoke_service.clone(),
                ))
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
            instance_token_repo,
            instance_token_validator,
            provider_repo: provider_repo_for_services,
            managed_model_service,
            _managed_model_server: managed_model_server,
            _managed_model_refresh_task: managed_model_refresh_task,
            provider_model_repo: provider_model_repo.clone(),
            provider_model_capability_repo: provider_model_capability_repo.clone(),
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
            ssh_pool,
            robot,
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
            computer_history,
            customer_service_service,
            cs_dialogue_engine,
            workshop_service,
            miniapp_service,
            creation_service,
            model_invoke_service,
            knowledge_service,
            #[cfg(feature = "browser-use")]
            browser_session_hub: None,
            browser_platform_shutdown,
            #[cfg(feature = "browser-use")]
            _browser_lane_provider_slot: browser_lane_provider_slot,
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
            .map(nomi_browser_engine::StorageState::into_cookie_only)
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
            let persistent_login_key = services.encryption_key;
            let factory = nomi_browser::ManagedEngineHostFactory::new(engine_config)
                .with_identity_vault(
                    nomi_browser_engine::shared_storage_state_path(&services.data_dir),
                    services.encryption_key,
                )
                .with_lane_policy(Arc::new(move |tool| {
                    tool.persistent_login_key(persistent_login_key)
                }));
            let mut hub_config = nomifun_browser_platform::HubConfig {
                // Hub applies this preference only to Primary identity when
                // constructing HostLaunchRequest; Anonymous/Replica/Isolated
                // Hosts remain headless even in external display mode.
                headful: primary_host_is_headful(display_mode),
                // The policy is the separate axis deciding whether the Hub may
                // resolve visibility per Lane at all. Without this the Hub would
                // always see the `Auto` default, so a user who explicitly pinned
                // `headless` would still get a window at an attended moment —
                // exactly the override the design promises never happens.
                visibility_policy: browser_visibility_policy(display_mode),
                ..Default::default()
            };
            // Derive installation-wide throughput from this machine before
            // constructing the Hub. Resource telemetry updates pressure and
            // RSS decisions, but deliberately do not rewrite scheduler limits;
            // leaving HubConfig::default() here would therefore pin every
            // machine to the fallback 8-GiB/4-CPU capacity for the lifetime of
            // the process. Per-task limits remain preset constants and are not
            // expanded by a larger machine or HighConcurrency.
            let mut startup_system = sysinfo::System::new();
            startup_system.refresh_memory();
            let startup_total_memory_bytes = startup_system.total_memory();
            let startup_available_memory_bytes = startup_system.available_memory();
            let startup_logical_cpus = std::thread::available_parallelism()
                .ok()
                .map(std::num::NonZeroUsize::get);
            hub_config.resource_policy = browser_startup_resource_policy(
                startup_total_memory_bytes,
                startup_logical_cpus,
            );
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
            // Seed memory pressure synchronously before the Hub is published
            // to any provider or MCP entry point. The periodic sampler starts
            // later, so relying on its spawned first tick would leave a short
            // admission window in which a heavily pressured machine still
            // appeared healthy.
            // No managed Host exists yet, so avoid an unnecessary full-system
            // process scan on the startup critical path. Host RSS joins begin
            // with the periodic sampler after Host authority is available.
            let initial_resource_sample = browser_resource_telemetry_from_measurements(
                startup_total_memory_bytes,
                startup_available_memory_bytes,
                startup_logical_cpus,
                None,
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                None,
            );
            hub.update_resource_telemetry(initial_resource_sample).await;
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
        BrowserProfileFootprint,
        BrowserIdentityMode, BrowserLaneDriver, BrowserOperation,
        BrowserOperationResult, BrowserPlatformError, DriverOperationContext, HostLaunchRequest,
        HostLifecycleState, HubConfig, LaneFreezeOutcome, LaneLaunchRequest,
        SnapshotComponentCoverage,
    };
    #[cfg(feature = "browser-use")]
    use tokio::sync::{Notify, Semaphore};

    #[cfg(feature = "browser-use")]
    struct ActiveBrowserLoopGuard {
        active: Arc<AtomicUsize>,
    }

    #[cfg(feature = "browser-use")]
    impl ActiveBrowserLoopGuard {
        fn enter(active: Arc<AtomicUsize>, maximum: &AtomicUsize) -> Self {
            let live = active.fetch_add(1, Ordering::AcqRel) + 1;
            maximum.fetch_max(live, Ordering::AcqRel);
            Self { active }
        }
    }

    #[cfg(feature = "browser-use")]
    impl Drop for ActiveBrowserLoopGuard {
        fn drop(&mut self) {
            self.active.fetch_sub(1, Ordering::AcqRel);
        }
    }

    #[cfg(feature = "browser-use")]
    fn spawn_tracked_pending_browser_loop(
        loop_name: &'static str,
        shutdown: BrowserPlatformTaskShutdown,
        active: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
    ) -> tokio::task::JoinHandle<()> {
        spawn_supervised_browser_platform_loop(loop_name, shutdown, move || {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            async move {
                let _guard = ActiveBrowserLoopGuard::enter(active, &maximum);
                std::future::pending::<()>().await;
            }
        })
    }

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

        // This fake manages no on-disk profile, so report a completed
        // zero measurement. Inheriting the trait default would instead
        // mean "could not measure", which fences Primary fail-closed.
        async fn profile_footprint(
            &self,
            _stop_after_bytes: u64,
            _stop_after_entries: u64,
        ) -> Result<Option<BrowserProfileFootprint>, BrowserPlatformError> {
            Ok(Some(BrowserProfileFootprint::EMPTY))
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
        // A stored value under the current v3 marker is authoritative.
        assert_eq!(
            resolve_browser_display_mode(Some("headless"), Some("3")),
            ("headless", false)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("external"), Some("\"3\"")),
            ("external", false)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("auto"), Some("3")),
            ("auto", false)
        );
        // A version-2 marker proves an explicit choice: an external opt-in is
        // preserved, while v2's universal `headless` default adopts `auto`.
        assert_eq!(
            resolve_browser_display_mode(Some("external"), Some("\"2\"")),
            ("external", true)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("headless"), Some("2")),
            ("auto", true)
        );
        // Every unversioned historical value converges once on `auto`, including
        // the previous *inferred* external setting, which was never a real user
        // choice and must not resurrect an operating-system window.
        assert_eq!(
            resolve_browser_display_mode(Some("external"), None),
            ("auto", true)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("external"), Some("1")),
            ("auto", true)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("headless"), Some("1")),
            ("auto", true)
        );
        assert_eq!(resolve_browser_display_mode(None, None), ("auto", true));
        // Missing or invalid mode under the current marker is repaired.
        assert_eq!(
            resolve_browser_display_mode(Some("embedded"), Some("3")),
            ("auto", true)
        );
        assert_eq!(
            resolve_browser_display_mode(Some("invalid"), Some("3")),
            ("auto", true)
        );
        assert_eq!(
            resolve_browser_display_mode(None, Some("3")),
            ("auto", true),
            "a marker without a valid mode fails safe to the auto default"
        );
        assert_eq!(
            resolve_browser_display_mode(Some("  \"headless\"  "), Some("  \"3\"  ")),
            ("headless", false)
        );
        // Only the user's explicit external policy launches a visible Primary
        // Host. `auto` starts silent and lets the Hub surface a window later.
        assert!(primary_host_is_headful("external"));
        assert!(!primary_host_is_headful("headless"));
        assert!(!primary_host_is_headful("auto"));
        assert!(!primary_host_is_headful("embedded"));

        // The policy is the separate axis: it decides whether the Hub may resolve
        // visibility per Lane at all. A pinned choice must forbid that.
        use nomifun_browser_platform::BrowserVisibilityPolicy;
        assert_eq!(
            browser_visibility_policy("external"),
            BrowserVisibilityPolicy::AlwaysHeadful
        );
        assert_eq!(
            browser_visibility_policy("headless"),
            BrowserVisibilityPolicy::AlwaysHeadless
        );
        assert_eq!(
            browser_visibility_policy("auto"),
            BrowserVisibilityPolicy::Auto
        );
        assert_eq!(
            browser_visibility_policy("embedded"),
            BrowserVisibilityPolicy::Auto,
            "unrecognized state fails closed to auto, which still launches silently"
        );
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
    async fn browser_display_mode_migrates_unversioned_external_to_auto_once() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"external\""),
            ("agent.browserUse.source", "\"system\""),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(
            preferences.display_mode, "auto",
            "unversioned external state was never an explicit user choice, so it \
             must not keep opening an operating-system window"
        );
        assert_eq!(
            repo.writes(),
            vec![
                (
                    BROWSER_DISPLAY_MODE_PREF_KEY.to_owned(),
                    "\"auto\"".to_owned()
                ),
                (
                    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY.to_owned(),
                    BROWSER_DISPLAY_MODE_POLICY_VERSION.to_owned()
                ),
            ]
        );
    }

    /// A user who explicitly opted into a visible window under version 2 keeps
    /// it; the new `auto` default must not silently take it away.
    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_display_mode_preserves_version_two_explicit_external() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"external\""),
            (
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION,
            ),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "external");
        assert_eq!(
            repo.writes(),
            vec![
                (
                    BROWSER_DISPLAY_MODE_PREF_KEY.to_owned(),
                    "\"external\"".to_owned()
                ),
                (
                    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY.to_owned(),
                    BROWSER_DISPLAY_MODE_POLICY_VERSION.to_owned()
                ),
            ],
            "the preserved choice is restamped with the current lineage marker"
        );
    }

    /// Version 2's `headless` was the default for every installation rather than
    /// a deliberate "never show me a window", so it adopts the new `auto`
    /// default.
    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_display_mode_migrates_version_two_headless_default_to_auto() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"headless\""),
            (
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION,
            ),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "auto");
    }

    /// An explicit `headless` under the *current* lineage is a real "never show
    /// me a window" choice and is preserved verbatim.
    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_display_mode_preserves_current_explicit_headless() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"headless\""),
            (
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                BROWSER_DISPLAY_MODE_POLICY_VERSION,
            ),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "headless");
        assert!(repo.writes().is_empty());
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn fresh_install_persists_auto_display_mode() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "auto");
        assert_eq!(
            repo.writes(),
            vec![
                (
                    BROWSER_DISPLAY_MODE_PREF_KEY.to_owned(),
                    "\"auto\"".to_owned()
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
    async fn invalid_display_mode_is_repaired_to_auto() {
        let repo = BrowserPreferenceTestRepository::with_rows(&[
            (BROWSER_DISPLAY_MODE_PREF_KEY, "\"visible\""),
            (
                BROWSER_DISPLAY_MODE_VERSION_PREF_KEY,
                BROWSER_DISPLAY_MODE_POLICY_VERSION,
            ),
        ]);

        let preferences = load_browser_startup_preferences(&repo).await;
        assert_eq!(preferences.display_mode, "auto");
        assert_eq!(
            repo.writes(),
            vec![
                (
                    BROWSER_DISPLAY_MODE_PREF_KEY.to_owned(),
                    "\"auto\"".to_owned()
                ),
                (
                    BROWSER_DISPLAY_MODE_VERSION_PREF_KEY.to_owned(),
                    BROWSER_DISPLAY_MODE_POLICY_VERSION.to_owned()
                ),
            ],
            "malformed configuration must converge on the auto default, which \
             still launches silently"
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
            assert_eq!(
                event.data["change_kind"],
                crate::browser_inventory_events::BROWSER_INVENTORY_RESYNC_CHANGE_KIND
            );
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
    async fn ordered_hub_shutdown_cancels_platform_loops_before_waiting_for_host_cleanup() {
        let (_unbound_coordinator, probe, hub) = shutdown_coordinator_fixture().await;
        probe.block_shutdown.store(true, Ordering::Release);
        let platform_tasks_shutdown = BrowserPlatformTaskShutdown::default();
        let coordinator = BrowserShutdownCoordinator::with_platform_tasks(
            hub,
            platform_tasks_shutdown.clone(),
        );

        let shutdown = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.shutdown().await })
        };
        tokio::time::timeout(
            Duration::from_secs(1),
            platform_tasks_shutdown.cancelled(),
        )
        .await
        .expect("ordered Hub shutdown must stop loop supervisors first");
        probe.wait_for_shutdown_calls(1).await;
        assert!(
            !shutdown.is_finished(),
            "the loop cancellation boundary must not depend on Host cleanup completing"
        );

        probe.shutdown_release.add_permits(1);
        assert!(shutdown.await.unwrap().is_ok());
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
        let shutdown = BrowserPlatformShutdown::from_steps(Some(gateway.step()));

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(gateway.calls.load(Ordering::Acquire), 1);

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

        assert!(shutdown.shutdown().await.is_ok());
        assert_eq!(hub_probe.shutdown_calls.load(Ordering::Acquire), 1);
        assert_eq!(gateway.calls.load(Ordering::Acquire), 2);
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn unsafe_orphan_recovery_degrades_browser_functionality() {
        let safe = nomi_browser_engine::profile::ProfileRecoveryReport::default();
        assert!(BrowserOrphanRecoveryOutcome::from_report(&safe).permits_host_composition());

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
    fn browser_platform_loop_restart_backoff_is_exponential_and_capped() {
        let initial = Duration::from_millis(100);
        let maximum = Duration::from_secs(5);

        assert_eq!(
            browser_platform_loop_restart_delay(0, initial, maximum),
            Duration::from_millis(100)
        );
        assert_eq!(
            browser_platform_loop_restart_delay(1, initial, maximum),
            Duration::from_millis(200)
        );
        assert_eq!(
            browser_platform_loop_restart_delay(5, initial, maximum),
            Duration::from_millis(3_200)
        );
        assert_eq!(
            browser_platform_loop_restart_delay(6, initial, maximum),
            maximum
        );
        assert_eq!(
            browser_platform_loop_restart_delay(u32::MAX, initial, maximum),
            maximum
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_shutdown_signal_has_no_cancel_wait_race() {
        // Alternate cancel-before-subscribe with concurrent cancellation after
        // waiter creation. Repetition makes the old AtomicBool + notify_waiters
        // lost-wakeup window deterministic enough to catch if reintroduced;
        // watch retains the terminal value for every later subscriber.
        for round in 0..128 {
            let shutdown = BrowserPlatformTaskShutdown::default();
            if round % 2 == 0 {
                shutdown.cancel();
            }
            let mut waiters = tokio::task::JoinSet::new();
            for _ in 0..8 {
                let shutdown = shutdown.clone();
                waiters.spawn(async move { shutdown.cancelled().await });
            }
            if round % 2 != 0 {
                let mut cancellers = tokio::task::JoinSet::new();
                for _ in 0..8 {
                    let shutdown = shutdown.clone();
                    cancellers.spawn(async move { shutdown.cancel() });
                }
                while let Some(result) = cancellers.join_next().await {
                    result.expect("concurrent cancellation must not panic");
                }
            }
            tokio::time::timeout(Duration::from_secs(1), async {
                while let Some(result) = waiters.join_next().await {
                    result.expect("shutdown waiter must not panic");
                }
            })
            .await
            .expect("every pre-existing and future shutdown waiter must wake");
            assert!(shutdown.is_cancelled());
        }
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_platform_supervisor_restarts_without_overlapping_loop_instances() {
        let shutdown = BrowserPlatformTaskShutdown::default();
        let attempts = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));

        let attempts_for_loop = Arc::clone(&attempts);
        let active_for_loop = Arc::clone(&active);
        let maximum_for_loop = Arc::clone(&maximum);
        let supervisor_shutdown = shutdown.clone();
        let supervisor = tokio::spawn(supervise_browser_platform_loop(
            "test_restart",
            supervisor_shutdown,
            move || {
                let attempt = attempts_for_loop.fetch_add(1, Ordering::AcqRel);
                let active = Arc::clone(&active_for_loop);
                let maximum = Arc::clone(&maximum_for_loop);
                async move {
                    let _guard = ActiveBrowserLoopGuard::enter(active, &maximum);
                    match attempt {
                        0 => {}
                        1 => panic!("injected supervised-loop panic"),
                        _ => std::future::pending::<()>().await,
                    }
                }
            },
            Duration::ZERO,
            Duration::ZERO,
        ));

        tokio::time::timeout(Duration::from_secs(1), async {
            while attempts.load(Ordering::Acquire) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the supervisor must rebuild the loop after return and panic");
        assert_eq!(maximum.load(Ordering::Acquire), 1);
        assert_eq!(active.load(Ordering::Acquire), 1);

        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(1), supervisor)
            .await
            .expect("cancellation must stop the supervisor")
            .expect("the supervisor task must not panic");
        assert_eq!(active.load(Ordering::Acquire), 0);
        let settled_attempts = attempts.load(Ordering::Acquire);
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            attempts.load(Ordering::Acquire),
            settled_attempts,
            "shutdown must not start another loop attempt"
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn dropping_browser_platform_tasks_cancels_every_inline_loop() {
        let shutdown = BrowserPlatformTaskShutdown::default();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let tasks = BrowserPlatformTasks {
            shutdown: shutdown.clone(),
            sweep: spawn_tracked_pending_browser_loop(
                "drop_sweep",
                shutdown.clone(),
                Arc::clone(&active),
                Arc::clone(&maximum),
            ),
            events: spawn_tracked_pending_browser_loop(
                "drop_events",
                shutdown.clone(),
                Arc::clone(&active),
                Arc::clone(&maximum),
            ),
            telemetry: spawn_tracked_pending_browser_loop(
                "drop_telemetry",
                shutdown.clone(),
                Arc::clone(&active),
                Arc::clone(&maximum),
            ),
        };

        tokio::time::timeout(Duration::from_secs(1), async {
            while active.load(Ordering::Acquire) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all three platform loops must start");
        assert_eq!(active.load(Ordering::Acquire), 3);

        drop(tasks);
        tokio::time::timeout(Duration::from_secs(1), async {
            while active.load(Ordering::Acquire) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropping AppServices-owned handles must drop active loop futures");
        assert!(shutdown.is_cancelled());
        assert_eq!(maximum.load(Ordering::Acquire), 3);
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
            std::collections::HashMap::from([(4_242, 0.25)]),
            Some(37.5),
        );

        assert_eq!(telemetry.total_memory_bytes, 16_000);
        assert_eq!(telemetry.available_memory_bytes, 7_500);
        assert_eq!(telemetry.logical_cpus, 12);
        assert_eq!(telemetry.chromium_rss_bytes, 2_048);
        assert_eq!(telemetry.host_rss_by_process_id.get(&4_242), Some(&2_048));
        assert_eq!(
            telemetry.host_cpu_pressure_by_process_id.get(&4_242),
            Some(&0.25)
        );
        assert!((telemetry.cpu_pressure - 0.375).abs() < f64::EPSILON);
        assert_eq!(telemetry.gpu_pressure, None);

        let unknown = browser_resource_telemetry_from_measurements(
            16_000,
            7_500,
            None,
            None,
            std::collections::HashMap::new(),
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
    fn browser_startup_policy_uses_machine_capacity_without_widening_task_limits() {
        let small_machine =
            browser_startup_resource_policy(4 * 1024 * 1024 * 1024, Some(2));
        let large_machine = browser_startup_resource_policy(
            64 * 1024 * 1024 * 1024,
            Some(16),
        );

        assert_eq!(small_machine.max_active_operations, 1);
        assert_eq!(small_machine.max_open_lanes, 4);
        assert_eq!(large_machine.max_active_operations, 32);
        assert_eq!(large_machine.max_open_lanes, 128);
        assert_eq!(
            large_machine.max_task_memory_bytes,
            small_machine.max_task_memory_bytes
        );
        assert_eq!(
            large_machine.max_task_active_operations,
            small_machine.max_task_active_operations
        );
        assert_eq!(
            large_machine.max_task_open_lanes,
            small_machine.max_task_open_lanes
        );
        assert_eq!(large_machine.max_task_tabs, small_machine.max_task_tabs);
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_startup_policy_falls_back_only_without_authoritative_hardware() {
        let fallback = nomifun_browser_platform::ResourcePolicy::default();
        assert_eq!(browser_startup_resource_policy(0, Some(8)), fallback);
        assert_eq!(
            browser_startup_resource_policy(16 * 1024 * 1024 * 1024, None),
            fallback
        );
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_telemetry_skips_process_scans_without_managed_hosts() {
        assert!(!browser_telemetry_needs_process_scan(&[]));
        assert!(browser_telemetry_needs_process_scan(&[
            nomifun_browser_platform::BrowserProcessIdentity {
                process_id: 42,
                started_at_epoch_seconds: 7,
                platform_start_key: 9,
            }
        ]));
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_telemetry_uses_idle_backoff_only_without_managed_hosts() {
        let active = browser_resource_sample_period(5_000, true);
        let idle = browser_resource_sample_period(5_000, false);

        assert_eq!(active, Duration::from_secs(5));
        assert_eq!(idle, BROWSER_IDLE_TELEMETRY_PERIOD);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn browser_telemetry_inventory_event_wakes_idle_collector_immediately() {
        let (sender, mut receiver) = tokio::sync::broadcast::channel(1);
        sender
            .send(nomifun_browser_platform::BrowserInventoryEvent {
                sequence: 1,
                change_kind: "host_started".to_owned(),
                lane_id: None,
                user_id: None,
                conversation_id: None,
                at_ms: 1,
            })
            .unwrap();

        tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_browser_resource_sample(5_000, false, &mut receiver),
        )
        .await
        .expect("a Host inventory event must wake the idle collector");
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_process_tree_rss_counts_roots_and_descendants_once() {
        let (rss, hosts) = browser_process_tree_rss(
            &[
                legacy_browser_process(10),
                legacy_browser_process(20),
                legacy_browser_process(10),
            ],
            [
                (10, Some(1), 100, 100),
                (11, Some(10), 50, 101),
                (12, Some(11), 25, 102),
                (20, Some(1), 200, 100),
                (99, Some(1), 9_999, 100),
            ],
        );
        assert_eq!(rss, Some(375));
        assert_eq!(hosts.get(&10), Some(&175));
        assert_eq!(hosts.get(&20), Some(&200));
        assert_eq!(
            browser_process_tree_rss(&[legacy_browser_process(10)], [(11, Some(10), 50, 101)]),
            (Some(50), std::collections::HashMap::from([(10, 50)]))
        );
        assert_eq!(
            browser_process_tree_rss(
                &[legacy_browser_process(10)],
                [(99, Some(1), 9_999, 100)],
            ),
            (None, std::collections::HashMap::new())
        );
        assert_eq!(
            browser_process_tree_rss(&[], [(10, Some(1), 100, 100)]),
            (None, std::collections::HashMap::new())
        );
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_process_tree_rss_rejects_stale_parent_pid_reuse_edges() {
        let (rss, hosts) = browser_process_tree_rss(
            &[nomifun_browser_platform::BrowserProcessIdentity {
                process_id: 10,
                started_at_epoch_seconds: 200,
                platform_start_key: 10_200,
            }],
            [
                // Current Chromium root reused PID 10 at t=200.
                (10, Some(1), 100, 200),
                (11, Some(10), 50, 201),
                // This orphan still reports parent 10 but predates the
                // current root, so neither it nor its real child belongs to
                // the managed Chromium tree.
                (90, Some(10), 9_999, 100),
                (91, Some(90), 8_888, 101),
            ],
        );

        assert_eq!(rss, Some(150));
        assert_eq!(hosts, std::collections::HashMap::from([(10, 150)]));
    }

    #[cfg(feature = "browser-use")]
    fn legacy_browser_process(
        process_id: u32,
    ) -> nomifun_browser_platform::BrowserProcessIdentity {
        nomifun_browser_platform::BrowserProcessIdentity {
            process_id,
            started_at_epoch_seconds: 0,
            platform_start_key: 0,
        }
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_process_tree_rss_rejects_reused_root_pid() {
        let expected = nomifun_browser_platform::BrowserProcessIdentity {
            process_id: 10,
            started_at_epoch_seconds: 100,
            platform_start_key: 10_100,
        };
        assert_eq!(
            browser_process_tree_rss(&[expected], [(10, Some(1), 500, 101)]),
            (None, std::collections::HashMap::new())
        );
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_process_tree_cpu_pressure_normalizes_managed_trees_only() {
        let hosts = browser_process_tree_cpu_pressure(
            &[
                legacy_browser_process(10),
                legacy_browser_process(20),
                legacy_browser_process(10),
            ],
            4,
            [
                (10, Some(1), 100.0, 100),
                (11, Some(10), 50.0, 101),
                (12, Some(11), 50.0, 102),
                (20, Some(1), 100.0, 100),
                (21, Some(20), f32::NAN, 101),
                (22, Some(20), -50.0, 101),
                (99, Some(1), 900.0, 100),
            ],
        );

        assert_eq!(hosts.get(&10), Some(&0.5));
        assert_eq!(hosts.get(&20), Some(&0.25));
        assert!(!hosts.contains_key(&99));
        assert!(browser_process_tree_cpu_pressure(
            &[legacy_browser_process(10)],
            0,
            [(10, Some(1), 100.0, 100)]
        )
        .is_empty());
    }

    #[cfg(feature = "browser-use")]
    #[test]
    fn browser_process_tree_cpu_pressure_rejects_pid_reuse() {
        let expected = nomifun_browser_platform::BrowserProcessIdentity {
            process_id: 10,
            started_at_epoch_seconds: 200,
            platform_start_key: 10_200,
        };
        let hosts = browser_process_tree_cpu_pressure(
            &[expected],
            4,
            [
                (10, Some(1), 100.0, 200),
                (11, Some(10), 100.0, 201),
                // The stale orphan predates the current root and must not be
                // joined to this managed Host merely through reused PID 10.
                (90, Some(10), 400.0, 100),
                (91, Some(90), 400.0, 101),
            ],
        );
        assert_eq!(hosts, std::collections::HashMap::from([(10, 0.5)]));

        let reused = nomifun_browser_platform::BrowserProcessIdentity {
            process_id: 10,
            started_at_epoch_seconds: 100,
            platform_start_key: 10_100,
        };
        assert!(browser_process_tree_cpu_pressure(
            &[reused],
            4,
            [(10, Some(1), 400.0, 101)]
        )
        .is_empty());
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

}
