//! Desktop in-process serving: a PERMANENT loopback listener for the app's own
//! webview plus an ON-DEMAND LAN listener for remote browsers, sharing ONE
//! router that is built exactly once.
//!
//! Why two listeners instead of rebinding one (see the design doc):
//! - The loopback serve task is never touched by a LAN toggle, so the desktop
//!   webview's long-lived `/ws` and in-flight requests never blip.
//! - A LAN bind failure (port in use, firewall) is reported via [`WebUiStatus`]
//!   without affecting the already-serving loopback listener.
//! - `connect-info` (real peer IP for rate-limiting) and the SPA static
//!   fallback are added ONLY to the LAN listener, leaving the loopback path and
//!   the standalone-web/test paths byte-identical.
//!
//! Trust model: the router is built under [`AuthPolicy::TrustLocalToken`] with a
//! per-boot secret. The desktop injects that secret into its own webview
//! (`window.__nomiLocalTrust`), which presents it on every request — so the
//! desktop webview is trusted with no login while remote LAN browsers must log
//! in. Trust is the secret, NOT "arrived on loopback", so other local OS
//! accounts and same-host reverse proxies are not trusted.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::{future::Future, time::Duration};

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Bytes;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use nomifun_auth::generate_random_hex_secret;
use percent_encoding::percent_decode_str;
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, OnceCell, watch};
use tower_http::services::{ServeDir, ServeFile};

use crate::cli::Cli;
use crate::lan_endpoint::detect_all_lan_ipv4s;
use crate::{AppServices, bootstrap, create_router};
use nomifun_auth::AuthPolicy;
use nomifun_db::{IClientPreferenceRepository, IUserRepository};

/// Stable, bookmarkable port for the LAN listener (matches the UI's
/// `WEBUI_DEFAULT_PORT`). Falls back to an ephemeral port if occupied.
pub const WEBUI_LAN_PORT: u16 = 25808;

/// `client_preferences` row holding the desktop "WebUI remote access" switch.
///
/// The ONLY writer is the frontend `WebuiServerContext`
/// (`ui/src/renderer/hooks/context/WebuiServerContext.tsx`, via configService →
/// `PUT /api/settings/client`); [`DesktopServer::restore_lan_if_requested`] is
/// the ONLY reader. Before that reader existed the preference was write-only, so
/// every desktop restart came up loopback-only even though the user had left the
/// switch on — phones and robots then found a closed port until someone opened
/// the panel and toggled it again.
const DESKTOP_WEBUI_ENABLED_PREF_KEY: &str = "webui.desktop.enabled";

/// Result of the startup LAN-restore step, so the caller (and the smoke test)
/// can tell "the user never asked" apart from "asked, but refused/failed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanRestoreOutcome {
    /// No stored request to expose the LAN listener: loopback only.
    NotRequested,
    /// Requested, but the installation still has no admin credential. Binding
    /// `0.0.0.0` in that state would let the first visitor on the network claim
    /// the empty-password admin via `/api/auth/setup`.
    BlockedWithoutCredential,
    /// The LAN listener is serving again.
    Restored,
    /// Restore was requested but could not be completed (preference read
    /// failure, bind failure, missing app shell…). Startup continues.
    Failed,
}

/// Whether the persisted preference is a positive request to expose LAN serving.
///
/// Fail-safe by construction: ONLY a strict JSON `true` opens a listener on
/// `0.0.0.0`. A missing key, an empty value, `false`, the JSON *string*
/// `"true"`, `1`, `null`, or any parse failure all read as "stay loopback-only".
/// An ambiguous stored value must never be resolved in favour of exposing the
/// machine to the network.
fn lan_restore_requested(stored: Option<&str>) -> bool {
    stored
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .is_some_and(|value| matches!(value, serde_json::Value::Bool(true)))
}

/// Whether a failed desktop startup positively verified teardown of every
/// resource acquired before the failure was returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupCleanupDisposition {
    /// Startup either failed before acquiring long-lived resources or
    /// explicitly completed every required teardown step.
    Verified,
    /// Startup acquired resources whose cleanup did not complete. The caller
    /// must retain the Tokio runtime and fail closed instead of treating the
    /// returned error as proof that teardown succeeded.
    Unverified,
}

/// Typed failure returned by [`DesktopServer::start_with_outcome`].
///
/// The cleanup disposition is intentionally independent of the error text:
/// callers must never infer cleanup authority by parsing or classifying an
/// `anyhow::Error`.
pub struct DesktopStartError {
    error: anyhow::Error,
    cleanup_disposition: StartupCleanupDisposition,
    /// Retained startup authority when cleanup could not be verified.
    ///
    /// The native shell must keep this alive and retry before releasing its
    /// backend runtime. It is consumed exactly once by the startup supervisor.
    retained_keep_alive: Option<DesktopKeepAlive>,
}

impl DesktopStartError {
    fn verified(error: anyhow::Error) -> Self {
        Self {
            error,
            cleanup_disposition: StartupCleanupDisposition::Verified,
            retained_keep_alive: None,
        }
    }

    fn unverified(error: anyhow::Error, keep_alive: DesktopKeepAlive) -> Self {
        Self {
            error,
            cleanup_disposition: StartupCleanupDisposition::Unverified,
            retained_keep_alive: Some(keep_alive),
        }
    }

    /// Fail closed when an internal invariant prevents us from proving cleanup
    /// and no retry authority can be recovered. The native supervisor must
    /// retain its runtime and refuse process handoff rather than treating an
    /// inconsistent state as verified teardown.
    fn unverified_without_authority(error: anyhow::Error) -> Self {
        Self {
            error,
            cleanup_disposition: StartupCleanupDisposition::Unverified,
            retained_keep_alive: None,
        }
    }

    pub fn cleanup_disposition(&self) -> StartupCleanupDisposition {
        self.cleanup_disposition
    }

    pub fn cleanup_verified(&self) -> bool {
        self.cleanup_disposition == StartupCleanupDisposition::Verified
    }

    /// Transfer the retained startup authority to the caller.
    pub fn take_retained_keep_alive(&mut self) -> Option<DesktopKeepAlive> {
        self.retained_keep_alive.take()
    }

    pub fn into_inner(self) -> anyhow::Error {
        self.error
    }
}

impl std::fmt::Debug for DesktopStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopStartError")
            .field("error", &self.error)
            .field("cleanup_disposition", &self.cleanup_disposition)
            .field(
                "retained_keep_alive",
                &self.retained_keep_alive.as_ref().map(|_| "<retained>"),
            )
            .finish()
    }
}

impl std::fmt::Display for DesktopStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, formatter)
    }
}

impl std::error::Error for DesktopStartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

#[cfg(test)]
mod desktop_start_error_tests {
    use super::{DesktopStartError, StartupCleanupDisposition};

    #[test]
    fn invariant_failure_without_authority_is_unverified() {
        let error = DesktopStartError::unverified_without_authority(anyhow::anyhow!(
            "synthetic inconsistent cleanup state"
        ));

        assert_eq!(
            error.cleanup_disposition(),
            StartupCleanupDisposition::Unverified
        );
        assert!(!error.cleanup_verified());
    }
}

/// One frontend asset resolved by the desktop host.
///
/// The desktop crate adapts Tauri's compile-time `frontendDist` asset store to
/// this host-agnostic shape. Keeping the resolver abstraction in `nomifun-app`
/// avoids a Tauri dependency in the backend while letting the LAN listener
/// serve the exact bytes already embedded in the desktop executable.
#[derive(Clone, Debug)]
pub struct WebUiAsset {
    pub bytes: Bytes,
    pub content_type: String,
    pub csp_header: Option<String>,
}

impl WebUiAsset {
    pub fn new(bytes: impl Into<Bytes>, content_type: impl Into<String>) -> Self {
        Self {
            bytes: bytes.into(),
            content_type: content_type.into(),
            csp_header: None,
        }
    }

    pub fn with_csp_header(mut self, csp_header: Option<String>) -> Self {
        self.csp_header = csp_header;
        self
    }
}

/// Immutable, thread-safe snapshot of the desktop's embedded WebUI assets.
///
/// The desktop host resolves and decompresses Tauri's `frontendDist` exactly
/// once during startup, then drops Tauri's resolver before placing this source
/// in managed backend state. Requests therefore share ref-counted bytes without
/// repeated decompression, unbounded blocking work, or an AppManager reference
/// cycle. Unknown client routes use the same HTML fallback order as Tauri.
#[derive(Clone)]
pub struct WebUiAssetSource {
    assets: Arc<HashMap<String, WebUiAsset>>,
}

impl WebUiAssetSource {
    pub fn new<I, K>(assets: I) -> Self
    where
        I: IntoIterator<Item = (K, WebUiAsset)>,
        K: Into<String>,
    {
        let assets = assets
            .into_iter()
            .map(|(key, asset)| {
                let key = key.into();
                (key.trim_start_matches('/').to_owned(), asset)
            })
            .collect();
        Self {
            assets: Arc::new(assets),
        }
    }

