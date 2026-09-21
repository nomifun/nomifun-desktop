//! Shared application services for dependency injection.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomifun_ai_agent::{
    AgentRegistry, AgentRuntimeSessions, InMemoryAgentRuntimeSessions,
    build_agent_model_config_resolver,
};
use nomifun_agent_execution::AgentExecutionLifecycle;
use nomifun_api_types::{GatewayMcpConfig, RequirementMcpConfig};
use nomifun_auth::{
    AuthPolicy, CookieConfig, InstanceTokenValidator, JwtService, QrTokenStore, resolve_jwt_secret,
};
use nomifun_db::{
    Database, IAgentMetadataRepository, IInstanceTokenRepository,
    IProviderModelCapabilityRepository,
    IProviderModelRepository, IProviderRepository,
    IUserRepository, SqliteAgentMetadataRepository,
    SqliteInstanceTokenRepository,
    SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
    SqliteProviderRepository,
    SqliteTerminalRepository, SqliteUserRepository,
};
use nomifun_realtime::{BroadcastEventBus, WebSocketManager};
use nomifun_terminal::{TerminalEventEmitter, TerminalLifecycleServer, TerminalService};
use tokio_util::sync::CancellationToken;
use tokio::task::JoinHandle;

use crate::config::{AppConfig, load_or_create_data_encryption_key};
use crate::router::remote_runtime::NomiCoreRemoteRuntimeCoordinator;

fn require_utf8_executable_path(path: &std::path::Path) -> anyhow::Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        anyhow::anyhow!(
            "backend executable path is not valid Unicode; refusing to configure child-process bridges or lifecycle hooks: {path:?}"
        )
    })
}

/// Process-lifetime task registry for background loops that retain
/// repository/service handles.
///
/// A cancellation token alone is not sufficient for shutdown proof: a task
/// may observe cancellation only after its current await completes.  The
/// registry therefore keeps every host-owned loop's join handle and drains
/// them before the database is closed.  It is intentionally small and
/// app-local; domain services retain ownership of their own internal workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackgroundTaskRegistryPhase {
    Open,
    Closing,
    Closed,
}

struct BackgroundTaskRegistryState {
    phase: BackgroundTaskRegistryPhase,
    tasks: Vec<JoinHandle<()>>,
}

pub(crate) struct BackgroundTaskRegistry {
    shutdown: CancellationToken,
    state: Mutex<BackgroundTaskRegistryState>,
    shutdown_owner: tokio::sync::Mutex<()>,
}

impl BackgroundTaskRegistry {
    fn new(shutdown: CancellationToken) -> Self {
        let phase = if shutdown.is_cancelled() {
            BackgroundTaskRegistryPhase::Closing
        } else {
            BackgroundTaskRegistryPhase::Open
        };
        Self {
            shutdown,
            state: Mutex::new(BackgroundTaskRegistryState {
                phase,
                tasks: Vec::new(),
            }),
            shutdown_owner: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) fn spawn(
        &self,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.phase != BackgroundTaskRegistryPhase::Open || self.shutdown.is_cancelled() {
            return false;
        }
        // Start and record under the same admission mutex as shutdown. There
        // is no spawned-but-unregistered window and no post-close task to reap.
        state.tasks.retain(|task| !task.is_finished());
        state.tasks.push(tokio::spawn(task));
        true
    }

    #[track_caller]
    pub(crate) fn register(&self, handle: JoinHandle<()>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.phase == BackgroundTaskRegistryPhase::Open && self.shutdown.is_cancelled() {
            // Defensive synchronization for a CancellationToken clone that was
            // cancelled outside AppServices. The host-owned path changes the
            // phase before cancelling, so ordinary register/shutdown races are
            // fully ordered by this mutex.
            state.phase = BackgroundTaskRegistryPhase::Closing;
        }

        match state.phase {
            BackgroundTaskRegistryPhase::Open => {
                // Finished handles no longer need to be retained. Reaping them
                // on every registration keeps timer/observer-heavy
                // installations from growing an unbounded vector while
                // preserving active join ownership.
                state.tasks.retain(|task| !task.is_finished());
                state.tasks.push(handle);
            }
            BackgroundTaskRegistryPhase::Closing => {
                // A task published after shutdown admission must never escape
                // the current drain. Abort it immediately, but retain its
                // JoinHandle so shutdown can prove cancellation completed.
                handle.abort();
                state.tasks.push(handle);
            }
            BackgroundTaskRegistryPhase::Closed => {
                // Closed is a hard rejection boundary. Retain the aborted
                // handle and reopen Closing so a retry cannot incorrectly
                // report quiescence without joining this invariant-violating
                // late registration.
                handle.abort();
                state.tasks.push(handle);
                state.phase = BackgroundTaskRegistryPhase::Closing;
                tracing::error!(
                    registration_source = %std::panic::Location::caller(),
                    "background task registration attempted after the registry was closed; task aborted and retained for shutdown retry"
                );
            }
        }
    }

    fn request_shutdown(&self) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.phase == BackgroundTaskRegistryPhase::Open {
                state.phase = BackgroundTaskRegistryPhase::Closing;
            }
        }
        // Publish Closing before waking tasks. Any task that races to register
        // a child after observing cancellation is therefore aborted and added
        // to the same drain.
        self.shutdown.cancel();
    }

    pub(crate) async fn shutdown(&self, timeout: Duration) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        self.request_shutdown();

        // Only one caller owns the drain. Followers share the same total
        // deadline and observe the terminal state after the active owner
        // releases this guard.
        let _shutdown_owner = match self.shutdown_owner.try_lock() {
            Ok(owner) => owner,
            Err(_) => {
                let remaining =
                    deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return vec![
                        "background task shutdown owner did not become available before the shutdown deadline"
                            .to_owned(),
                    ];
                }
                match tokio::time::timeout(remaining, self.shutdown_owner.lock()).await {
                    Ok(owner) => owner,
                    Err(_) => {
                        return vec![
                            "background task shutdown owner did not become available before the shutdown deadline"
                                .to_owned(),
                        ];
                    }
                }
            }
        };

        let mut errors = Vec::new();

        loop {
            let tasks = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match state.phase {
                    BackgroundTaskRegistryPhase::Open => {
                        // `request_shutdown` linearizes Open -> Closing before
                        // this owner is acquired.
                        state.phase = BackgroundTaskRegistryPhase::Closing;
                    }
                    BackgroundTaskRegistryPhase::Closing => {}
                    BackgroundTaskRegistryPhase::Closed if state.tasks.is_empty() => {
                        return errors;
                    }
                    BackgroundTaskRegistryPhase::Closed => {
                        // A defensive late registration retained a task after
                        // the previous drain. Re-enter Closing and join it.
                        state.phase = BackgroundTaskRegistryPhase::Closing;
                    }
                }

                if state.tasks.is_empty() {
                    // The empty check and Closed publication share the same
                    // mutex as register(). No registration can slip between
                    // them and become an untracked detached handle.
                    state.phase = BackgroundTaskRegistryPhase::Closed;
                    return errors;
                }
                std::mem::take(&mut state.tasks)
            };

            let mut tasks = tasks.into_iter();
            while let Some(mut task) = tasks.next() {
                let remaining =
                    deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    task.abort();
                    let mut retained = Vec::with_capacity(1 + tasks.len());
                    retained.push(task);
                    for task in tasks {
                        task.abort();
                        retained.push(task);
                    }
                    let retained_count = retained.len();
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.phase = BackgroundTaskRegistryPhase::Closing;
                    state.tasks.extend(retained);
                    errors.push(format!(
                        "{retained_count} background task(s) did not quiesce before the shutdown deadline; aborted handles retained for retry"
                    ));
                    return errors;
                }

                match tokio::time::timeout(remaining, &mut task).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        if !error.is_cancelled() {
                            errors.push(format!("background task join failed: {error}"));
                        }
                    }
                    Err(_) => {
                        task.abort();
                        let mut retained = Vec::with_capacity(1 + tasks.len());
                        retained.push(task);
                        for task in tasks {
                            task.abort();
                            retained.push(task);
                        }
                        let retained_count = retained.len();
                        let mut state = self
                            .state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        state.phase = BackgroundTaskRegistryPhase::Closing;
                        state.tasks.extend(retained);
                        errors.push(format!(
                            "{retained_count} background task(s) exceeded the shutdown deadline; aborted handles retained for retry"
                        ));
                        return errors;
                    }
                }
            }

            // Registrations that raced while the current batch was awaited are
            // stored in state.tasks with phase Closing. Loop until an empty
            // queue can be sealed Closed under the registration mutex.
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> (BackgroundTaskRegistryPhase, usize) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (state.phase, state.tasks.len())
    }
}

impl Drop for BackgroundTaskRegistry {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for task in &state.tasks {
            task.abort();
        }
        if !state.tasks.is_empty() {
            tracing::error!(
                retained_tasks = state.tasks.len(),
                phase = ?state.phase,
                "background task registry dropped with retained task handles"
            );
        }
    }
}