    fn resolve(&self, path: &str) -> Option<WebUiAsset> {
        let decoded = percent_decode_str(path).decode_utf8_lossy();
        // Tauri's Windows AssetKey converts native separators to URL-style
        // slashes. Normalize them on every host so encoded `\` has identical
        // lookup semantics across macOS, Linux, and Windows.
        let normalized = decoded.replace('\\', "/");
        let mut path = normalized.as_str();
        if path.ends_with('/') {
            path = &path[..path.len() - 1];
        }
        let path = path.strip_prefix('/').unwrap_or(path);
        let path = if path.is_empty() { "index.html" } else { path };

        self.assets
            .get(path)
            .or_else(|| self.assets.get(&format!("{path}.html")))
            .or_else(|| self.assets.get(&format!("{path}/index.html")))
            .or_else(|| self.assets.get("index.html"))
            .cloned()
    }
}

/// Snapshot of the WebUI serving state, surfaced to the desktop UI and emitted
/// on every change. Field names match the frontend `IWebUIStatus` /
/// `IWebUIStartResult` shapes.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebUiStatus {
    /// LAN listener is active (remote browsers can connect).
    pub running: bool,
    /// LAN listener port when running (else 0).
    pub port: u16,
    /// Whether remote (non-loopback) access is allowed.
    pub allow_remote: bool,
    /// `http://localhost:<loopback_port>` — the desktop's own origin.
    pub local_url: String,
    /// `http://<lan_ip>:<port>` — the primary/preferred LAN URL.
    pub network_url: Option<String>,
    /// A quick-access URL for EVERY non-loopback IPv4 NIC (`http://<ip>:<port>`),
    /// routing-preferred first. A multi-homed / VPN host has several; the UI
    /// lists them all so the user can pick whichever their other device reaches.
    pub network_urls: Vec<String>,
    /// Detected LAN/VPN IPv4 used to build the primary network URL.
    #[serde(rename = "lanIP")]
    pub lan_ip: Option<String>,
    /// Admin username for the remote login (default `admin`).
    pub admin_username: String,
    /// Whether a real admin password has been set (non-empty `password_hash`).
    /// Lets the desktop UI distinguish "credential configured (hidden)" from
    /// "never provisioned" even when the LAN server is stopped, so a persisted
    /// password is not perceived as lost after a restart.
    pub password_set: bool,
    /// One-time initial password, set ONLY in the direct return of a first
    /// `start` that provisioned credentials. Never broadcast on the status
    /// channel (so it is shown once, not persisted in memory).
    pub initial_password: Option<String>,
    /// Populated when the last start attempt failed (e.g. port in use).
    pub error: Option<String>,
}

struct LanListener {
    shutdown: watch::Sender<bool>,
    termination: ListenerTermination,
    port: u16,
}

/// The listener's lifecycle is represented by a state transition, rather than
/// by two independently observed flags.  In particular, once a listener has
/// reached either terminal state, a later stop request cannot rewrite that
/// result.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ListenerCompletion {
    Running,
    StopRequested,
    RequestedStop,
    UnexpectedExit(String),
}

impl ListenerCompletion {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::RequestedStop | Self::UnexpectedExit(_)
        )
    }
}

/// Synchronous state transition authority shared by the listener task and its
/// management commands.  The lock is held only while publishing a lifecycle
/// transition; it is never held across an await or browser/router operation.
#[derive(Clone)]
struct ListenerTermination {
    state: Arc<StdMutex<ListenerCompletion>>,
    completion_tx: watch::Sender<ListenerCompletion>,
}

impl ListenerTermination {
    fn new() -> Self {
        let (completion_tx, _) = watch::channel(ListenerCompletion::Running);
        Self {
            state: Arc::new(StdMutex::new(ListenerCompletion::Running)),
            completion_tx,
        }
    }

    fn subscribe(&self) -> watch::Receiver<ListenerCompletion> {
        self.completion_tx.subscribe()
    }

    fn same_listener(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    fn snapshot(&self) -> ListenerCompletion {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn request_stop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(*state, ListenerCompletion::Running) {
            *state = ListenerCompletion::StopRequested;
            self.completion_tx.send_replace(state.clone());
        }
    }

    /// Publish the one immutable terminal result for this listener.
    ///
    /// A clean server result is considered user-requested only when the stop
    /// transition was already published.  Errors always win over an in-flight
    /// stop request, so a request that arrives after an abnormal exit cannot
    /// suppress the failure.
    fn complete(&self, result: Result<(), String>) -> ListenerCompletion {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.is_terminal() {
            return state.clone();
        }

        let completion = match result {
            Ok(()) if matches!(*state, ListenerCompletion::StopRequested) => {
                ListenerCompletion::RequestedStop
            }
            Ok(()) => ListenerCompletion::UnexpectedExit(
                "listener exited before shutdown was requested".to_owned(),
            ),
            Err(error) => ListenerCompletion::UnexpectedExit(error),
        };
        *state = completion.clone();
        self.completion_tx.send_replace(completion.clone());
        completion
    }
}

/// Completion signals for the two listener tasks. `watch` is used instead of
/// a one-shot notification so a waiter that subscribes after a task exits
/// still observes the terminal state.
struct ListenerLifecycle {
    loopback_termination: ListenerTermination,
}

/// Owns the desktop's in-process backend serving. Construct with
/// [`DesktopServer::start`]; drive the LAN listener with the `*_blocking`
/// methods (safe to call from Tauri command threads).
///
/// Holds only `Send + Sync` handles so it can live in Tauri managed state. The
/// heavy, app-lifetime state (the exclusive data-dir lock and all service
/// handles the router depends on) is returned separately as [`DesktopKeepAlive`]
/// and held by the backend thread.
pub struct DesktopServer {
    loopback_port: u16,
    local_trust_secret: Arc<str>,
    /// Built once; cloned per listener.
    router: Router,
    /// Bundled SPA directory (`ui/dist`) served to remote browsers by the LAN
    /// listener in PRODUCTION. This is a compatibility fallback for hosts that
    /// cannot expose their compile-time asset store.
    spa_dir: Option<PathBuf>,
    /// Canonical production source: the exact `frontendDist` bytes already
    /// embedded in the Tauri executable. Unlike `spa_dir`, this is independent
    /// of bundle layout, current working directory, and platform path rules.
    webui_asset_source: Option<WebUiAssetSource>,
    /// In DEV, the vite dev-server URL (e.g. `http://localhost:5173`) the desktop
    /// webview itself loads. When set, the LAN listener PROXIES the SPA to it
    /// instead of serving the (stale) bundled `ui/dist`, so remote browsers get
    /// the exact same live frontend the desktop shows. `None` in production.
    dev_frontend_url: Option<Arc<str>>,
    /// Backend runtime handle, so Tauri command threads can drive async work.
    runtime: Handle,
    /// The singleton terminal service (live PTY map + session repo). Held here
    /// so the unified shutdown path can clean up all terminal sessions before
    /// the database is closed.
    terminal_service: Arc<nomifun_terminal::TerminalService>,
    /// The process SSH connection pool. Held here so the unified shutdown path can
    /// close every live remote session — and write the resulting host status — while
    /// the database is still open.
    ssh_pool: nomifun_ssh::SshConnectionPool,
    /// LAN robot gateway. Held here for two reasons: the unified shutdown path
    /// stops its accept loop and loopback MCP front, and the LAN listener's
    /// status is projected into its endpoint advertiser (the OTA response is the
    /// only channel that can tell a device where to connect).
    robot: Option<Arc<crate::robot_wiring::RobotServices>>,
    /// Clone of the database pool used by the embedded router. Closing this
    /// clone closes the shared pool, so fatal listener failures cannot leave
    /// the backend's persistent resources alive while the host is exiting.
    database: nomifun_db::Database,
    /// Complete startup authority. Keeping this alongside the published
    /// server prevents a listener failure from dropping the environment lock
    /// or long-lived services before cleanup has been verified.
    _keep_alive: DesktopKeepAlive,
    /// Shared process-wide Gateway/Browser shutdown authority. Desktop
    /// exit/restart always stops the Gateway; browser-enabled builds also stop
    /// ACP browser ingress and then join the same Hub shutdown flight used by
    /// services/server cleanup.
    browser_platform_shutdown: crate::services::BrowserPlatformShutdown,
    /// The first unexpected listener failure is delivered to the desktop
    /// backend thread, which then drops [`DesktopKeepAlive`] and exits the
    /// host. `watch` keeps this signal observable without exposing internals.
    failure_tx: watch::Sender<Option<String>>,
    /// Gracefully stop the permanent loopback listener during fatal cleanup.
    loopback_shutdown: watch::Sender<bool>,
    listener_lifecycle: ListenerLifecycle,
    fatal_reported: Arc<AtomicBool>,
    /// Process-wide cleanup single-flight. Fatal listener cleanup and the
    /// Tauri exit hook must share one terminal result rather than racing
    /// independent terminal/browser/database teardown paths.
    shutdown_success: Arc<OnceCell<()>>,
    /// Published only by an explicit desktop-shell shutdown entry point, after
    /// every ordered shutdown stage succeeds and its completion callback has
    /// run. Fatal listener cleanup calls [`Self::shutdown_all`] directly and
    /// must publish `failure_tx` instead of looking like a normal shell exit.
    shutdown_complete_tx: watch::Sender<bool>,
    shutdown_complete_rx: watch::Receiver<bool>,
    /// For the pre-LAN safety gate (refuse to expose before an admin exists).
    user_repo: Arc<dyn IUserRepository>,
    lan: Mutex<Option<LanListener>>,
    status_tx: watch::Sender<WebUiStatus>,
    status_rx: watch::Receiver<WebUiStatus>,
}

/// App-lifetime state that must outlive the server: the exclusive data-dir lock
/// (inside `ServerEnvironment`) and all long-lived service handles (MCP servers,
/// background tasks) the router depends on. The backend thread holds this for
/// the process lifetime; dropping it tears the backend down.
#[derive(Clone)]
pub struct DesktopKeepAlive {
    inner: Arc<DesktopKeepAliveInner>,
}

struct DesktopKeepAliveInner {
    _env: bootstrap::ServerEnvironment,
    cleanup: DesktopStartupCleanupAuthority,
}

enum DesktopStartupCleanupAuthority {
    Services(AppServices),
    Startup(Arc<crate::services::StartupCleanupAuthority>),
}

impl DesktopKeepAlive {
    /// Retry the authoritative cleanup required when startup fails after
    /// service construction has acquired long-lived resources.
    ///
    /// A fully constructed service graph uses its shared browser-platform
    /// shutdown authority. A construction failure instead retains the
    /// startup-only authority supplied by `AppServices::try_from_config`.
    /// Either path leaves the database open when cleanup fails so a later retry
    /// can use the exact retained authority.
    pub async fn shutdown_after_startup_failure(&self) -> anyhow::Result<()> {
        match &self.inner.cleanup {
            DesktopStartupCleanupAuthority::Services(services) => {
                services.shutdown_computer_history().await;
                services.shutdown_browser_platform().await?;
                services.database.close().await;
                Ok(())
            }
            DesktopStartupCleanupAuthority::Startup(authority) => authority.cleanup().await,
        }
    }

    /// Bridge retained startup cleanup onto the still-live backend runtime.
    pub fn shutdown_after_startup_failure_blocking(
        &self,
        runtime: &tokio::runtime::Runtime,
    ) -> anyhow::Result<()> {
        let keep_alive = self.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        runtime.spawn(async move {
            let _ = tx.send(keep_alive.shutdown_after_startup_failure().await);
        });
        rx.recv_timeout(Duration::from_secs(31)).map_err(|error| {
            anyhow::anyhow!(
                "retained desktop startup cleanup did not report back: {error}"
            )
        })?
    }

    fn from_parts(env: bootstrap::ServerEnvironment, services: AppServices) -> Self {
        Self {
            inner: Arc::new(DesktopKeepAliveInner {
                _env: env,
                cleanup: DesktopStartupCleanupAuthority::Services(services),
            }),
        }
    }

    fn from_startup_cleanup_authority(
        env: bootstrap::ServerEnvironment,
        authority: Arc<crate::services::StartupCleanupAuthority>,
    ) -> Self {
        Self {
            inner: Arc::new(DesktopKeepAliveInner {
                _env: env,
                cleanup: DesktopStartupCleanupAuthority::Startup(authority),
            }),
        }
    }

    fn services(&self) -> Option<&AppServices> {
        match &self.inner.cleanup {
            DesktopStartupCleanupAuthority::Services(services) => Some(services),
            DesktopStartupCleanupAuthority::Startup(_) => None,
        }
    }
}

async fn cleanup_start_failure(
    keep_alive: DesktopKeepAlive,
    error: anyhow::Error,
) -> DesktopStartError {
    match keep_alive.shutdown_after_startup_failure().await {
        Ok(()) => {
            drop(keep_alive);
            DesktopStartError::verified(error)
        }
        Err(cleanup_error) => DesktopStartError::unverified(
            anyhow::anyhow!(
                "{error:#}; managed browser platform cleanup after startup failure also failed: {cleanup_error:#}"
            ),
            keep_alive,
        ),
    }
}

impl DesktopServer {
    /// Boot the embedded backend under `TrustLocalToken`, bind the permanent
    /// loopback listener, and spawn its serve task. Returns the shared handle
    /// plus a [`DesktopKeepAlive`] the caller MUST keep alive for the process
    /// lifetime; the loopback listener runs until the process exits.
    ///
    /// `spa_dir` is the bundled `ui/dist` directory used to serve the app shell
    /// to remote browsers as a compatibility fallback. `dev_frontend_url` (e.g.
    /// `http://localhost:5173`) is set ONLY in dev: the LAN listener then proxies
    /// the SPA to the vite dev server so remote browsers match the live desktop.
    /// `webui_asset_source` is the preferred production source and should adapt
    /// the desktop host's compile-time embedded frontend assets.
    pub async fn start(
        cli: &Cli,
        merged_path: &str,
        spa_dir: Option<PathBuf>,
        dev_frontend_url: Option<String>,
        webui_asset_source: Option<WebUiAssetSource>,
    ) -> Result<(Arc<DesktopServer>, DesktopKeepAlive)> {
        Self::start_with_outcome(
            cli,
            merged_path,
            spa_dir,
            dev_frontend_url,
            webui_asset_source,
        )
        .await
        .map_err(DesktopStartError::into_inner)
    }

    /// Typed desktop startup entry point used by the native shell.
    ///
    /// Unlike [`Self::start`], failures retain a positive cleanup disposition
    /// so the shell can distinguish a safe-to-release runtime from a failed
    /// teardown that must retain runtime authority and fail closed.
    pub async fn start_with_outcome(
        cli: &Cli,
        merged_path: &str,
        spa_dir: Option<PathBuf>,
        dev_frontend_url: Option<String>,
        webui_asset_source: Option<WebUiAssetSource>,
    ) -> std::result::Result<
        (Arc<DesktopServer>, DesktopKeepAlive),
        DesktopStartError,
    > {
        let env = bootstrap::init_environment(cli, merged_path)
            .map_err(DesktopStartError::verified)?;

        // Override the CLI-derived policy: the desktop trusts its own webview via
        // a per-boot secret, and requires login for everyone else.
        // The desktop trusts its own webview via a per-boot secret presented in a
        // header (HTTP) and as a `Sec-WebSocket-Protocol` subprotocol (WS upgrade,
        // where browsers cannot set custom headers). The secret MUST therefore be
        // a valid subprotocol token — hex, not base64 (base64's `+`/`/`/`=` make
        // `new WebSocket(url, [secret])` throw, which silently killed every desktop
        // WebSocket → no live stream → the companion bubble never echoed).
        let secret: Arc<str> = Arc::from(generate_random_hex_secret().as_str());
        let mut config = env.config.clone();
        config.auth_policy = AuthPolicy::TrustLocalToken;
        config.local_trust_secret = Some(secret.clone());

        let database = bootstrap::init_data_layer(&config)
            .await
            .map_err(DesktopStartError::verified)?;
        let services = match AppServices::try_from_config(database, &config).await {
            Ok(services) => services,
            Err(failure) => {
                let (error, cleanup_error, authority) = failure.into_parts();
                return Err(match (cleanup_error, authority) {
                    (None, None) => DesktopStartError::verified(error),
                    (Some(cleanup_error), Some(authority)) => {
                        let keep_alive =
                            DesktopKeepAlive::from_startup_cleanup_authority(env, authority);
                        DesktopStartError::unverified(
                            anyhow::anyhow!(
                                "{error:#}; managed browser platform cleanup during AppServices startup also failed: {cleanup_error:#}"
                            ),
                            keep_alive,
                        )
                    }
                    _ => DesktopStartError::unverified_without_authority(anyhow::anyhow!(
                        "{error:#}; AppServices startup cleanup state was internally inconsistent"
                    )),
                });
            }
        };
        let services = match services
            .try_with_boot_reconciliation_authority(
                env.boot_reconciliation_authority(),
                &config,
            )
            .await
        {
            Ok(services) => services,
            Err(failure) => {
                let (services, error) = failure.into_parts();
                let keep_alive = DesktopKeepAlive::from_parts(env, services);
                return Err(cleanup_start_failure(keep_alive, error).await);
            }
        };
        if let Err(error) = bootstrap::finalize_data_layer(&config) {
            let keep_alive = DesktopKeepAlive::from_parts(env, services);
            return Err(cleanup_start_failure(keep_alive, error).await);
        }
        let user_repo = services.user_repo.clone();
        let terminal_service = services.terminal_service.clone();

        // Reserve the permanent loopback socket before router assembly. Router
        // construction starts background forwarders, so every fallible socket
        // setup step must complete first; otherwise a bind/local-address
        // failure could return from startup after detached work was published.
        let loopback = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .context("failed to bind loopback listener")
        {
            Ok(loopback) => loopback,
            Err(error) => {
                let keep_alive = DesktopKeepAlive::from_parts(env, services);
                return Err(cleanup_start_failure(keep_alive, error).await);
            }
        };
        let loopback_port = match loopback.local_addr() {
            Ok(address) => address.port(),
            Err(error) => {
                let keep_alive = DesktopKeepAlive::from_parts(env, services);
                return Err(
                    cleanup_start_failure(keep_alive, anyhow::Error::new(error)).await
                );
            }
        };
        let router = create_router(&services).await;

        // Seed the initial status with the PERSISTED admin identity so the
        // desktop UI shows the real username / "password set" state immediately
        // at boot, before the LAN listener is ever started.
        let (initial_admin, initial_pw_set) = resolve_admin(&*user_repo).await;
        let initial = WebUiStatus {
            running: false,
            local_url: format!("http://localhost:{loopback_port}"),
            admin_username: initial_admin,
            password_set: initial_pw_set,
            ..Default::default()
        };
        let (status_tx, status_rx) = watch::channel(initial);
        let (failure_tx, _) = watch::channel(None);
        let (loopback_shutdown, _) = watch::channel(false);
        let loopback_termination = ListenerTermination::new();
        let (shutdown_complete_tx, shutdown_complete_rx) = watch::channel(false);
        let keep_alive = DesktopKeepAlive::from_parts(env, services);
        let services = keep_alive
            .services()
            .expect("DesktopKeepAlive built from AppServices");

        tracing::info!(
            loopback_port,
            "desktop backend serving on loopback (LAN access off)"
        );

        let server = Arc::new(DesktopServer {
            loopback_port,
            local_trust_secret: secret,
            router,
            spa_dir,
            webui_asset_source,
            dev_frontend_url: dev_frontend_url.map(|u| Arc::from(u.trim_end_matches('/'))),
            runtime: Handle::current(),
            terminal_service,
            ssh_pool: services.ssh_pool.clone(),
            robot: services.robot.clone(),
            database: services.database.clone(),
            _keep_alive: keep_alive.clone(),
            browser_platform_shutdown: services.browser_platform_shutdown.clone(),
            failure_tx,
            loopback_shutdown,
            listener_lifecycle: ListenerLifecycle {
                loopback_termination,
            },
            fatal_reported: Arc::new(AtomicBool::new(false)),
            shutdown_success: Arc::new(OnceCell::new()),
            shutdown_complete_tx,
            shutdown_complete_rx,
            user_repo,
            lan: Mutex::new(None),
            status_tx,
            status_rx,
        });
        server.spawn_loopback(loopback);
        server.spawn_robot_endpoint_projection();
        // Tail of startup: the backend is ready and the app-shell source has
        // been resolved, so re-opening LAN serving here is the earliest moment
        // that can serve a real request. Doing it in Rust (not the webview) is
        // what makes a headless/auto-start boot and an early robot OTA poll find
        // an open port instead of waiting for the frontend to mount.
        server.restore_lan_if_requested().await;
        Ok((server, keep_alive))
    }