impl nomifun_conversation::BackgroundTaskRegistrar for BackgroundTaskRegistry {
    fn spawn(
        &self,
        task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>,
    ) -> bool {
        BackgroundTaskRegistry::spawn(self, task)
    }
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
        let config = match request.expected_config_revision {
            Some(revision) => {
                nomifun_ai_agent::factory::provider_config::resolve_provider_config_at_revision(
                    self.model_invoke.as_ref(),
                    &request.provider_id,
                    &request.model,
                    revision,
                    &self.workspace,
                )
                .await
            }
            None => nomifun_ai_agent::factory::provider_config::resolve_provider_config(
                self.model_invoke.as_ref(),
                &request.provider_id,
                &request.model,
                &self.workspace,
            )
            .await,
        }
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

#[derive(Default)]
struct BrowserPlatformShutdownState {
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

/// Cloneable, process-wide authority for Gateway ingress shutdown.
///
/// One shared flight waits for authoritative Gateway quiescence before the
/// database can close. A failed flight remains retryable. Native Workspace and
/// the new isolated rendering runtime own their separate cleanup barriers.
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
        let inner = Arc::clone(&self.inner);
        let active_flight = Arc::clone(&flight);
        tokio::spawn(async move {
            let result = match tokio::spawn(run_browser_platform_shutdown(
                gateway,
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
) -> BrowserShutdownResult {
    await_browser_shutdown_step(gateway)
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

pub struct AppServices {
    /// Process-owned handle to the one immutable official Runtime provider.
    pub(crate) official_runtime: Arc<crate::router::official_runtime::OfficialRuntimeHost>,
    pub database: Database,
    /// Process-lifetime cancellation shared by background domain tasks that
    /// must stop before the database is closed.
    pub(crate) background_shutdown: CancellationToken,
    pub(crate) background_tasks: Arc<BackgroundTaskRegistry>,
    /// Channel plugin manager registered during router assembly. It is
    /// stopped after ingress/queue tasks have quiesced and before SQLite
    /// closes.
    pub(crate) channel_manager:
        Mutex<Option<Arc<nomifun_channel::manager::ChannelManager>>>,
    /// Cron timer owner. Timers must be cancelled before the host joins
    /// occurrence tasks and closes SQLite.
    pub(crate) cron_service:
        Mutex<Option<Arc<nomifun_cron::service::CronService>>>,
    /// Singleton AutoWork runner. Its sweeper, boot-resume task, and active
    /// target loops must stop before the shared SQLite pool is closed.
    pub(crate) auto_work_runner:
        Mutex<Option<Arc<nomifun_requirement::AutoWorkRunner>>>,
    /// Lifecycle authority shared with the Agent Execution engine so its
    /// scheduler, planning, cleanup, and outbox tasks stop before SQLite.
    pub(crate) agent_execution_lifecycle: AgentExecutionLifecycle,
    /// Sidecar-free Nomi-core Remote task supervisor. It tracks only bounded
    /// receipt/event convergence tasks and is shut down before SQLite.
    pub(crate) nomi_core_remote_runtime: NomiCoreRemoteRuntimeCoordinator,
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
    pub agent_runtime_sessions: Arc<dyn AgentRuntimeSessions>,
    pub agent_registry: Arc<AgentRegistry>,
    /// Singleton requirement service (shares its repo + WS emitter with the
    /// nomi native-tool sink). Router assembly attaches the canonical
    /// AgentSession owner for AutoWork config persistence.
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
    /// loop is attached during router assembly, where the canonical
    /// AgentSession command/query owner exists.
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
    /// Resolved skill paths shared with the Agent compiler and Session host for
    /// immutable snapshot resolution.
    pub skill_paths: Arc<nomifun_skill_library::SkillPaths>,
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
    /// Singleton Creative Studio service — canonical projects, assets,
    /// templates, and archives. Asset binaries live under
    /// `{data_dir}/workshop/`; project documents live in SQLite. Shared by the
    /// `/api/creative-studio/*` routes and Gateway capabilities.
    pub workshop_service: Arc<nomifun_workshop::WorkshopService>,
    /// Phase M1 Plugin application facade over the clean-start, owner-scoped
    /// Product/Project/Release data root.
    pub plugin_runtime:
        Arc<nomifun_plugin_platform::runtime::PluginRuntimeApplicationService>,
    /// Singleton generation service — the Creative Studio media task queue.
    /// Shared by the `/api/creative-studio/tasks*` routes and Gateway tools.
    pub creation_service: Arc<nomifun_creation::CreationService>,
    /// Singleton unified multimodal invoke layer (P1 redesign): catalog
    /// resolution + protocol adapters over the shared proxy-aware HTTP client.
    /// Shared by `/api/tts` today; later tasks (media/probe rewiring) reuse it.
    pub model_invoke_service: Arc<nomifun_model_invoke::ModelInvokeService>,
    /// Singleton knowledge service (knowledge base platform). Shared between
    /// the `/api/knowledge/*` routes and the AgentSession runtime host, which
    /// mounts bound bases into session workspaces at task start.
    pub knowledge_service: Arc<nomifun_knowledge::KnowledgeService>,
    /// AgentSession-owned managed Browser Resources supplied by the desktop composition.
    #[cfg(feature = "browser-use")]
    pub browser_resources: Option<Arc<nomifun_browser_platform::workspace::BrowserResourceService>>,
    #[cfg(feature = "browser-use")]
    pub attached_chrome: Option<Arc<crate::AttachedChromeProviderService>>,
    #[cfg(feature = "browser-use")]
    pub local_web_search: Option<Arc<nomifun_ai_agent::local_web_search::BrowserSearchProvider>>,
    #[cfg(feature="browser-use")]
    pub headless_render: Option<Arc<crate::headless_render::HeadlessRenderRuntime>>,
    /// Ordered, cloneable Gateway/Browser shutdown authority used by every
    /// process entry point. It exists in builds without `browser-use` because
    /// the Platform Gateway is unconditional and must quiesce before the DB.
    pub(crate) browser_platform_shutdown: BrowserPlatformShutdown,
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
/// ingress handles needed for a later retry.
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
    /// semantics for the actual Gateway owner.
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
            // error. Retain the exact ingress handles and open database in
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

    /// Replace the process-local Agent runtime registry after construction.
    ///
    /// Primarily used by tests to inject mock implementations.
    pub fn with_agent_runtime_sessions(mut self, runtime_sessions: Arc<dyn AgentRuntimeSessions>) -> Self {
        self.agent_runtime_sessions = runtime_sessions;
        self
    }

    /// Explicitly stop Gateway plus the optional managed Browser platform.
    /// Teardown is asynchronous and must be confirmed before an entry point
    /// closes the database or reports startup/shutdown success; relying on
    /// `Drop` is insufficient even when `browser-use` is disabled.
    pub async fn shutdown_browser_platform(&self) -> anyhow::Result<()> {
        #[cfg(feature = "browser-use")]
        let attached_chrome = match &self.attached_chrome {
            Some(service) => service.shutdown().await,
            None => Ok(()),
        };
        #[cfg(feature="browser-use")]
        let render=match &self.headless_render {Some(runtime)=>runtime.shutdown().await,None=>Ok(())};
        let platform = self.browser_platform_shutdown.shutdown().await;
        #[cfg(feature = "browser-use")]
        if let Some(resources) = &self.browser_resources {
            if let Err(error) = resources.shutdown().await {
                return Err(anyhow::anyhow!("managed Browser Resource shutdown failed: {error}; browser platform: {platform:?}"));
            }
        }
        #[cfg(feature="browser-use")]
        render.map_err(|error|anyhow::anyhow!("headless render shutdown failed: {error}"))?;
        #[cfg(feature = "browser-use")]
        attached_chrome.map_err(|error|anyhow::anyhow!("attached Chrome Provider disconnect failed: {error}"))?;
        platform
    }

    /// Stop process-lifetime background tasks before their repositories close.
    pub(crate) fn request_background_shutdown(&self) {
        self.background_tasks.request_shutdown();
    }

    #[track_caller]
    pub(crate) fn register_background_task(&self, handle: JoinHandle<()>) {
        self.background_tasks.register(handle);
    }

    pub(crate) fn set_cron_service(
        &self,
        service: Arc<nomifun_cron::service::CronService>,
    ) {
        *self
            .cron_service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(service);
    }

    pub(crate) fn set_auto_work_runner(
        &self,
        runner: Arc<nomifun_requirement::AutoWorkRunner>,
    ) {
        *self
            .auto_work_runner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(runner);
    }

    pub(crate) async fn quiesce_auto_work_runner(&self) -> anyhow::Result<()> {
        let runner = self
            .auto_work_runner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(runner) = runner {
            runner
                .quiesce()
                .await
                .map_err(|error| anyhow::anyhow!(error))?;
        }
        Ok(())
    }

    pub(crate) fn shutdown_cron_timers(&self) {
        if let Some(service) = self
            .cron_service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            service.shutdown_timers();
        }
    }

    pub(crate) fn spawn_knowledge_resume_task(&self) {
        self.register_background_task(tokio::spawn(
            Arc::clone(&self.knowledge_service).resume_pending_source_fetches(),
        ));
    }

    pub(crate) async fn shutdown_background_tasks(&self, timeout: Duration) -> Vec<String> {
        self.background_tasks.shutdown(timeout).await
    }

    /// Ordered shutdown shared by every Nomi-core host authority.
    ///
    /// Desktop startup failures can retain a bare `AppServices` before
    /// `NomiCoreApplication` is assembled. Keeping the complete sequence here
    /// prevents that path from closing SQLite while host-owned background,
    /// terminal, channel, Agent Execution, Browser/Gateway, robot, or SSH
    /// resources are still active.
    pub(crate) async fn shutdown_nomi_core_host(&self) -> anyhow::Result<()> {
        let mut errors = Vec::new();

        self.request_background_shutdown();
        self.shutdown_cron_timers();
        // Fence runtime admission immediately, before awaiting any producer or
        // resource owner. Its owned flight runs while these owners wind down.
        let engine_shutdown = self.agent_runtime_sessions.shutdown_and_wait();
        if let Err(error) = self
            .plugin_runtime
            .shutdown_service_runtime(self.authoritative_user_id.as_ref())
            .await
        {
            errors.push(format!("Plugin Service cleanup failed: {error}"));
        }
        if let Err(error) = self.quiesce_auto_work_runner().await {
            errors.push(format!("AutoWork quiesce failed: {error:#}"));
        }
        if !self.nomi_core_remote_runtime.shutdown().await {
            errors.push(
                "Nomi-core Remote tasks remained active after the shutdown abort deadline"
                    .to_owned(),
            );
        }
        let background_errors = self
            .shutdown_background_tasks(Duration::from_secs(15))
            .await;
        if !background_errors.is_empty() {
            errors.extend(
                background_errors
                    .into_iter()
                    .map(|error| format!("background task cleanup failed: {error}")),
            );
        }
        if let Err(error) = self.shutdown_channel_manager().await {
            errors.push(format!("channel plugin cleanup failed: {error:#}"));
        }
        if let Err(error) = self.agent_execution_lifecycle.shutdown().await {
            errors.push(format!("Agent Execution cleanup failed: {error}"));
        }
        let runtimes_stopped = match tokio::time::timeout(Duration::from_secs(15), engine_shutdown).await {
            Ok(Ok(())) => true,
            Ok(Err(error)) => { errors.push(format!("Agent runtime cleanup failed: {error}")); false }
            Err(_) => { errors.push("Agent runtime cleanup timed out".into()); false }
        };
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.terminal_service.shutdown_cleanup(),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => errors.push(format!("terminal cleanup failed: {error}")),
            Err(_) => errors.push("terminal cleanup timed out after 5 seconds".to_owned()),
        }

        // Retain native cleanup authority when an Agent may still submit work.
        // A later shutdown retries the same quarantined runtime; it cannot reopen.
        if runtimes_stopped {
            if let Err(error) = self.shutdown_browser_platform().await {
                errors.push(format!("browser/gateway cleanup failed: {error:#}"));
            }
        }
        if let Some(robot) = &self.robot {
            robot.shutdown();
        }
        match tokio::time::timeout(Duration::from_secs(5), self.ssh_pool.shutdown_all()).await {
            Ok(report) if report.lost == 0 => {}
            Ok(report) => errors.push(format!(
                "{} SSH link(s) were released without proof the remote shell stopped",
                report.lost
            )),
            Err(_) => errors.push("SSH cleanup timed out after 5 seconds".to_owned()),
        }

        // Channel/terminal/browser shutdown hooks are not expected to publish
        // new host tasks, but seal the registry once more after every producer
        // has stopped. A defensive post-close registration reopens Closing and
        // is therefore caught here instead of racing the database close.
        let final_background_errors = self
            .shutdown_background_tasks(Duration::from_secs(1))
            .await;
        if !final_background_errors.is_empty() {
            errors.extend(
                final_background_errors
                    .into_iter()
                    .map(|error| format!("final background task cleanup failed: {error}")),
            );
        }

        if errors.is_empty() {
            tokio::time::timeout(Duration::from_secs(5), self.companion_service.close_storage())
                .await
                .map_err(|_| anyhow::anyhow!("Companion storage cleanup timed out"))?;
            self.database.close().await;
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Nomi-core cleanup failed: {}",
                errors.join("; ")
            ))
        }
    }

    pub(crate) fn set_channel_manager(
        &self,
        manager: Arc<nomifun_channel::manager::ChannelManager>,
    ) {
        *self
            .channel_manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(manager);
    }

    pub(crate) async fn shutdown_channel_manager(&self) -> anyhow::Result<()> {
        let manager = self
            .channel_manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(manager) = manager {
            manager.shutdown().await;
        }
        Ok(())
    }

    /// Close browser resources and the database after a startup-stage failure,
    /// preserving the original failure as the primary error.
    pub async fn cleanup_after_startup_failure(&self, error: anyhow::Error) -> anyhow::Error {
        self.request_background_shutdown();
        if let Err(error) = self
            .plugin_runtime
            .shutdown_service_runtime(self.authoritative_user_id.as_ref())
            .await
        {
            tracing::error!(
                %error,
                "Plugin Service cleanup failed during startup failure cleanup"
            );
        }
        if let Err(cleanup_error) = self.quiesce_auto_work_runner().await {
            tracing::error!(
                %cleanup_error,
                "AutoWork runner did not shut down during startup cleanup"
            );
        }
        let background_errors = self
            .shutdown_background_tasks(Duration::from_secs(10))
            .await;
        if !background_errors.is_empty() {
            tracing::error!(
                errors = ?background_errors,
                "background tasks did not fully quiesce during startup cleanup"
            );
        }
        if let Err(error) = self.shutdown_channel_manager().await {
            tracing::error!(%error, "channel manager did not shut down during startup cleanup");
        }
        if !self.nomi_core_remote_runtime.shutdown().await {
            tracing::error!(
                "Nomi-core Remote tasks remained active during startup failure cleanup"
            );
        }
        let authority = Arc::new(StartupCleanupAuthority::new(self.database.clone()));
        authority
            .install_browser_platform(self.browser_platform_shutdown.clone())
            .await;
        finish_startup_cleanup(authority, error).await
    }

    /// Wire the dependency bundle into the Platform Gateway MCP server.
    /// Called from `create_router` after `build_module_states` (the
    /// canonical AgentSession/Cron adapters live there).
    pub(crate) async fn inject_gateway_deps(&self, deps: Arc<nomifun_gateway::CompatibilityCapabilityHost>) {
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
        Self::try_from_config_with_host(database, config, crate::desktop::DesktopHostServices::default()).await
    }

    pub(crate) async fn try_from_config_with_host(
        database: Database,
        config: &AppConfig,
        host_services: crate::desktop::DesktopHostServices,
    ) -> Result<Self, RetainedAppServicesConstructionError> {
        let startup_cleanup_authority =
            Arc::new(StartupCleanupAuthority::new(database.clone()));
        match Self::from_config_inner(
            database,
            config,
            Arc::clone(&startup_cleanup_authority),
            host_services,
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
        host_services: crate::desktop::DesktopHostServices,
    ) -> anyhow::Result<Self> {
        #[cfg(not(feature = "browser-use"))]
        let _ = host_services;
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
        let agent_metadata_repo: Arc<dyn IAgentMetadataRepository> =
            Arc::new(SqliteAgentMetadataRepository::new(database.pool().clone()));
        let agent_registry = AgentRegistry::new(agent_metadata_repo);
        agent_registry
            .hydrate()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to hydrate agent registry: {e}"))?;


        // Skill paths need app resource dir (for builtin rules) + data dir
        // (for user skills + materialized views). AcpSkillManager uses these
        // for first-message skill index/body loading.
        let app_resource_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok())
            .and_then(|p| p.parent().map(|pp| pp.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let skill_paths = Arc::new(nomifun_skill_library::resolve_skill_paths(
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

        // 客服独立域 (customer-service domain): agents/notes/bindings CRUD
        // service + the stateless concurrent dialogue engine. The engine's
        // LLM turns go through the generic one-shot entry whose tool table is
        // restricted to the selected Agent's subset of three read-only tools —
        // no workspace mount, no runtime registry, no Conversation.
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

        // Plugin M1 starts from its own owner-scoped Product/Project/Release
        // data root. Legacy HTML snapshots and conversation workspaces are not
        // migrated, read, or dual-written.
        let plugin_runtime_repository =
            Arc::new(nomifun_db::SqlitePluginRuntimeRepository::new(
                database.pool().clone(),
            ));
        nomifun_db::IPluginRuntimeRepository::revoke_all_surface_sessions_on_startup(
            plugin_runtime_repository.as_ref(),
        )
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to revoke stale Plugin Surface sessions: {error}"
                )
            })?;
        let plugin_runtime_repository: Arc<dyn nomifun_db::IPluginRuntimeRepository> =
            plugin_runtime_repository;
        let plugin_runtime_store_root = data_dir.join("plugin-m1");
        let plugin_runtime_source_store = Arc::new(
            nomifun_plugin_platform::runtime::PluginRuntimeSourceStore::new(
                plugin_runtime_store_root.join("source"),
            )
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to initialize Plugin Source Store under {}: {error}",
                    plugin_runtime_store_root.display()
                )
            })?,
        );
        let plugin_runtime_release_store = Arc::new(
            nomifun_plugin_platform::runtime::PluginRuntimeReleaseStore::new(
                plugin_runtime_store_root.join("release"),
            )
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to initialize Plugin Release Store under {}: {error}",
                    plugin_runtime_store_root.display()
                )
            })?,
        );
        let plugin_runtime = Arc::new(
            nomifun_plugin_platform::runtime::PluginRuntimeApplicationService::new_with_stores(
                plugin_runtime_repository,
                plugin_runtime_source_store,
                plugin_runtime_release_store,
            )?,
        );

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
        // canonical AgentSession owner is built here so the device face and OTA
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

        // AppServices owns the lifecycle registry before router assembly. Its
        // sole factory resolves the source-installed provider lazily; typed
        // Session/Broker/Kernel ports are installed later in router state.
        let official_runtime = Arc::new(crate::router::official_runtime::OfficialRuntimeHost::default());
        let runtime_sessions_concrete = Arc::new(
            InMemoryAgentRuntimeSessions::new(official_runtime.factory())
                .with_model_config_resolver(build_agent_model_config_resolver(
                    model_invoke_service.clone(),
                )),
        );
        let agent_runtime_sessions: Arc<dyn AgentRuntimeSessions> = runtime_sessions_concrete.clone();
        let _runtime_sessions_owner = runtime_sessions_concrete;

        let background_shutdown = CancellationToken::new();
        let background_tasks = Arc::new(BackgroundTaskRegistry::new(background_shutdown.clone()));
        let services = Self {
            official_runtime,
            database,
            background_shutdown,
            background_tasks,
            channel_manager: Mutex::new(None),
            cron_service: Mutex::new(None),
            auto_work_runner: Mutex::new(None),
            agent_execution_lifecycle: AgentExecutionLifecycle::new(),
            nomi_core_remote_runtime: NomiCoreRemoteRuntimeCoordinator::new(),
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
            agent_runtime_sessions,
            agent_registry,
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
            customer_service_service,
            cs_dialogue_engine,
            workshop_service,
            plugin_runtime,
            creation_service,
            model_invoke_service,
            knowledge_service,
            #[cfg(feature = "browser-use")]
            browser_resources: host_services.browser_resources,
            #[cfg(feature = "browser-use")]
            attached_chrome: host_services.attached_chrome,
            #[cfg(feature = "browser-use")]
            local_web_search: host_services.local_web_search,
            #[cfg(feature="browser-use")]
            headless_render: host_services.headless_render,
            browser_platform_shutdown,
        };

        services.register_background_task(
            services.terminal_service.spawn_scrollback_flusher_with_shutdown(
                services.background_shutdown.clone(),
            ),
        );
        Ok(services)
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
    use std::sync::{Condvar, Mutex as StdMutex};

    struct BackgroundTaskDropProbe {
        dropped: Arc<AtomicUsize>,
    }

    impl Drop for BackgroundTaskDropProbe {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[tokio::test]
    async fn background_registry_rejects_unstarted_observers_during_and_after_shutdown() {
        let shutdown = CancellationToken::new();
        let registry = Arc::new(BackgroundTaskRegistry::new(shutdown.clone()));
        let registrar: Arc<dyn nomifun_conversation::BackgroundTaskRegistrar> = registry.clone();
        let dropped = Arc::new(AtomicUsize::new(0));
        let polled_rejections = Arc::new(AtomicUsize::new(0));
        let probe = BackgroundTaskDropProbe { dropped: dropped.clone() };
        let (started, ready) = tokio::sync::oneshot::channel();
        assert!(registrar.spawn(Box::pin(async move {
            let _probe = probe;
            let _ = started.send(());
            shutdown.cancelled().await;
        })));
        ready.await.unwrap();
        registry.request_shutdown();
        for phase in [BackgroundTaskRegistryPhase::Closing, BackgroundTaskRegistryPhase::Closed] {
            let probe = BackgroundTaskDropProbe { dropped: dropped.clone() };
            let polled = polled_rejections.clone();
            assert!(!registrar.spawn(Box::pin(async move {
                let _probe = probe;
                polled.fetch_add(1, Ordering::SeqCst);
            })));
            assert_eq!(registry.snapshot().0, phase, "rejected work cannot reopen a sealed registry");
            assert!(registry.shutdown(Duration::from_secs(1)).await.is_empty());
        }
        assert_eq!(polled_rejections.load(Ordering::SeqCst), 0);
        assert_eq!(dropped.load(Ordering::SeqCst), 3);
        assert_eq!(registry.snapshot(), (BackgroundTaskRegistryPhase::Closed, 0));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn background_registry_spawn_and_shutdown_share_one_admission_boundary() {
        let shutdown = CancellationToken::new();
        let registry = Arc::new(BackgroundTaskRegistry::new(shutdown.clone()));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let dropped = Arc::new(AtomicUsize::new(0));
        let owner = registry.clone();
        let producer_barrier = barrier.clone();
        let producer_dropped = dropped.clone();
        let producer = tokio::spawn(async move {
            producer_barrier.wait().await;
            for _ in 0..128 {
                let cancel = shutdown.clone();
                let probe = BackgroundTaskDropProbe { dropped: producer_dropped.clone() };
                owner.spawn(async move {
                    let _probe = probe;
                    cancel.cancelled().await;
                });
                tokio::task::yield_now().await;
            }
        });
        barrier.wait().await;
        assert!(registry.shutdown(Duration::from_secs(1)).await.is_empty());
        producer.await.unwrap();
        assert_eq!(dropped.load(Ordering::SeqCst), 128);
        assert_eq!(registry.snapshot(), (BackgroundTaskRegistryPhase::Closed, 0));
    }

    async fn wait_for_background_registry_snapshot(
        registry: &BackgroundTaskRegistry,
        expected_phase: BackgroundTaskRegistryPhase,
        expected_tasks: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if registry.snapshot() == (expected_phase, expected_tasks) {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("background registry did not reach the expected state");
    }

    #[tokio::test]
    async fn background_registry_drains_registration_racing_with_shutdown() {
        let shutdown = CancellationToken::new();
        let registry = Arc::new(BackgroundTaskRegistry::new(shutdown.clone()));

        let (first_started_tx, first_started_rx) = tokio::sync::oneshot::channel();
        let (release_first_tx, release_first_rx) = tokio::sync::oneshot::channel();
        registry.register(tokio::spawn(async move {
            let _ = first_started_tx.send(());
            let _ = release_first_rx.await;
        }));
        first_started_rx
            .await
            .expect("first background task did not start");

        let registry_for_shutdown = Arc::clone(&registry);
        let shutdown_task = tokio::spawn(async move {
            registry_for_shutdown
                .shutdown(Duration::from_secs(1))
                .await
        });
        shutdown.cancelled().await;
        wait_for_background_registry_snapshot(
            &registry,
            BackgroundTaskRegistryPhase::Closing,
            0,
        )
        .await;

        let dropped = Arc::new(AtomicUsize::new(0));
        let dropped_for_task = Arc::clone(&dropped);
        let (late_started_tx, late_started_rx) = tokio::sync::oneshot::channel();
        let late = tokio::spawn(async move {
            let _drop_probe = BackgroundTaskDropProbe {
                dropped: dropped_for_task,
            };
            let _ = late_started_tx.send(());
            std::future::pending::<()>().await;
        });
        late_started_rx
            .await
            .expect("late background task did not start");
        registry.register(late);
        release_first_tx
            .send(())
            .expect("first background task already stopped");

        let errors = shutdown_task
            .await
            .expect("background shutdown owner panicked");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(dropped.load(Ordering::Acquire), 1);
        assert_eq!(
            registry.snapshot(),
            (BackgroundTaskRegistryPhase::Closed, 0)
        );
    }

    #[tokio::test]
    async fn background_registry_registration_after_closed_is_retained_for_retry() {
        let registry = BackgroundTaskRegistry::new(CancellationToken::new());
        assert!(
            registry
                .shutdown(Duration::from_secs(1))
                .await
                .is_empty()
        );
        assert_eq!(
            registry.snapshot(),
            (BackgroundTaskRegistryPhase::Closed, 0)
        );

        let dropped = Arc::new(AtomicUsize::new(0));
        let dropped_for_task = Arc::clone(&dropped);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let late = tokio::spawn(async move {
            let _drop_probe = BackgroundTaskDropProbe {
                dropped: dropped_for_task,
            };
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.expect("late task did not start");
        registry.register(late);

        assert_eq!(
            registry.snapshot(),
            (BackgroundTaskRegistryPhase::Closing, 1),
            "a rejected post-close handle must remain owned until a shutdown retry joins it"
        );
        assert!(
            registry
                .shutdown(Duration::from_secs(1))
                .await
                .is_empty()
        );
        assert_eq!(dropped.load(Ordering::Acquire), 1);
        assert_eq!(
            registry.snapshot(),
            (BackgroundTaskRegistryPhase::Closed, 0)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn background_registry_timeout_is_bounded_and_retains_join_authority() {
        let registry = BackgroundTaskRegistry::new(CancellationToken::new());
        let release = Arc::new((StdMutex::new(false), Condvar::new()));
        let release_for_task = Arc::clone(&release);
        let completed = Arc::new(AtomicUsize::new(0));
        let completed_for_task = Arc::clone(&completed);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        registry.register(tokio::task::spawn_blocking(move || {
            let _ = started_tx.send(());
            let (released, release_signal) = &*release_for_task;
            let released = released
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _released = release_signal
                .wait_while(released, |released| !*released)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            completed_for_task.fetch_add(1, Ordering::AcqRel);
        }));
        started_rx.await.expect("blocking task did not start");

        let started_at = std::time::Instant::now();
        let errors = registry.shutdown(Duration::from_millis(40)).await;
        let elapsed = started_at.elapsed();
        let timed_out_snapshot = registry.snapshot();

        {
            let (released, release_signal) = &*release;
            *released
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
            release_signal.notify_all();
        }
        let retry_errors = registry.shutdown(Duration::from_secs(1)).await;

        assert!(
            elapsed < Duration::from_secs(1),
            "bounded shutdown took {elapsed:?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("retained for retry")),
            "{errors:?}"
        );
        assert_eq!(
            timed_out_snapshot,
            (BackgroundTaskRegistryPhase::Closing, 1)
        );
        assert!(retry_errors.is_empty(), "{retry_errors:?}");
        assert_eq!(completed.load(Ordering::Acquire), 1);
        assert_eq!(
            registry.snapshot(),
            (BackgroundTaskRegistryPhase::Closed, 0)
        );
    }

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

    struct ShutdownRegistryProbe {
        inner: Arc<dyn AgentRuntimeSessions>,
        allow: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait::async_trait]
    impl AgentRuntimeSessions for ShutdownRegistryProbe {
        fn get_runtime(&self, id:&str)->Option<nomifun_ai_agent::AgentRuntimeHandle> {self.inner.get_runtime(id)}
        async fn get_or_create_runtime(&self,id:&str,options:nomifun_ai_agent::types::AgentRuntimeBuildOptions)->Result<nomifun_ai_agent::AgentRuntimeHandle,nomifun_common::AppError> {self.inner.get_or_create_runtime(id,options).await}
        fn terminate(&self,id:&str,reason:Option<nomifun_common::AgentKillReason>)->Result<(),nomifun_common::AppError> {self.inner.terminate(id,reason)}
        fn terminate_all(&self) {self.inner.terminate_all();}
        fn active_runtime_count(&self)->usize {self.inner.active_runtime_count()}
        fn shutdown_and_wait(&self)->nomifun_ai_agent::RuntimeTeardown {
            if self.allow.load(Ordering::Acquire) {self.inner.shutdown_and_wait()}
            else {Box::pin(async {Err(nomifun_common::AppError::Internal("fixture: Agent exit is not proven".into()))})}
        }
    }

    #[tokio::test]
    async fn failed_agent_shutdown_retains_browser_and_database_until_retry() {
        let db=nomifun_db::init_database_memory().await.unwrap();
        let root=tempfile::tempdir().unwrap();
        let mut services=AppServices::from_config(db,&test_config(root.path())).await.unwrap();
        let allow=Arc::new(std::sync::atomic::AtomicBool::new(false));
        services.agent_runtime_sessions=Arc::new(ShutdownRegistryProbe {inner:services.agent_runtime_sessions.clone(),allow:allow.clone()});
        let browser_shutdowns=Arc::new(AtomicUsize::new(0));
        let original=services.browser_platform_shutdown.clone();
        let calls=browser_shutdowns.clone();
        services.browser_platform_shutdown=BrowserPlatformShutdown::from_steps(Some(BrowserShutdownStep::new("browser shutdown order",move || {
            let original=original.clone(); let calls=calls.clone();
            async move {calls.fetch_add(1,Ordering::AcqRel);original.shutdown().await.map_err(|error|error.to_string())}
        })));
        let failure=services.shutdown_nomi_core_host().await.unwrap_err();
        assert!(failure.to_string().contains("Agent runtime cleanup failed"));
        assert_eq!(browser_shutdowns.load(Ordering::Acquire),0);
        sqlx::query("SELECT 1").execute(services.database.pool()).await.unwrap();
        allow.store(true,Ordering::Release);
        services.shutdown_nomi_core_host().await.unwrap();
        assert_eq!(browser_shutdowns.load(Ordering::Acquire),1);
        assert!(services.database.pool().is_closed());
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
        assert!(tmp.path().join("plugin-m1/source").is_dir());
        assert!(tmp.path().join("plugin-m1/release").is_dir());
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