    /// The loopback port the webview connects to (`window.__backendPort`).
    pub fn loopback_port(&self) -> u16 {
        self.loopback_port
    }

    /// Keep the robot endpoint advertiser in step with the LAN listener.
    ///
    /// A robot is told where to connect exactly once, in its OTA response, and
    /// that address must be an interface the robot can reach — never loopback.
    /// So the advertiser follows the LAN listener specifically: LAN off means no
    /// reachable endpoint, and the OTA response then carries an empty websocket
    /// URL instead of one that dials nowhere.
    fn spawn_robot_endpoint_projection(self: &Arc<Self>) {
        let Some(robot) = self.robot.clone() else {
            return;
        };
        let mut status = self.subscribe_status();
        tokio::spawn(async move {
            loop {
                let (running, port) = {
                    let current = status.borrow_and_update();
                    (current.running, current.port)
                };
                let snapshot = nomifun_robot::endpoint::LanEndpointSnapshot {
                    enabled: running,
                    port,
                    // Re-detected per change rather than cached: a laptop that
                    // moves between networks keeps the same listener.
                    ipv4s: if running {
                        detect_all_lan_ipv4s()
                    } else {
                        Vec::new()
                    },
                };
                robot.endpoint_tx.send_if_modified(|current| {
                    if *current == snapshot {
                        return false;
                    }
                    tracing::info!(
                        enabled = snapshot.enabled,
                        port = snapshot.port,
                        interfaces = snapshot.ipv4s.len(),
                        "robot: reachable endpoint changed"
                    );
                    *current = snapshot;
                    true
                });
                if status.changed().await.is_err() {
                    break;
                }
            }
        });
    }

    /// The per-boot local-trust secret to inject into the webview
    /// (`window.__nomiLocalTrust`).
    pub fn local_trust_secret(&self) -> &str {
        &self.local_trust_secret
    }

    /// Current status snapshot.
    pub fn status(&self) -> WebUiStatus {
        self.status_rx.borrow().clone()
    }

    /// Status snapshot with the admin identity resolved FRESH from the DB.
    ///
    /// The cached watch value only carries the username while the LAN listener
    /// is running (it is set inside `start_lan`); a stopped or just-booted
    /// server would otherwise report an empty username and `password_set=false`,
    /// making a persisted credential look lost after a restart. This overlays
    /// the persisted `admin_username` / `password_set` so `getStatus` is always
    /// truthful regardless of LAN state.
    pub async fn status_snapshot(&self) -> WebUiStatus {
        let mut st = self.status();
        let (username, password_set) = resolve_admin(&*self.user_repo).await;
        st.admin_username = username;
        st.password_set = password_set;
        st
    }

    /// Subscribe to status changes (for emitting a Tauri event on each change).
    pub fn subscribe_status(&self) -> watch::Receiver<WebUiStatus> {
        self.status_rx.clone()
    }

    /// Subscribe to fatal embedded-listener failures. A value is published
    /// only after the server has stopped sibling listeners and attempted
    /// managed browser/database cleanup.
    pub fn subscribe_failure(&self) -> watch::Receiver<Option<String>> {
        self.failure_tx.subscribe()
    }

    /// Return the currently retained fatal listener failure, if any.
    pub fn current_failure(&self) -> Option<String> {
        self.failure_tx.borrow().clone()
    }

    /// Subscribe to completion of the ordered application shutdown.
    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown_complete_rx.clone()
    }

    fn spawn_loopback(self: &Arc<Self>, listener: TcpListener) {
        let server = Arc::clone(self);
        let router = self.router.clone();
        let mut shutdown_rx = self.loopback_shutdown.subscribe();
        let termination = self.listener_lifecycle.loopback_termination.clone();
        self.runtime.spawn(async move {
            let shutdown = async move {
                while shutdown_rx.changed().await.is_ok() {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            };
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
                .map_err(|error| error.to_string());
            // Publish the immutable terminal result before fatal cleanup. The
            // cleanup path waits for every listener and must never wait for
            // this supervisor itself.
            if let ListenerCompletion::UnexpectedExit(error) = termination.complete(result) {
                server.handle_listener_failure("loopback", error).await;
            }
        });
    }

    /// Wait until the listener publishes a terminal state and return it.
    ///
    /// `UnexpectedExit` is returned as `Ok`: it is an immutable terminal state
    /// proving the listener is already stopped, so a shutdown waiter must
    /// never treat it as "still needs stopping" — that would make every later
    /// `shutdown_all` fail forever against a state that cannot change (F15).
    /// Callers decide how loudly to report it.
    async fn wait_for_listener_completion(
        mut completion: watch::Receiver<ListenerCompletion>,
        component: &'static str,
    ) -> anyhow::Result<ListenerCompletion> {
        loop {
            let state = completion.borrow().clone();
            if state.is_terminal() {
                return Ok(state);
            }

            completion.changed().await.map_err(|_| {
                anyhow::anyhow!("{component} listener completion channel closed unexpectedly")
            })?;
        }
    }

    async fn stop_listeners_and_wait(&self) -> anyhow::Result<()> {
        self.listener_lifecycle
            .loopback_termination
            .request_stop();
        self.loopback_shutdown.send_replace(true);
        let lan_completion = {
            let lan = self.lan.lock().await;
            if let Some(listener) = lan.as_ref() {
                listener.termination.request_stop();
                listener.shutdown.send_replace(true);
                // Keep the owner in `self.lan` until completion is confirmed.
                // If this attempt is cancelled or times out, the next explicit
                // shutdown must still have the sender and completion receiver
                // needed to retry/wait rather than closing the DB underneath a
                // still-running listener.
                Some(listener.termination.subscribe())
            } else {
                None
            }
        };
        let loopback_completion = self
            .listener_lifecycle
            .loopback_termination
            .subscribe();
        let wait = async {
            let loopback_wait = Self::wait_for_listener_completion(loopback_completion, "loopback");
            let lan_wait = async {
                if let Some(completion) = lan_completion {
                    Self::wait_for_listener_completion(completion, "LAN")
                        .await
                        .map(Some)
                } else {
                    Ok(None)
                }
            };
            tokio::try_join!(loopback_wait, lan_wait)
        };
        match tokio::time::timeout(std::time::Duration::from_secs(5), wait).await {
            Ok(Ok((loopback_state, lan_state))) => {
                // A listener pinned in UnexpectedExit is by definition already
                // stopped: there is nothing left to stop, so shutdown proceeds
                // loudly instead of failing forever (which would leave the
                // database open and the app unable to ever exit).
                if let ListenerCompletion::UnexpectedExit(error) = &loopback_state {
                    tracing::error!(
                        %error,
                        "loopback listener had already exited unexpectedly; continuing shutdown"
                    );
                }
                if let Some(ListenerCompletion::UnexpectedExit(error)) = &lan_state {
                    tracing::error!(
                        %error,
                        "LAN listener had already exited unexpectedly; continuing shutdown"
                    );
                }
                // Remove the LAN inventory only after the listener completion
                // future itself returned a confirmed terminal state; an outer
                // timeout success carrying an inner error is not sufficient.
                let mut lan = self.lan.lock().await;
                if lan
                    .as_ref()
                    .is_some_and(|listener| listener.termination.snapshot().is_terminal())
                {
                    lan.take();
                }
                Ok(())
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(anyhow::anyhow!(
                "desktop listener shutdown timed out after 5 seconds"
            )),
        }
    }

    async fn perform_shutdown(&self) -> anyhow::Result<()> {
        // Stop ingress first so no new request can race terminal/browser or
        // database teardown. Each stage runs even when an earlier stage fails;
        // the final error retains every cleanup diagnostic.
        let mut errors = Vec::new();

        if let Err(error) = self.stop_listeners_and_wait().await {
            errors.push(format!("listener cleanup failed: {error:#}"));
        }

        let terminal_result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.terminal_service.shutdown_cleanup(),
        )
        .await;
        match terminal_result {
            Ok(Ok(deleted)) => {
                tracing::info!(deleted, "terminal sessions cleaned up during desktop shutdown");
            }
            Ok(Err(error)) => errors.push(format!("terminal cleanup failed: {error}")),
            Err(_) => errors.push(
                "terminal cleanup timed out after 5 seconds".to_owned(),
            ),
        }

        let browser_result: anyhow::Result<()> =
            self.browser_platform_shutdown.shutdown().await;
        if let Err(error) = browser_result {
            errors.push(format!("browser cleanup failed: {error:#}"));
        }

        // Stop the robot gateway before the listeners go: its accept loop and the
        // loopback MCP front are pure in-process tasks with nothing durable to
        // write, so this is unconditional and cannot fail. Live sessions end with
        // their own sockets when the listener closes.
        if let Some(robot) = &self.robot {
            robot.shutdown();
        }

        // Quiesce the SSH pool before the database closes: closing a link walks the
        // host row back from "connected", and a link let go of without exit evidence
        // is a real leak on someone else's machine, so it is reported rather than
        // silently counted as clean.
        let ssh_result = tokio::time::timeout(            std::time::Duration::from_secs(5),
            self.ssh_pool.shutdown_all(),
        )
        .await;
        match ssh_result {
            Ok(report) => {
                tracing::info!(
                    reaped = report.reaped,
                    lost = report.lost,
                    already_down = report.already_down,
                    "ssh links closed during desktop shutdown"
                );
                if report.lost > 0 {
                    errors.push(format!(
                        "{} ssh link(s) were let go of without proof the remote shell died",
                        report.lost
                    ));
                }
            }
            Err(_) => errors.push("ssh pool cleanup timed out after 5 seconds".to_owned()),
        }

        // Do not close the shared database after an earlier cleanup failure.
        // Terminal cleanup intentionally preserves durable rows on failure and
        // BrowserSessionHub retains Host authority for an explicit retry; closing
        // the shared pool here would make both retries fail for the wrong reason.
        // The database is therefore closed only after listeners, terminals, and
        // every explicit Host shutdown have all completed successfully.
        close_database_after_cleanup(errors, || self.database.close()).await
    }

    pub async fn shutdown_all(&self) -> anyhow::Result<()> {
        run_shutdown_once(&self.shutdown_success, || async {
            self.perform_shutdown().await
        })
        .await
    }

    fn publish_shutdown_complete(&self) {
        self.shutdown_complete_tx.send_replace(true);
    }

    async fn handle_listener_failure(&self, component: &str, error: String) {
        let raw_failure = format!("desktop {component} server exited unexpectedly: {error}");
        tracing::error!(component, error = %error, "embedded desktop listener exited");

        // Cleanup must finish (or hit the outer bound) before claiming the
        // single fatal report. This preserves the failure signal if cleanup is
        // cancelled and lets a later exit request retry a failed cleanup.
        let cleanup_result = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.shutdown_all(),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!(
                "desktop backend cleanup timed out after 30 seconds"
            )),
        };
        if !claim_first_fatal_report(&self.fatal_reported) {
            return;
        }
        let failure = merge_listener_failure_with_cleanup(raw_failure, cleanup_result);
        let mut status = self.status();
        status.running = false;
        status.error = Some(failure.clone());
        self.status_tx.send_replace(status);
        // The loopback task can fail before `start` returns and before a caller
        // subscribes. Retain that early fatal value instead of dropping it.
        self.failure_tx.send_replace(Some(failure));
    }

    /// Re-open LAN serving when the user left the "WebUI remote access" switch
    /// on, so remote access survives a desktop restart.
    ///
    /// Called exactly once at the tail of [`Self::start_with_outcome`]; it is
    /// public so the desktop smoke test can drive the identical restore path
    /// against a real backend without restarting a locked data directory.
    ///
    /// Two invariants:
    /// - Nothing here can fail startup. A preference read error, a missing app
    ///   shell, or a bind failure is logged and returns [`LanRestoreOutcome`];
    ///   remote access is a convenience and never a reason to refuse to launch.
    /// - No credential, no exposure. `has_users() == false` means the admin
    ///   password was never provisioned, and `start_lan` would generate one
    ///   silently at boot with nobody watching the one-time value — while the
    ///   port is already reachable. We stay loopback-only and say why.
    pub async fn restore_lan_if_requested(self: &Arc<Self>) -> LanRestoreOutcome {
        let prefs = nomifun_db::SqliteClientPreferenceRepository::new(
            self.database.pool().clone(),
        );
        let stored = match prefs.get_by_keys(&[DESKTOP_WEBUI_ENABLED_PREF_KEY]).await {
            Ok(rows) => rows
                .into_iter()
                .find(|row| row.key == DESKTOP_WEBUI_ENABLED_PREF_KEY)
                .map(|row| row.value),
            Err(error) => {
                tracing::warn!(
                    %error,
                    key = DESKTOP_WEBUI_ENABLED_PREF_KEY,
                    "could not read the persisted WebUI remote-access preference; LAN access stays off — re-enable it in Settings › WebUI"
                );
                return LanRestoreOutcome::Failed;
            }
        };
        if !lan_restore_requested(stored.as_deref()) {
            return LanRestoreOutcome::NotRequested;
        }

        match self.user_repo.has_users().await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    "WebUI remote access was left on, but this installation has no admin password yet, \
                     so the LAN listener was NOT restored: an exposed instance without a credential can \
                     be claimed by the first visitor on the network. Enable WebUI once from \
                     Settings › WebUI to provision the password; later restarts will then restore it."
                );
                return LanRestoreOutcome::BlockedWithoutCredential;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "could not verify that an admin credential exists; LAN access stays off"
                );
                return LanRestoreOutcome::Failed;
            }
        }

        let status = self.start_lan().await;
        if status.running {
            tracing::info!(
                port = status.port,
                "restored WebUI remote access from the persisted desktop preference"
            );
            LanRestoreOutcome::Restored
        } else {
            tracing::warn!(
                error = status.error.as_deref().unwrap_or("unknown"),
                "could not restore WebUI remote access; the desktop continues on loopback only"
            );
            LanRestoreOutcome::Failed
        }
    }

    /// Start LAN serving (bind `0.0.0.0:WEBUI_LAN_PORT`). Awaitable directly
    /// from an async Tauri command — the LAN serve task is spawned on the
    /// backend runtime regardless of which runtime drives this call.
    pub async fn start_lan(self: &Arc<Self>) -> WebUiStatus {
        let mut lan = self.lan.lock().await;
        if let Some(listener) = lan.as_ref() {
            if listener.termination.snapshot().is_terminal() {
                // A listener pinned in a terminal state (an unexpected exit
                // that fatal cleanup has not collected yet) is a corpse, not a
                // running server. Never let it permanently block re-enabling
                // LAN sharing: drop the dead entry and bind a fresh listener.
                tracing::warn!(
                    port = listener.port,
                    "removing dead LAN listener entry before starting a fresh one"
                );
                lan.take();
            } else {
                return self.status();
            }
        }

        if self.spa_dir.is_none()
            && self.dev_frontend_url.is_none()
            && self.webui_asset_source.is_none()
        {
            return self.fail_start(
                "WebUI app shell not found; remote browsers would receive QR/API endpoints but no app shell"
                    .to_string(),
            );
        }

        // Provision an admin credential before exposing the LAN listener.
        // Without one, the first remote visitor could claim the empty-password
        // admin via `/api/auth/setup`. On first enable we generate a strong
        // password and return it ONCE (shown to the owner), mirroring the old
        // desktop behavior; thereafter the stored credential is reused.
        //
        // We fill ONLY the password (`set_system_user_password_if_uninitialized`),
        // never the username: the panel lets the user rename the admin while
        // WebUI is off (which leaves password_hash empty), so overwriting the
        // username here would silently revert their chosen name back to "admin".
        let has_password = self.user_repo.has_users().await.unwrap_or(false);
        let mut initial_password: Option<String> = None;
        if !has_password {
            let pw = nomifun_auth::generate_password(20);
            let pw_for_hash = pw.clone();
            let hashed = match tokio::task::spawn_blocking(move || {
                nomifun_auth::hash_password(&pw_for_hash)
            })
            .await
            {
                Ok(Ok(h)) => h,
                Ok(Err(e)) => return self.fail_start(format!("生成访问密码失败: {e}")),
                Err(e) => return self.fail_start(format!("生成访问密码失败: {e}")),
            };
            match self
                .user_repo
                .set_system_user_password_if_uninitialized(&hashed)
                .await
            {
                // Freshly provisioned — surface the one-time plaintext once.
                Ok(true) => initial_password = Some(pw),
                // A password already existed (race / prior enable): reuse it.
                Ok(false) => {}
                Err(e) => return self.fail_start(format!("写入访问密码失败: {e}")),
            }
        }

        // Build the LAN app: shared router (/api, /ws) + the app shell.
        // In dev → proxy the SPA to the vite dev server (live, matches desktop).
        // In prod → serve Tauri's compile-time embedded frontend assets. A
        // filesystem `ui/dist` remains as a compatibility fallback.
        let mut app = self.router.clone();
        if let Some(dev_url) = &self.dev_frontend_url {
            let dev_url = dev_url.clone();
            app = app.fallback(move |req: Request| {
                let dev_url = dev_url.clone();
                async move { dev_spa_proxy(&dev_url, req).await }
            });
        } else if let Some(asset_source) = &self.webui_asset_source {
            let asset_source = asset_source.clone();
            app = app.fallback(move |req: Request| {
                let asset_source = asset_source.clone();
                async move { embedded_spa_response(asset_source, req).await }
            });
        } else if let Some(dir) = &self.spa_dir {
            app = app.fallback_service(
                ServeDir::new(dir)
                    .append_index_html_on_directories(true)
                    .fallback(ServeFile::new(dir.join("index.html"))),
            );
        }
        // DNS-rebinding guard on the LAN edge (Host/Origin must be IP/localhost).
        app = app.layer(axum::middleware::from_fn(host_guard_middleware));

        let lan_ips = detect_all_lan_ipv4s();
        let lan_ip = lan_ips.first().copied();
        let (admin_username, password_set) = resolve_admin(&*self.user_repo).await;

        let (port, listener) = match bind_lan(WEBUI_LAN_PORT).await {
            Ok(pair) => pair,
            Err(e) => return self.fail_start(format!("无法绑定局域网端口: {e}")),
        };

        let (sd_tx, mut sd_rx) = watch::channel(false);
        let termination = ListenerTermination::new();
        let make = app.into_make_service_with_connect_info::<SocketAddr>();
        let server = Arc::clone(self);
        let task_termination = termination.clone();
        // Publish the listener handle before spawning either task. A freshly
        // spawned task may fail immediately; cleanup must then see and revoke
        // this listener rather than racing a later inventory insertion.
        *lan = Some(LanListener {
            shutdown: sd_tx.clone(),
            termination,
            port,
        });
        self.runtime.spawn(async move {
            let shutdown = async move {
                while sd_rx.changed().await.is_ok() {
                    if *sd_rx.borrow() {
                        break;
                    }
                }
            };
            let result = axum::serve(listener, make)
                .with_graceful_shutdown(shutdown)
                .await
                .map_err(|error| error.to_string());
            // Publish the immutable terminal result before invoking fatal
            // cleanup so cleanup never waits for the supervisor performing it.
            if let ListenerCompletion::UnexpectedExit(error) =
                task_termination.complete(result)
            {
                server.handle_listener_failure("lan", error).await;
            }
        });

        let network_url = lan_ip.as_ref().map(|ip| format!("http://{ip}:{port}"));
        let network_urls: Vec<String> = lan_ips
            .iter()
            .map(|ip| format!("http://{ip}:{port}"))
            .collect();

        // Broadcast the persistent status WITHOUT the one-time password (it must
        // not linger in the watch channel / future getStatus calls).
        let broadcast = WebUiStatus {
            running: true,
            port,
            allow_remote: true,
            local_url: format!("http://localhost:{}", self.loopback_port),
            network_url: network_url.clone(),
            network_urls: network_urls.clone(),
            lan_ip: lan_ip.as_ref().map(|ip| ip.to_string()),
            admin_username: admin_username.clone(),
            password_set,
            initial_password: None,
            error: None,
        };
        tracing::info!(port, "desktop LAN serving started");
        let _ = self.status_tx.send(broadcast.clone());

        // Direct return carries the one-time initial password (if just provisioned).
        WebUiStatus {
            initial_password,
            ..broadcast
        }
    }

    /// Build a failed-start status, broadcast it, and return it.
    fn fail_start(self: &Arc<Self>, error: String) -> WebUiStatus {
        let st = WebUiStatus {
            running: false,
            local_url: format!("http://localhost:{}", self.loopback_port),
            error: Some(error),
            ..Default::default()
        };
        let _ = self.status_tx.send(st.clone());
        st
    }

    /// Stop LAN serving and return the resulting status. The loopback listener
    /// (the desktop's own webview) is unaffected.
    pub async fn stop_lan(self: &Arc<Self>) -> WebUiStatus {
        let previous_status = self.status();
        let stopped_listener = {
            let lan = self.lan.lock().await;
            if let Some(listener) = lan.as_ref() {
                let termination = listener.termination.clone();
                let completion = termination.subscribe();
                termination.request_stop();
                listener.shutdown.send_replace(true);
                tracing::info!(port = listener.port, "desktop LAN serving stopped");
                Some((listener.port, termination, completion))
            } else {
                None
            }
        };
        let stop_result = if let Some((port, termination, completion)) = stopped_listener {
            let wait = Self::wait_for_listener_completion(completion, "LAN");
            match tokio::time::timeout(std::time::Duration::from_secs(5), wait).await {
                Ok(Ok(completion_state)) => {
                    // Both terminal states prove the listener is stopped, so
                    // the inventory entry is removed either way — a dead
                    // (UnexpectedExit) entry must never block a later
                    // start_lan. Only the reported status differs.
                    let mut lan = self.lan.lock().await;
                    if lan
                        .as_ref()
                        .is_some_and(|listener| {
                            listener.termination.same_listener(&termination)
                                && listener.termination.snapshot().is_terminal()
                        })
                    {
                        lan.take();
                    }
                    drop(lan);
                    match completion_state {
                        ListenerCompletion::UnexpectedExit(error) => {
                            tracing::warn!(
                                port,
                                %error,
                                "LAN listener had already exited unexpectedly; entry removed so it can be restarted"
                            );
                            LanStopOutcome::StoppedAfterUnexpectedExit(error)
                        }
                        _ => LanStopOutcome::Stopped,
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(port, %error, "desktop LAN listener shutdown was not confirmed");
                    LanStopOutcome::Failed(error)
                }
                Err(_) => {
                    let error =
                        anyhow::anyhow!("desktop LAN listener shutdown timed out after 5 seconds");
                    tracing::warn!(port, %error);
                    LanStopOutcome::Failed(error)
                }
            }
        } else {
            LanStopOutcome::Stopped
        };
        // Carry the persisted admin identity into the stopped status so the UI
        // keeps showing the real username / password-set state after stopping.
        let (admin_username, password_set) = resolve_admin(&*self.user_repo).await;
        let status = stop_lan_status(
            previous_status,
            admin_username,
            password_set,
            stop_result,
        );
        let _ = self.status_tx.send(status.clone());
        status
    }

    /// Synchronously perform the complete application shutdown on the backend
    /// runtime. This is called from the Tauri main thread, so it schedules the
    /// async single-flight cleanup and waits without calling `Handle::block_on`.
    /// Cleanup is ordered: listeners, terminal sessions, BrowserSessionHub,
    /// then the database.
    pub fn shutdown_all_blocking(self: &Arc<Self>) -> anyhow::Result<()> {
        if self.shutdown_success.get().is_some() {
            tracing::info!(
                "desktop listeners, terminals, browser platform, and database already cleaned up"
            );
            self.publish_shutdown_complete();
            return Ok(());
        }

        // A failed attempt is not cached by `shutdown_all`, so one bounded
        // immediate retry can recover transient terminal/Host cleanup failures.
        // The caller receives the terminal failure and must not report cleanup
        // success merely because the bounded wait returned.
        let mut last_error = None;
        for attempt in 1..=2 {
            let server = Arc::clone(self);
            let (tx, rx) = std::sync::mpsc::channel();
            self.runtime.spawn(async move {
                let result = match tokio::time::timeout(
                    Duration::from_secs(30),
                    server.shutdown_all(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "desktop shutdown timed out after 30 seconds"
                    )),
                };
                let _ = tx.send(result);
            });
            match rx.recv_timeout(Duration::from_secs(31)) {
                Ok(Ok(())) => {
                    tracing::info!(
                        attempt,
                        "desktop listeners, terminals, browser platform, and database cleaned up on exit"
                    );
                    self.publish_shutdown_complete();
                    return Ok(());
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        attempt,
                        %error,
                        "desktop backend exit cleanup failed"
                    );
                    last_error = Some(error);
                }
                Err(error) => {
                    tracing::warn!(
                        attempt,
                        %error,
                        "desktop backend exit cleanup did not report back; proceeding with retry"
                    );
                    last_error = Some(anyhow::anyhow!(
                        "desktop backend exit cleanup did not report back: {error}"
                    ));
                }
            }
        }
        let error = last_error.unwrap_or_else(|| {
            anyhow::anyhow!(
                "desktop backend exit cleanup did not complete successfully after bounded retries"
            )
        });
        tracing::error!(
            %error,
            "desktop backend exit cleanup did not complete successfully after bounded retries"
        );
        Err(error)
    }

    /// Start the complete ordered shutdown without blocking the Tauri event
    /// loop. The callback runs on the backend runtime thread after cleanup
    /// succeeds or fails and is invoked exactly once for this request. This
    /// lower-level form deliberately does not publish a normal shell-exit
    /// notification, so setup/fatal cleanup cannot race its error dialog.
    pub fn cleanup_all_async<F>(self: &Arc<Self>, callback: F)
    where
        F: FnOnce(anyhow::Result<()>) + Send + 'static,
    {
        let server = Arc::clone(self);
        self.runtime.spawn(async move {
            let result = match tokio::time::timeout(
                Duration::from_secs(30),
                server.shutdown_all(),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!(
                    "desktop shutdown timed out after 30 seconds"
                )),
            };
            callback(result);
        });
    }

    /// Start an explicit desktop-shell shutdown. The callback is allowed to
    /// update the shell exit state and request the final Tauri exit before the
    /// backend supervisor is notified and drops its keep-alives.
    pub fn shutdown_all_async<F>(self: &Arc<Self>, callback: F)
    where
        F: FnOnce(anyhow::Result<()>) + Send + 'static,
    {
        let server = Arc::clone(self);
        self.cleanup_all_async(move |result| {
            let completed = result.is_ok();
            callback(result);
            if completed {
                server.publish_shutdown_complete();
            }
        });
    }
}

/// Run one shutdown attempt through the process-wide single-flight cell.
///
/// `tokio::sync::OnceCell::get_or_try_init` caches only the `Ok` value. An
/// error (or cancellation) leaves the cell uninitialized, which is exactly the
/// retry contract required by explicit shutdown: success is idempotent, while
/// failed cleanup retains authority for a later attempt.
async fn run_shutdown_once<F, Fut>(success: &OnceCell<()>, init: F) -> anyhow::Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    success.get_or_try_init(init).await.map(|_| ())
}

async fn close_database_after_cleanup<F, Fut>(
    errors: Vec<String>,
    close: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    if !errors.is_empty() {
        let error = anyhow::anyhow!("{}", errors.join("; "));
        tracing::warn!(
            error = %error,
            "desktop shutdown cleanup failed; database left open for retry"
        );
        return Err(error);
    }

    // Database::close closes the shared pool and is safe to call more than
    // once. It must be last so terminal cleanup can still read its durable
    // rows. Bound the await so a stuck connection cannot suppress the fatal
    // signal indefinitely.
    match tokio::time::timeout(Duration::from_secs(5), close()).await {
        Ok(()) => Ok(()),
        Err(_) => Err(anyhow::anyhow!(
            "database close timed out after 5 seconds"
        )),
    }
}

fn claim_first_fatal_report(fatal_reported: &AtomicBool) -> bool {
    !fatal_reported.swap(true, Ordering::AcqRel)
}

fn merge_listener_failure_with_cleanup(
    listener_failure: String,
    cleanup_result: anyhow::Result<()>,
) -> String {
    match cleanup_result {
        Ok(()) => listener_failure,
        Err(cleanup_error) => {
            format!("{listener_failure}; backend cleanup also failed: {cleanup_error:#}")
        }
    }
}

/// Result of one `stop_lan` request. Both `Stopped*` variants prove the
/// listener is down and its inventory entry was removed (restart possible);
/// `Failed` means stopping was not confirmed and the entry is retained.
enum LanStopOutcome {
    Stopped,
    StoppedAfterUnexpectedExit(String),
    Failed(anyhow::Error),
}

fn stop_lan_status(
    previous_status: WebUiStatus,
    admin_username: String,
    password_set: bool,
    stop_result: LanStopOutcome,
) -> WebUiStatus {
    match stop_result {
        LanStopOutcome::Stopped => WebUiStatus {
            running: false,
            local_url: previous_status.local_url,
            admin_username,
            password_set,
            ..Default::default()
        },
        // The listener is positively down (it died on its own); report a
        // stopped-with-error status instead of pretending it is still serving.
        LanStopOutcome::StoppedAfterUnexpectedExit(error) => WebUiStatus {
            running: false,
            local_url: previous_status.local_url,
            admin_username,
            password_set,
            error: Some(format!(
                "LAN listener had exited unexpectedly before it was stopped: {error}"
            )),
            ..Default::default()
        },
        LanStopOutcome::Failed(error) => WebUiStatus {
            admin_username,
            password_set,
            initial_password: None,
            error: Some(format!("failed to stop LAN listener: {error:#}")),
            ..previous_status
        },
    }
}

/// Serve a frontend response from the immutable embedded-asset snapshot. The
/// source mirrors Tauri's percent-decoding and SPA fallback; this adapter adds
/// the HTTP semantics needed by normal remote browsers.
async fn embedded_spa_response(asset_source: WebUiAssetSource, req: Request) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "GET, HEAD")
            .body(axum::body::Body::empty())
            .unwrap_or_else(|_| StatusCode::METHOD_NOT_ALLOWED.into_response());
    }

    let method = req.method().clone();
    let request_path = req.uri().path().to_owned();
    // Resolution is an in-memory hash lookup over ref-counted bytes. Tauri's
    // synchronous decompression already happened once during desktop startup.
    let asset = asset_source.resolve(&request_path);
    let Some(asset) = asset else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let content_length = asset.bytes.len();
    let is_head = method == Method::HEAD;
    let cache_control =
        if asset.content_type.starts_with("text/html") || !request_path.starts_with("/assets/") {
            "no-cache"
        } else {
            // Vite filenames under /assets include a content hash, so they can
            // be cached permanently without serving stale code after upgrades.
            "public, max-age=31536000, immutable"
        };
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, asset.content_type)
        .header(header::CONTENT_LENGTH, content_length)
        .header(header::CACHE_CONTROL, cache_control)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(csp_header) = asset.csp_header {
        builder = builder.header(header::CONTENT_SECURITY_POLICY, csp_header);
    }
    let body = if is_head {
        axum::body::Body::empty()
    } else {
        axum::body::Body::from(asset.bytes)
    };
    builder
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Bind `0.0.0.0:preferred` for the LAN listener, falling back through a bounded
/// port scan to an ephemeral port. Delegates to the shared `bind_with_fallback`
/// so the desktop LAN listener, `nomifun-web`, and the `nomicore` bin fail over
/// identically.
async fn bind_lan(preferred: u16) -> Result<(u16, TcpListener)> {
    crate::bootstrap::bind_with_fallback(IpAddr::V4(Ipv4Addr::UNSPECIFIED), preferred).await
}

/// Resolve the persisted admin identity from the DB: `(username, password_set)`.
///
/// `password_set` is true when the installation owner has a non-empty `password_hash`
/// (i.e. a real credential exists). Falls back to `("admin", false)` on any DB
/// error or missing row, so callers always get a displayable username.
async fn resolve_admin(user_repo: &dyn IUserRepository) -> (String, bool) {
    match user_repo.get_system_user().await {
        Ok(Some(u)) => {
            let name = if u.username.is_empty() { "admin".to_string() } else { u.username };
            (name, !u.password_hash.trim().is_empty())
        }
        _ => ("admin".to_string(), false),
    }
}

/// Reverse-proxy a SPA request to the vite dev server (DEV only) so remote
/// browsers receive the exact live frontend the desktop webview loads — instead
/// of a stale bundled `ui/dist`. Only reached for paths the backend router did
/// not handle (non-`/api` SPA assets); vite's own SPA fallback serves
/// `index.html` for client routes.
async fn dev_spa_proxy(dev_url: &str, req: Request) -> Response {
    let path_q = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let target = format!("{dev_url}{path_q}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    match client.get(&target).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ctype = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_owned());
            let body = resp.bytes().await.unwrap_or_default();
            let mut builder = Response::builder().status(status);
            if let Some(ct) = ctype {
                builder = builder.header(header::CONTENT_TYPE, ct);
            }
            builder
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(e) => {
            tracing::warn!(error = %e, %target, "dev SPA proxy to vite failed");
            (StatusCode::BAD_GATEWAY, "dev frontend (vite) not reachable").into_response()
        }
    }
}

/// Reject requests whose `Host` (or, if present, `Origin`) is a DNS name rather
/// than an IP literal or `localhost`. This blocks DNS-rebinding against the
/// LAN-exposed server while permitting all IP-based access (the URL/QR the app
/// advertises is always IP-based). Only applied to the LAN listener.
async fn host_guard_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    let headers = request.headers();
    if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok())
        && !host_is_ip_or_localhost(host)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        && !origin_is_ip_or_localhost(origin)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(next.run(request).await)
}

/// True if `host` (possibly `host:port`) is an IP literal or `localhost`.
fn host_is_ip_or_localhost(host: &str) -> bool {
    let bare = strip_port(host);
    bare.eq_ignore_ascii_case("localhost") || bare.parse::<IpAddr>().is_ok()
}

fn origin_is_ip_or_localhost(origin: &str) -> bool {
    // origin = scheme://host[:port]
    let after_scheme = origin.split("://").nth(1).unwrap_or(origin);
    host_is_ip_or_localhost(after_scheme)
}

/// Strip a trailing `:port` from a host, handling bracketed IPv6 (`[::1]:80`).
fn strip_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        // [ipv6]:port → ipv6
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        Some((h, _)) => h,
        None => host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_guard_allows_ip_and_localhost() {
        assert!(host_is_ip_or_localhost("127.0.0.1"));
        assert!(host_is_ip_or_localhost("127.0.0.1:25808"));
        assert!(host_is_ip_or_localhost("192.168.1.5:25808"));
        assert!(host_is_ip_or_localhost("localhost:25808"));
        assert!(host_is_ip_or_localhost("[::1]:25808"));
    }

    #[test]
    fn host_guard_rejects_domain_names() {
        assert!(!host_is_ip_or_localhost("evil.com"));
        assert!(!host_is_ip_or_localhost("attacker.example:25808"));
        assert!(!host_is_ip_or_localhost("nomi.local"));
    }

    #[test]
    fn origin_shape_check() {
        assert!(origin_is_ip_or_localhost("http://192.168.1.5:25808"));
        assert!(!origin_is_ip_or_localhost("http://evil.com"));
    }

    #[test]
    fn embedded_webui_source_matches_tauri_fallbacks_without_copying_bytes() {
        let source = WebUiAssetSource::new([
            (
                "index.html",
                WebUiAsset::new(b"index".to_vec(), "text/html"),
            ),
            (
                "assets/app.js",
                WebUiAsset::new(b"script".to_vec(), "text/javascript"),
            ),
            (
                "settings.html",
                WebUiAsset::new(b"settings".to_vec(), "text/html"),
            ),
            (
                "docs/index.html",
                WebUiAsset::new(b"docs".to_vec(), "text/html"),
            ),
        ]);

        let exact = source.resolve("/assets/app%2Ejs").expect("encoded exact asset");
        let exact_again = source.resolve("/assets/app.js").expect("exact asset");
        let windows_style = source
            .resolve("/assets%5Capp.js")
            .expect("encoded Windows separator");
        assert_eq!(exact.bytes, Bytes::from_static(b"script"));
        assert_eq!(windows_style.bytes, Bytes::from_static(b"script"));
        assert_eq!(
            exact.bytes.as_ptr(),
            exact_again.bytes.as_ptr(),
            "asset clones must share storage"
        );
        assert_eq!(
            source.resolve("/settings").unwrap().bytes,
            Bytes::from_static(b"settings")
        );
        assert_eq!(
            source.resolve("/docs").unwrap().bytes,
            Bytes::from_static(b"docs")
        );
        assert_eq!(
            source.resolve("/unknown/client-route").unwrap().bytes,
            Bytes::from_static(b"index")
        );
    }

    #[test]
    fn requested_listener_stop_has_an_immutable_terminal_outcome() {
        let termination = ListenerTermination::new();
        termination.request_stop();

        assert_eq!(
            termination.complete(Ok(())),
            ListenerCompletion::RequestedStop
        );
        assert_eq!(
            termination.complete(Err("late fixture failure".to_owned())),
            ListenerCompletion::RequestedStop
        );
    }

    #[test]
    fn late_stop_request_does_not_mask_listener_failure() {
        let termination = ListenerTermination::new();

        assert_eq!(
            termination.complete(Err("fixture listener failure".to_owned())),
            ListenerCompletion::UnexpectedExit("fixture listener failure".to_owned())
        );
        termination.request_stop();
        assert_eq!(
            termination.snapshot(),
            ListenerCompletion::UnexpectedExit("fixture listener failure".to_owned())
        );
    }

    #[test]
    fn listener_error_wins_over_an_in_flight_stop_request() {
        let termination = ListenerTermination::new();
        termination.request_stop();

        assert_eq!(
            termination.complete(Err("fixture listener failure".to_owned())),
            ListenerCompletion::UnexpectedExit("fixture listener failure".to_owned())
        );
    }

    #[tokio::test]
    async fn listener_wait_reports_unexpected_exit_as_terminal_complete() {
        let termination = ListenerTermination::new();
        let completion = termination.subscribe();
        termination.complete(Err("fixture listener failure".to_owned()));

        // F15: the immutable UnexpectedExit state proves the listener is
        // already stopped. The waiter must resolve (letting shutdown_all
        // proceed and close the database) instead of failing forever.
        let state = DesktopServer::wait_for_listener_completion(completion, "LAN")
            .await
            .expect("a terminal state must resolve the waiter");
        assert_eq!(
            state,
            ListenerCompletion::UnexpectedExit("fixture listener failure".to_owned())
        );
    }

    #[tokio::test]
    async fn listener_wait_resolves_requested_stop() {
        let termination = ListenerTermination::new();
        let completion = termination.subscribe();
        termination.request_stop();
        termination.complete(Ok(()));

        let state = DesktopServer::wait_for_listener_completion(completion, "LAN")
            .await
            .expect("requested stop must resolve the waiter");
        assert_eq!(state, ListenerCompletion::RequestedStop);
    }

    #[test]
    fn only_the_first_fatal_listener_failure_is_reported() {
        let fatal_reported = AtomicBool::new(false);
        assert!(claim_first_fatal_report(&fatal_reported));
        assert!(!claim_first_fatal_report(&fatal_reported));
    }

    #[test]
    fn listener_failure_preserves_cleanup_failure_context() {
        let message = merge_listener_failure_with_cleanup(
            "desktop loopback server exited unexpectedly: fixture listener error".to_string(),
            Err(anyhow::anyhow!("fixture cleanup error")),
        );

        assert!(message.contains("fixture listener error"));
        assert!(message.contains("fixture cleanup error"));
    }

    #[test]
    fn failed_lan_stop_keeps_running_status_and_reports_the_error() {
        let previous = WebUiStatus {
            running: true,
            port: 25808,
            allow_remote: true,
            local_url: "http://localhost:12345".to_owned(),
            network_url: Some("http://192.168.1.10:25808".to_owned()),
            network_urls: vec!["http://192.168.1.10:25808".to_owned()],
            lan_ip: Some("192.168.1.10".to_owned()),
            admin_username: "old-admin".to_owned(),
            password_set: false,
            initial_password: Some("one-time".to_owned()),
            error: None,
        };

        let status = stop_lan_status(
            previous,
            "admin".to_owned(),
            true,
            LanStopOutcome::Failed(anyhow::anyhow!("fixture listener timeout")),
        );

        assert!(status.running);
        assert_eq!(status.port, 25808);
        assert!(status.allow_remote);
        assert_eq!(
            status.network_url.as_deref(),
            Some("http://192.168.1.10:25808")
        );
        assert_eq!(status.admin_username, "admin");
        assert!(status.password_set);
        assert!(status.initial_password.is_none());
        assert!(
            status
                .error
                .as_deref()
                .is_some_and(|error| error.contains("fixture listener timeout"))
        );
    }

    #[test]
    fn confirmed_lan_stop_reports_stopped_status() {
        let previous = WebUiStatus {
            running: true,
            port: 25808,
            allow_remote: true,
            local_url: "http://localhost:12345".to_owned(),
            ..Default::default()
        };

        let status = stop_lan_status(previous, "admin".to_owned(), true, LanStopOutcome::Stopped);

        assert!(!status.running);
        assert_eq!(status.port, 0);
        assert!(!status.allow_remote);
        assert_eq!(status.local_url, "http://localhost:12345");
        assert_eq!(status.admin_username, "admin");
        assert!(status.password_set);
        assert!(status.error.is_none());
    }

    #[test]
    fn lan_stop_after_unexpected_exit_reports_stopped_with_error() {
        let previous = WebUiStatus {
            running: true,
            port: 25808,
            allow_remote: true,
            local_url: "http://localhost:12345".to_owned(),
            ..Default::default()
        };

        let status = stop_lan_status(
            previous,
            "admin".to_owned(),
            true,
            LanStopOutcome::StoppedAfterUnexpectedExit("fixture accept failure".to_owned()),
        );

        // F31: the listener is positively down — never report "still serving",
        // and never retain state that would block a later start_lan.
        assert!(!status.running);
        assert_eq!(status.port, 0);
        assert!(!status.allow_remote);
        assert_eq!(status.local_url, "http://localhost:12345");
        assert!(
            status
                .error
                .as_deref()
                .is_some_and(|error| error.contains("fixture accept failure"))
        );
    }

    #[tokio::test]
    async fn shutdown_failure_is_not_cached_and_success_is_idempotent() {
        let success = OnceCell::new();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first_attempts = Arc::clone(&attempts);
        let first = run_shutdown_once(&success, || async move {
            first_attempts.fetch_add(1, Ordering::AcqRel);
            Err(anyhow::anyhow!("transient cleanup failure"))
        })
        .await;
        assert!(first.is_err());
        assert!(success.get().is_none());

        let second_attempts = Arc::clone(&attempts);
        let second = run_shutdown_once(&success, || async move {
            second_attempts.fetch_add(1, Ordering::AcqRel);
            Ok(())
        })
        .await;
        assert!(second.is_ok());
        assert!(success.get().is_some());

        // A successful shutdown is idempotent: the initializer is not run again.
        let third_attempts = Arc::clone(&attempts);
        let third = run_shutdown_once(&success, || async move {
            third_attempts.fetch_add(1, Ordering::AcqRel);
            Ok(())
        })
        .await;
        assert!(third.is_ok());
        assert_eq!(attempts.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn cleanup_failure_leaves_database_open_for_retry() {
        let close_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let close_calls_for_fn = Arc::clone(&close_calls);
        let result = close_database_after_cleanup(
            vec!["terminal cleanup failed: fixture".to_owned()],
            move || async move {
                close_calls_for_fn.fetch_add(1, Ordering::AcqRel);
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(close_calls.load(Ordering::Acquire), 0);
    }

    #[test]
    fn lan_restore_requested_only_for_strict_json_true() {
        assert!(lan_restore_requested(Some("true")));
        assert!(lan_restore_requested(Some(" true\n")));
    }

    #[test]
    fn lan_restore_is_refused_for_every_ambiguous_stored_value() {
        // Exposing 0.0.0.0 is the irreversible direction, so anything that is
        // not a strict JSON `true` must read as "off".
        for stored in [
            None,
            Some(""),
            Some("   "),
            Some("false"),
            // JSON *string* "true": what a naive string comparison would accept.
            Some("\"true\""),
            Some("1"),
            Some("null"),
            Some("TRUE"),
            Some("on"),
            Some("{\"enabled\":true}"),
            Some("not json at all"),
        ] {
            assert!(
                !lan_restore_requested(stored),
                "stored value {stored:?} must not auto-expose the LAN listener"
            );
        }
    }
}
