//! Relay pairing for the native Desktop shell.
//!
//! The relay console gives Desktop a short-lived, one-shot envelope.  This
//! module is deliberately the only place that handles that bearer:
//! - parse and validate the envelope without accepting alternate credential
//!   shapes;
//! - bootstrap the fixed Desktop backend through the Relay;
//! - install/verify the managed nfagent and run it with the shared
//!   process-runtime owner;
//! - keep public restart metadata separate from OS-user protected secrets;
//! - mint the final one-shot `nomifun://pair` device ticket only on explicit
//!   request; connecting or restarting the relay never creates one.

use std::fs;
use std::future::Future;
use std::io::{self, Read};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep};
use url::Url;

use nomifun_app::DesktopServer;

const ENVELOPE_PREFIX: &str = "nomifun-relay-pair:v1:";
const ENVELOPE_V2_PREFIX: &str = "nomifun-relay-pair:v2:";
const STATE_VERSION: u8 = 2;
const STATE_FILE_NAME: &str = "relay-pairing.json";
const AGENT_STATE_DIR_NAME: &str = "relay-agent";
const DESKTOP_BACKEND_ADDR: &str = "127.0.0.1:25808";
const REQUIRED_DESKTOP_PORT: u16 = 25808;
const V2_DESKTOP_BACKEND_ADDR: &str = "127.0.0.1:25811";
const V2_DESKTOP_PORT: u16 = 25811;
const AGENT_POLL_TIMEOUT: Duration = Duration::from_secs(45);
const RESTORE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const HEALTH_REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const SHUTDOWN_OPERATION_GRACE: Duration = Duration::from_secs(5);
const NFAGENT_RUNTIME_DIR_NAME: &str = "runtime";
const NFAGENT_CACHE_DIR_NAME: &str = "nfagent";
const NFAGENT_MAX_DOWNLOAD_BYTES: u64 = 32 * 1024 * 1024;
const NFAGENT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const NFAGENT_RUNTIME_VERSION_MAX_LEN: usize = 64;
const NFAGENT_FILE_NAME_MAX_LEN: usize = 128;
const NFAGENT_MANIFEST: &str = include_str!("../nfagent-runtime.json");

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingEnvelope {
    bootstrap_url: String,
    invite: String,
    expires_at: String,
    #[serde(skip)]
    protocol_version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPairing {
    version: u8,
    #[serde(default = "legacy_protocol_version")]
    protocol_version: u8,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    control_url: String,
    #[serde(default)]
    l2_port: u16,
    relay: String,
    pin: String,
    business_url: String,
    #[serde(default)]
    probe_url: String,
    invite_id: String,
    tunnel_id: String,
    tunnel_slug: String,
    agent_state_dir: String,
}

fn legacy_protocol_version() -> u8 { 1 }
fn enabled_by_default() -> bool { true }

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingBootstrap {
    bootstrap_url: String,
    invite: String,
    expires_at: String,
    attempt_id: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayControlCredential {
    control_url: String,
    control_token: String,
    #[serde(default)]
    enrol_token: Option<String>,
    #[serde(default)]
    enrol_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayHealth {
    pub backend_reachable: bool,
    pub tunnel_reachable: bool,
    pub checked_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayPairingStatus {
    pub state: &'static str,
    pub error: Option<String>,
    pub relay: Option<String>,
    pub business_url: Option<String>,
    pub invite_id: Option<String>,
    pub tunnel_id: Option<String>,
    pub tunnel_slug: Option<String>,
    pub agent_state_dir: Option<String>,
    pub agent_pid: Option<u32>,
    pub runtime_available: bool,
    #[serde(rename = "pairUrl")]
    pub pairing_url: Option<String>,
    pub qr_expires_at_ms: Option<u64>,
    pub webui_port: Option<u16>,
    pub protocol_version: u8,
    pub enabled: bool,
    pub health: RelayHealth,
    pub qr_state: &'static str,
    pub ticket_id: Option<String>,
    pub error_code: Option<&'static str>,
    // `error_code` names the current state condition; `action_error_code`
    // records why the last lifecycle operation failed.
    pub action_error_code: Option<&'static str>,
    pub operation: Option<&'static str>,
    pub pending_bootstrap: bool,
    pub relay_state: Option<String>,
    pub relay_reason: Option<String>,
}

impl Default for RelayPairingStatus {
    fn default() -> Self {
        Self {
            state: "disconnected",
            error: None,
            relay: None,
            business_url: None,
            invite_id: None,
            tunnel_id: None,
            tunnel_slug: None,
            agent_state_dir: None,
            agent_pid: None,
            runtime_available: false,
            pairing_url: None,
            qr_expires_at_ms: None,
            webui_port: None,
            protocol_version: 2,
            enabled: false,
            health: RelayHealth::default(),
            qr_state: "absent",
            ticket_id: None,
            error_code: None,
            action_error_code: None,
            operation: None,
            pending_bootstrap: false,
            relay_state: None,
            relay_reason: None,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapResponse {
    #[serde(default = "legacy_protocol_version")]
    protocol_version: u8,
    invite_id: String,
    relay: String,
    business_url: String,
    #[serde(default)]
    probe_url: String,
    pin: String,
    enrol_token: String,
    enrol_expires_at: DateTime<Utc>,
    // The Relay returns a convenience command for manual installs as part of
    // the strict response contract. Desktop intentionally ignores it so the
    // embedded command is never logged or persisted. A separate, protected
    // short-lived enrol credential enables exact recovery after interruption.
    #[serde(rename = "install_cmd")]
    _install_cmd: String,
    tunnel: Value,
    #[serde(default)]
    control_url: String,
    #[serde(default)]
    control_token: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    l2_port: Option<u16>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct QrTokenResponse {
    ticket_id: String,
    qr_token: String,
    expires_at_ms: u64,
}

#[derive(Deserialize)]
struct TicketStatusEnvelope {
    success: bool,
    data: Option<TicketStatus>,
}

#[derive(Deserialize)]
struct TicketStatus { state: String }

#[derive(Deserialize)]
struct RelayConnectionProjection {
    protocol_version: u8,
    invite_id: String,
    tunnel_id: String,
    state: String,
    reason: String,
    revoked: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct QrTokenEnvelope {
    success: bool,
    data: Option<QrTokenResponse>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NfagentRuntimeManifest {
    version: String,
    assets: std::collections::HashMap<String, NfagentRuntimeAsset>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NfagentRuntimeAsset {
    url: String,
    sha256: String,
}

#[derive(Debug, Clone)]
struct NfagentDownloadSpec {
    version: String,
    file_name: String,
    url: Url,
    sha256: String,
}

#[derive(Default)]
struct PairingRuntime {
    persisted: Option<PersistedPairing>,
    process: Option<ManagedChildProcess>,
    status: RelayPairingStatus,
    generation: u64,
    health_checked_at: Option<Instant>,
    state_unreadable: bool,
}

/// Native pairing state.  The process slot is polled rather than awaited while
/// holding the mutex, so a stop/restart command can always take ownership and
/// perform the managed tree cleanup.
pub struct RelayPairingManager {
    data_dir: PathBuf,
    state_path: PathBuf,
    agent_state_dir: PathBuf,
    client: reqwest::Client,
    local_client: reqwest::Client,
    download_client: reqwest::Client,
    nfagent_install_lock: Mutex<()>,
    runtime: Mutex<PairingRuntime>,
    operation_gate: Mutex<()>,
    child_gate: Mutex<()>,
    closing: AtomicBool,
    shutdown_cancel: tokio_util::sync::CancellationToken,
    op_interrupt: std::sync::Mutex<tokio_util::sync::CancellationToken>,
    pending_path: PathBuf,
    control_path: PathBuf,
}

impl RelayPairingManager {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&data_dir)
            .with_context(|| format!("create Desktop data directory {}", data_dir.display()))?;
        let state_path = data_dir.join(STATE_FILE_NAME);
        let agent_state_dir = data_dir.join(AGENT_STATE_DIR_NAME);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .context("build Relay pairing HTTP client")?;
        let download_client = reqwest::Client::builder()
            .redirect(https_redirect_policy())
            .connect_timeout(Duration::from_secs(10))
            .timeout(NFAGENT_DOWNLOAD_TIMEOUT)
            .no_gzip()
            .user_agent(concat!(
                "NomiFun/",
                env!("CARGO_PKG_VERSION"),
                " nfagent-runtime"
            ))
            .build()
            .context("build nfagent download HTTP client")?;
        let local_client = reqwest::Client::builder().no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2)).timeout(Duration::from_secs(5)).build()?;

        let pending_path = data_dir.join("relay-bootstrap.pending.secret");
        let control_path = data_dir.join("relay-control.secret");
        let mut state_unreadable = false;
        // Unreadable protected state must not block Desktop startup: it is
        // isolated as a typed state error and only `forget_local` proceeds.
        let persisted = match load_persisted(&state_path) {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(%error, "Relay pairing state is unreadable at startup");
                state_unreadable = true;
                None
            }
        };
        let mut status = RelayPairingStatus::default();
        status.runtime_available = existing_nfagent_path(&data_dir).is_ok_and(|path| path.is_some());
        if let Some(state) = &persisted {
            status.state = if state.enabled { "connecting" } else { "paused" };
            status.enabled = state.enabled;
            status.protocol_version = state.protocol_version;
            status.relay = Some(state.relay.clone());
            status.business_url = Some(state.business_url.clone());
            status.invite_id = Some(state.invite_id.clone());
            status.tunnel_id = Some(state.tunnel_id.clone());
            status.tunnel_slug = Some(state.tunnel_slug.clone());
            status.agent_state_dir = Some(agent_state_dir.display().to_string());
            if state.protocol_version == 2 {
                if let Err(error) = read_secret_json::<RelayControlCredential>(&control_path) {
                    tracing::warn!(%error, "Relay control credential is unreadable at startup");
                    state_unreadable = true;
                }
            }
        }
        if pending_path.is_file() {
            match read_secret_json::<PendingBootstrap>(&pending_path) {
                Ok(pending) => {
                    if persisted.is_none() {
                        status.pending_bootstrap = true;
                        status.enabled = pending.enabled;
                        status.state = if pending.enabled { "connecting" } else { "paused" };
                        status.error_code = Some("RELAY_BOOTSTRAP_UNCONFIRMED");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "Relay pending bootstrap secret is unreadable at startup");
                    state_unreadable = true;
                }
            }
        }
        if state_unreadable {
            status.state = "error";
            status.error_code = Some("RELAY_STATE_UNREADABLE");
            status.enabled = false;
            status.pending_bootstrap = pending_path.is_file();
        }

        Ok(Self {
            data_dir: data_dir.clone(),
            state_path,
            agent_state_dir,
            client,
            local_client,
            download_client,
            nfagent_install_lock: Mutex::new(()),
            runtime: Mutex::new(PairingRuntime {
                persisted,
                process: None,
                status,
                generation: 0,
                health_checked_at: None,
                state_unreadable,
            }),
            operation_gate: Mutex::new(()),
            child_gate: Mutex::new(()),
            closing: AtomicBool::new(false),
            shutdown_cancel: tokio_util::sync::CancellationToken::new(),
            op_interrupt: std::sync::Mutex::new(tokio_util::sync::CancellationToken::new()),
            pending_path,
            control_path,
        })
    }

    /// Restore a previously paired agent after the embedded backend is ready.
    /// This is intentionally best-effort and non-blocking from the desktop
    /// startup supervisor: nfagent itself has a reconnect loop.
    pub async fn restore(self: Arc<Self>, server: Arc<DesktopServer>) {
        if self.runtime.lock().await.state_unreadable {
            return;
        }
        let _operation = self.operation_gate.lock().await;
        self.begin_operation("restore").await;
        let result = self.restore_locked(server.clone()).await;
        if result.is_err() { self.release_idle_relay_edge(&server).await; }
        self.finish_operation(&result).await;
    }

    async fn restore_locked(self: &Arc<Self>, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        let state = {
            let guard = self.runtime.lock().await;
            guard.persisted.clone()
        };
        let Some(state) = state else {
            if self.pending_path.is_file() {
                let pending: PendingBootstrap = self.read_protected(&self.pending_path).await?;
                if !pending.enabled { return Ok(self.runtime.lock().await.status.clone()); }
                return self.bootstrap_locked(server, &pending_envelope(&pending)?).await;
            }
            return Ok(self.runtime.lock().await.status.clone());
        };
        if !state.enabled { return Ok(self.runtime.lock().await.status.clone()); }
        if state.protocol_version == 2 && self.pending_path.is_file() {
            return self.restart_locked(server).await;
        }

        let nfagent_path = self.prepare_nfagent()
            .await
            .context("恢复 Relay agent 失败")?;
        self.ensure_webui_port(&server, state.protocol_version)
            .await
            .context("Relay 配对状态已保存，但 WebUI 端口未就绪")?;

        let enrol = self.pending_enrol_token(&state).await.ok().flatten();
        self.start_agent(state.clone(), enrol, nfagent_path)
            .await
            .context("恢复 Relay agent 失败")?;
        self.wait_for_agent_ready(&server, &state)
            .await
            .context("Relay agent 正在重连")?;
        self.mark_connected(&state).await;
        Ok(self.runtime.lock().await.status.clone())
    }

    pub async fn status(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        // Availability is a read-only local check even before any pairing.
        // It never repairs the cache or contacts a download endpoint.
        let runtime_available = existing_nfagent_path(&self.data_dir).is_ok_and(|path| path.is_some());
        let (persisted, mut status) = {
            let mut guard = self.runtime.lock().await;
            guard.status.runtime_available = runtime_available;
            (guard.persisted.clone(), guard.status.clone())
        };
        let Some(persisted) = persisted else {
            return Ok(status);
        };

        let Ok(_operation) = self.operation_gate.try_lock() else { return Ok(status); };

        if status.state == "connecting" || status.state == "connected" || status.state == "error" {
            let pid = self
                .runtime
                .lock()
                .await
                .process
                .as_ref()
                .and_then(ManagedChildProcess::id);
            status.agent_pid = pid;
        }

        if persisted.enabled && status.agent_pid.is_some() && status.error_code != Some("RELAY_AGENT_CLEANUP_UNCONFIRMED") {
            let stale = self
                .runtime
                .lock()
                .await
                .health_checked_at
                .is_none_or(|checked| checked.elapsed() >= HEALTH_REFRESH_INTERVAL);
            if stale {
                let health = match self.until_shutdown(async { Ok(self.check_health(&server, &persisted).await) }).await {
                    Ok(health) => health,
                    Err(_) => return Ok(self.runtime.lock().await.status.clone()),
                };
                let mut guard = self.runtime.lock().await;
                guard.health_checked_at = Some(Instant::now());
                apply_health(&mut guard.status, health);
                status = guard.status.clone();
            }
        }
        if status.state == "connected" {
            let now_ms = Utc::now().timestamp_millis().max(0) as u64;
            if status.qr_expires_at_ms.is_some_and(|expires| expires <= now_ms) {
                self.runtime.lock().await.status.qr_state = "expired";
            } else if let Some(ticket_id) = status.ticket_id.as_deref() {
                let endpoint = format!("http://127.0.0.1:{}/api/connections/pairing/{ticket_id}", server.loopback_port());
                let fetched = self.until_shutdown(async {
                    Ok(self
                        .local_client
                        .get(endpoint)
                        .header("x-nomi-local-trust", server.local_trust_secret())
                        .send()
                        .await)
                }).await;
                match fetched {
                    Ok(Ok(response)) => {
                        if response.status() == StatusCode::NOT_FOUND {
                            // A superseded or cleaned-up code is simply gone.
                            let mut guard = self.runtime.lock().await;
                            guard.status.pairing_url = None;
                            guard.status.ticket_id = None;
                            guard.status.qr_expires_at_ms = None;
                            guard.status.qr_state = "absent";
                        } else if let Ok(ticket) = response.json::<TicketStatusEnvelope>().await {
                            if ticket.success {
                                let terminal = ticket.data.and_then(|data| match data.state.as_str() {
                                    "used" => Some("used"),
                                    "failed" => Some("failed"),
                                    _ => None,
                                });
                                if let Some(state) = terminal {
                                    let mut guard = self.runtime.lock().await;
                                    guard.status.qr_state = state;
                                    guard.status.pairing_url = None;
                                }
                            }
                        }
                    }
                    Ok(Err(_)) => {}
                    Err(_) => return Ok(self.runtime.lock().await.status.clone()),
                }
            }
            status = self.runtime.lock().await.status.clone();
        }
        Ok(status)
    }

    /// Rejects an envelope before it becomes a lifecycle operation: the paired
    /// state is never touched, the typed action code explains the refusal, and
    /// `status.state` stays whatever it was.
    async fn precheck_bootstrap(&self, raw: &str) -> Result<(), (&'static str, anyhow::Error)> {
        if self.runtime.lock().await.state_unreadable {
            return Err(("RELAY_STATE_UNREADABLE", anyhow::anyhow!("本机 Relay 配置不可读；请使用“仅在本机忘记”后重新连接")));
        }
        {
            let guard = self.runtime.lock().await;
            if guard.persisted.is_some() || guard.process.is_some() {
                return Err(("RELAY_ALREADY_PAIRED", anyhow::anyhow!("Desktop 已存在 Relay 配对；请先断开现有配对")));
            }
        }
        let envelope = parse_pairing_envelope(raw).map_err(|error| ("RELAY_ENVELOPE_INVALID", error))?;
        let expires = envelope_expires_at(&envelope).map_err(|error| ("RELAY_ENVELOPE_INVALID", error))?;
        if self.pending_path.is_file() {
            let pending: PendingBootstrap = read_secret_json(&self.pending_path)
                .map_err(|error| ("RELAY_STATE_UNREADABLE", error))?;
            if envelope.protocol_version != 2 || pending.bootstrap_url != envelope.bootstrap_url || pending.invite != envelope.invite {
                return Err(("RELAY_PENDING_EXISTS", anyhow::anyhow!("存在尚未确认的 Relay 配对请求，请重试原请求或解除后重新连接")));
            }
        } else if expires <= Utc::now() {
            return Err(("RELAY_INVITE_EXPIRED", anyhow::anyhow!("Relay pairing 串已过期")));
        }
        Ok(())
    }

    /// A failed lifecycle action that left no managed agent no longer needs
    /// the dedicated relay edge listener.
    async fn release_idle_relay_edge(&self, server: &Arc<DesktopServer>) {
        if self.runtime.lock().await.process.is_none() {
            if let Err(error) = server.stop_relay().await {
                tracing::warn!(%error, "释放空闲 Relay edge listener 失败");
            }
        }
    }

    pub async fn bootstrap(
        self: &Arc<Self>,
        server: Arc<DesktopServer>,
        raw_envelope: &str,
    ) -> Result<RelayPairingStatus> {
        let _operation = self.operation_gate.lock().await;
        if let Err((code, error)) = self.precheck_bootstrap(raw_envelope).await {
            self.runtime.lock().await.status.action_error_code = Some(code);
            return Err(error);
        }
        self.begin_operation("bootstrap").await;
        let result = self.bootstrap_locked(server.clone(), raw_envelope).await;
        if result.is_err() { self.release_idle_relay_edge(&server).await; }
        self.finish_operation(&result).await;
        match result { Ok(_) => Ok(self.runtime.lock().await.status.clone()), Err(error) => Err(error) }
    }

    async fn bootstrap_locked(self: &Arc<Self>, server: Arc<DesktopServer>, raw_envelope: &str) -> Result<RelayPairingStatus> {
        anyhow::ensure!(!self.closing.load(Ordering::Acquire), "Desktop 正在关闭，不能进行 Relay 配对");
        self.refuse_unreadable_state().await?;
        let envelope = parse_pairing_envelope(raw_envelope)?;
        if self.pending_path.is_file() {
            let pending: PendingBootstrap = self.read_protected(&self.pending_path).await?;
            if envelope.protocol_version != 2 || pending.bootstrap_url != envelope.bootstrap_url || pending.invite != envelope.invite {
                anyhow::bail!("存在尚未确认的 Relay 配对请求，请重试原请求或解除后重新连接");
            }
        } else {
            validate_envelope_expiry(&envelope)?;
        }

        {
            let guard = self.runtime.lock().await;
            if guard.persisted.is_some() || guard.process.is_some() {
                anyhow::bail!("Desktop 已存在 Relay 配对；请先断开现有配对");
            }
        }

        // Prepare the verified runtime before exchanging the one-shot invite;
        // a missing runtime or failed download must never consume an invite.
        let nfagent_path = self.prepare_nfagent().await?;
        self.ensure_webui_port(&server, envelope.protocol_version).await?;
        prepare_agent_state_dir(&self.agent_state_dir)?;

        self.set_status(RelayPairingStatus {
            state: "connecting",
            error: None,
            webui_port: Some(desktop_port(envelope.protocol_version)),
            enabled: true,
            protocol_version: envelope.protocol_version,
            operation: Some("bootstrap"),
            ..Default::default()
        })
        .await;
        self.runtime.lock().await.health_checked_at = None;

        let response = self.interruptible(self.exchange_bootstrap(&envelope)).await?;
        validate_bootstrap_response(&response)?;
        if response.protocol_version != envelope.protocol_version {
            anyhow::bail!("Relay bootstrap protocol version mismatch");
        }
        if response.enrol_expires_at <= Utc::now() {
            anyhow::bail!("Relay 返回的 agent 入网令牌已过期");
        }

        let persisted = PersistedPairing {
            version: STATE_VERSION,
            protocol_version: response.protocol_version,
            enabled: true,
            control_url: response.control_url.clone(),
            l2_port: response.l2_port.unwrap_or(0),
            relay: response.relay.clone(),
            pin: response.pin.clone(),
            business_url: response.business_url.clone(),
            probe_url: response.probe_url.clone(),
            invite_id: response.invite_id.clone(),
            tunnel_id: tunnel_string(&response.tunnel, "id")?,
            tunnel_slug: tunnel_string(&response.tunnel, "slug")?,
            agent_state_dir: AGENT_STATE_DIR_NAME.to_owned(),
        };
        if persisted.protocol_version == 2 {
            write_secret_json(&self.control_path, &RelayControlCredential {
                control_url: response.control_url,
                control_token: response.control_token,
                enrol_token: Some(response.enrol_token.clone()),
                enrol_expires_at: Some(response.enrol_expires_at),
            })?;
        }
        // Publish restart authority before starting the agent. An uncertain
        // startup is recoverable with the same pairing rather than orphaning it.
        write_persisted(&self.state_path, &persisted)?;
        self.runtime.lock().await.persisted = Some(persisted.clone());
        self.set_status(status_from_persisted(&persisted, "connecting", None))
            .await;
        {
            let mut guard = self.runtime.lock().await;
            guard.status.pending_bootstrap = self.pending_path.is_file();
            guard.status.relay_state = Some(response.state.clone());
            guard.status.relay_reason = Some(response.reason.clone());
        }
        if persisted.protocol_version == 2 && response.state == "error" {
            self.set_action_error("RELAY_ENTRY_NOT_READY").await;
            anyhow::bail!("Relay 入口配置未就绪：{}；配对已保留，可重试连接或解除", response.reason);
        }
        if let Err(error) = self
            .start_agent(
                persisted.clone(),
                Some(response.enrol_token),
                nfagent_path,
            )
            .await
        {
            self.set_error(format!(
                "Relay 配对已保存，但 agent 启动失败：{error:#}。可重试连接或解除此配对。"
            ))
            .await;
            return Err(error);
        }

        if let Err(error) = self.wait_for_agent_ready(&server, &persisted).await {
            let _ = self.stop_agent().await;
            self.set_error(format!(
                "Relay 配对已保存，但 agent 未能上线：{error}。可重试连接或解除此配对。"
            ))
            .await;
            anyhow::bail!(
                "Relay bootstrap succeeded but nfagent did not become ready: {error:#}"
            );
        }

        self.mark_connected(&persisted).await;
        if self.pending_path.is_file() { fs::remove_file(&self.pending_path)?; }
        self.runtime.lock().await.status.pending_bootstrap = false;
        write_persisted(&self.state_path, &persisted)?;
        {
            let mut guard = self.runtime.lock().await;
            guard.persisted = Some(persisted.clone());
        }

        let status = {
            let guard = self.runtime.lock().await;
            guard.status.clone()
        };
        self.set_status(status.clone()).await;
        Ok(status)
    }

    pub async fn restart(
        self: &Arc<Self>,
        server: Arc<DesktopServer>,
    ) -> Result<RelayPairingStatus> {
        self.interrupt_waits();
        let _operation = self.operation_gate.lock().await;
        self.begin_operation("restart").await;
        let result = self.restart_locked(server.clone()).await;
        if result.is_err() { self.release_idle_relay_edge(&server).await; }
        self.finish_operation(&result).await;
        match result { Ok(_) => Ok(self.runtime.lock().await.status.clone()), Err(error) => Err(error) }
    }

    async fn restart_locked(self: &Arc<Self>, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        anyhow::ensure!(!self.closing.load(Ordering::Acquire), "Desktop 正在关闭，不能恢复 Relay 连接");
        self.refuse_unreadable_state().await?;
        if self.runtime.lock().await.persisted.is_none() && self.pending_path.is_file() {
            let mut pending: PendingBootstrap = self.read_protected(&self.pending_path).await?;
            // An explicit Reconnect re-arms the durable enable intent so the
            // confirmed pairing also survives the next reboot.
            pending.enabled = true;
            write_secret_json(&self.pending_path, &pending)?;
            self.runtime.lock().await.status.enabled = true;
            return self.bootstrap_locked(server, &pending_envelope(&pending)?).await;
        }
        let mut persisted = {
            let guard = self.runtime.lock().await;
            guard
                .persisted
                .clone()
                .context("没有可恢复的 Relay 配对状态")?
        };
        // Prepare the verified runtime before retrying the bootstrap exchange
        // or starting the saved connection.
        let nfagent_path = self.prepare_nfagent().await?;
        if self.pending_path.is_file() && persisted.protocol_version == 2 && !self.agent_state_dir.join("credential.json").is_file() {
            let pending: PendingBootstrap = self.read_protected(&self.pending_path).await?;
            let envelope = parse_pairing_envelope(&pending_envelope(&pending)?)?;
            let retry = self.interruptible(self.exchange_bootstrap(&envelope)).await?;
            validate_bootstrap_response(&retry)?;
            if retry.invite_id != persisted.invite_id || tunnel_string(&retry.tunnel, "id")? != persisted.tunnel_id {
                anyhow::bail!("Relay 重试回执与当前配对不匹配");
            }
            let mut guard = self.runtime.lock().await;
            guard.status.relay_state = Some(retry.state.clone());
            guard.status.relay_reason = Some(retry.reason.clone());
            drop(guard);
            if retry.state == "error" {
                self.set_action_error("RELAY_ENTRY_NOT_READY").await;
                anyhow::bail!("Relay 入口配置未就绪：{}", retry.reason);
            }
            persisted.l2_port = retry.l2_port.unwrap_or(0);
        }
        persisted.enabled = true;
        write_persisted(&self.state_path, &persisted)?;
        self.runtime.lock().await.persisted = Some(persisted.clone());
        ensure_child_path(&self.data_dir, &self.agent_state_dir)?;
        prepare_agent_state_dir(&self.agent_state_dir)?;
        if !self.agent_state_dir.join("credential.json").is_file() {
            if self.pending_enrol_token(&persisted).await?.is_none() {
                anyhow::bail!("agent 长期凭据不存在且入网邀请已失效；请解除配对后重新连接");
            }
        }
        self.ensure_webui_port(&server, persisted.protocol_version).await?;
        self.stop_agent().await?;
        let enrol = self.pending_enrol_token(&persisted).await?;
        self.start_agent(persisted.clone(), enrol, nfagent_path)
            .await?;
        if let Err(error) = self.wait_for_agent_ready(&server, &persisted).await {
            let _ = self.stop_agent().await;
            self.set_error(format!("重启 Relay agent 后未能恢复连接：{error}"))
                .await;
            anyhow::bail!("Relay agent restart did not become ready: {error:#}");
        }
        self.mark_connected(&persisted).await;
        if self.pending_path.is_file() { fs::remove_file(&self.pending_path)?; }
        self.runtime.lock().await.status.pending_bootstrap = false;
        let status = {
            let guard = self.runtime.lock().await;
            guard.status.clone()
        };
        self.set_status(status.clone()).await;
        Ok(status)
    }

    pub async fn disconnect(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        self.interrupt_waits();
        let _operation = self.operation_gate.lock().await;
        self.begin_operation("disconnect").await;
        let result = self.disconnect_locked(server).await;
        self.finish_operation(&result).await;
        match result { Ok(_) => Ok(self.runtime.lock().await.status.clone()), Err(error) => Err(error) }
    }

    async fn disconnect_locked(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        self.refuse_unreadable_state().await?;
        let state = self.runtime.lock().await.persisted.clone();
        if state.is_none() && self.pending_path.is_file() {
            // An uncertain bootstrap may already own remote resources. Recover
            // the exact receipt before forgetting its only revocation authority.
            let pending: PendingBootstrap = self.read_protected(&self.pending_path).await?;
            let mut cancel = Url::parse(&pending.bootstrap_url)?;
            cancel.set_path("/api/bootstrap/desktop/v2:cancel");
            let cancelled = self.interruptible(async {
                let response = self.client_for_url(cancel.as_str()).post(cancel.clone())
                    .bearer_auth(&pending.invite).header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(serde_json::to_vec(&serde_json::json!({"attempt_id":pending.attempt_id}))?)
                    .send().await.context("Relay 待确认配对取消未完成；保留原请求供重试")?;
                if response.status() != StatusCode::OK || response.json::<Value>().await?.get("revoked").and_then(Value::as_bool) != Some(true) {
                    anyhow::bail!("Relay 待确认配对取消未获确认；保留原请求供重试");
                }
                Ok(())
            }).await;
            if let Err(error) = cancelled {
                self.set_action_error("RELAY_REVOKE_UNCONFIRMED").await;
                return Err(error);
            }
        }
        if let Some(state) = state.as_ref().filter(|state| state.protocol_version == 2) {
            let credential: RelayControlCredential = self.read_protected(&self.control_path).await?;
            if credential.control_url != state.control_url {
                self.set_action_error("RELAY_STATE_UNREADABLE").await;
                anyhow::bail!("Relay 管理凭据与当前配对不匹配");
            }
            if let Err(error) = self.interruptible(self.revoke_control(&credential)).await {
                self.set_action_error("RELAY_REVOKE_UNCONFIRMED").await;
                return Err(error);
            }
        }
        self.clear_local_state(server).await
    }

    /// Remove local state without contacting the Relay. The Relay-side
    /// connection remains and must be revoked later in Relay Console.
    pub async fn forget_local(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        self.interrupt_waits();
        let _operation = self.operation_gate.lock().await;
        self.begin_operation("forget").await;
        let result = self.clear_local_state(server).await;
        self.finish_operation(&result).await;
        match result { Ok(_) => Ok(self.runtime.lock().await.status.clone()), Err(error) => Err(error) }
    }

    async fn clear_local_state(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        self.stop_agent().await?;
        server.stop_relay().await?;
        self.remove_local_files()?;
        let mut status = RelayPairingStatus::default();
        let mut guard = self.runtime.lock().await;
        status.runtime_available = guard.status.runtime_available;
        guard.persisted = None;
        guard.state_unreadable = false;
        guard.health_checked_at = None;
        guard.status = status.clone();
        Ok(status)
    }

    fn remove_local_files(&self) -> Result<()> {
        ensure_child_path(&self.data_dir, &self.agent_state_dir)?;
        for path in [
            self.state_path.clone(),
            self.state_path.with_extension("json.tmp"),
            self.control_path.clone(),
            self.pending_path.clone(),
        ] {
            if path.exists() {
                fs::remove_file(&path).with_context(|| {
                    format!("remove Relay local state {}", path.display())
                })?;
            }
        }
        if self.agent_state_dir.exists() {
            fs::remove_dir_all(&self.agent_state_dir).with_context(|| {
                format!("remove agent state directory {}", self.agent_state_dir.display())
            })?;
        }
        Ok(())
    }

    /// Stop the managed agent while retaining pairing metadata and
    /// `credential.json` for an explicit restart.
    pub async fn stop(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        self.interrupt_waits();
        let _operation = self.operation_gate.lock().await;
        self.begin_operation("stop").await;
        let result = self.stop_locked(server).await;
        self.finish_operation(&result).await;
        match result { Ok(_) => Ok(self.runtime.lock().await.status.clone()), Err(error) => Err(error) }
    }

    async fn stop_locked(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        self.refuse_unreadable_state().await?;
        let mut state = self.runtime.lock().await.persisted.clone();
        let has_pending = self.pending_path.is_file();
        if has_pending {
            let mut pending: PendingBootstrap = self.read_protected(&self.pending_path).await?;
            pending.enabled = false;
            write_secret_json(&self.pending_path, &pending)?;
        }
        if let Some(state) = state.as_mut() {
            state.enabled = false;
            write_persisted(&self.state_path, state)?;
            let mut guard = self.runtime.lock().await;
            guard.persisted = Some(state.clone());
            guard.status.enabled = false;
        }
        self.stop_agent().await?;
        server.stop_relay().await?;
        let mut guard = self.runtime.lock().await;
        let mut status = guard.status.clone();
        status.state = if state.is_some() || has_pending { "paused" } else { "disconnected" };
        status.enabled = false;
        status.health = RelayHealth::default();
        status.qr_state = "absent";
        status.ticket_id = None;
        status.agent_pid = None;
        status.pairing_url = None;
        status.qr_expires_at_ms = None;
        status.error = None;
        status.error_code = None;
        status.operation = None;
        guard.status = status.clone();
        guard.health_checked_at = None;
        Ok(status)
    }

    pub async fn refresh_qr(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        let _operation = self.operation_gate.lock().await;
        anyhow::ensure!(!self.closing.load(Ordering::Acquire), "Desktop 正在关闭，不能生成配对码");
        self.refuse_unreadable_state().await?;
        let state = self.runtime.lock().await.persisted.clone().context("没有 Relay 配对")?;
        if !state.enabled { anyhow::bail!("Relay 已暂停，请先恢复连接"); }
        let health = self.until_shutdown(async { Ok(self.check_health(&server, &state).await) }).await?;
        {
            let mut guard = self.runtime.lock().await;
            guard.status.health = health.clone();
            guard.health_checked_at = Some(Instant::now());
        }
        if !health.backend_reachable || !health.tunnel_reachable { anyhow::bail!("Relay 尚未接通，暂不能生成手机配对码"); }
        self.until_shutdown(self.refresh_pairing_url(&server, &state)).await?;
        Ok(self.runtime.lock().await.status.clone())
    }

    /// Host teardown preserves the durable enable intent for the next boot.
    pub async fn shutdown(&self) -> Result<()> {
        self.closing.store(true, Ordering::Release);
        self.shutdown_cancel.cancel();
        self.interrupt_waits();
        let _operation = tokio::time::timeout(SHUTDOWN_OPERATION_GRACE, self.operation_gate.lock()).await.ok();
        self.stop_agent().await
    }

    /// Begin a gated lifecycle operation with a fresh interruption token.
    async fn begin_operation(&self, name: &'static str) {
        *self.op_interrupt.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            tokio_util::sync::CancellationToken::new();
        let mut guard = self.runtime.lock().await;
        guard.status.operation = Some(name);
        guard.status.action_error_code = None;
    }

    /// Cancel the waits of the operation currently holding the gate; the next
    /// gated operation installs a fresh token in `begin_operation`.
    fn interrupt_waits(&self) {
        self.op_interrupt
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancel();
    }

    /// A wait inside a gated operation is preempted by Desktop shutdown or by
    /// a superseding connection operation.
    async fn interruptible<T>(&self, op: impl Future<Output = Result<T>>) -> Result<T> {
        let interrupted = self
            .op_interrupt
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        tokio::select! {
            biased;
            _ = self.shutdown_cancel.cancelled() => anyhow::bail!("Desktop 正在关闭，已停止等待 Relay"),
            _ = interrupted.cancelled() => anyhow::bail!("Relay 操作已被新的连接操作取代"),
            result = op => result,
        }
    }

    /// A status refresh never reads `op_interrupt`, so a stale cancelled token
    /// cannot break it; only Desktop shutdown cuts it short.
    async fn until_shutdown<T>(&self, op: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::select! {
            biased;
            _ = self.shutdown_cancel.cancelled() => anyhow::bail!("Desktop 正在关闭，已停止等待 Relay"),
            result = op => result,
        }
    }

    async fn refuse_unreadable_state(&self) -> Result<()> {
        let mut guard = self.runtime.lock().await;
        if guard.state_unreadable {
            guard.status.action_error_code = Some("RELAY_STATE_UNREADABLE");
            anyhow::bail!("本机 Relay 配置不可读；请使用“仅在本机忘记”后重新连接");
        }
        Ok(())
    }

    async fn set_action_error(&self, code: &'static str) {
        self.runtime.lock().await.status.action_error_code = Some(code);
    }

    /// A protected-state read failure inside a lifecycle operation reports the
    /// typed unreadable action code. Never hold the runtime lock while calling.
    async fn read_protected<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<T> {
        match read_secret_json(path) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.set_action_error("RELAY_STATE_UNREADABLE").await;
                Err(error)
            }
        }
    }

    async fn finish_operation(&self, result: &Result<RelayPairingStatus>) {
        let mut guard = self.runtime.lock().await;
        guard.status.operation = None;
        match result {
            Ok(_) => guard.status.action_error_code = None,
            Err(error) => {
                guard.status.state = "error";
                guard.status.error = Some(format!("{error:#}"));
                if guard.status.action_error_code.is_none() {
                    guard.status.action_error_code = Some("RELAY_OPERATION_FAILED");
                }
            }
        }
    }

    async fn ensure_webui_port(&self, server: &Arc<DesktopServer>, protocol_version: u8) -> Result<()> {
        if protocol_version == 2 {
            server.start_relay().await?;
            return Ok(());
        }
        let mut status = server.status_snapshot().await;
        if !status.running {
            status = server.start_lan().await;
        }
        if !status.running {
            anyhow::bail!(
                "{}",
                status
                    .error
                    .unwrap_or_else(|| "WebUI LAN listener 启动失败".to_owned())
            );
        }
        if status.port != REQUIRED_DESKTOP_PORT {
            anyhow::bail!(
                "WebUI 端口被占用：需要 {}，实际回退到 {}；未发送 Relay bootstrap 请求",
                REQUIRED_DESKTOP_PORT,
                status.port
            );
        }
        Ok(())
    }

    async fn exchange_bootstrap(&self, envelope: &PairingEnvelope) -> Result<BootstrapResponse> {
        let body = if envelope.protocol_version == 2 {
            let pending = if self.pending_path.is_file() {
                let pending: PendingBootstrap = self.read_protected(&self.pending_path).await?;
                if pending.bootstrap_url != envelope.bootstrap_url || pending.invite != envelope.invite {
                    anyhow::bail!("存在尚未确认的 Relay 配对请求，请重试原配对串或解除后重新连接");
                }
                pending
            } else {
                let pending = PendingBootstrap {
                    bootstrap_url: envelope.bootstrap_url.clone(), invite: envelope.invite.clone(),
                    expires_at: envelope.expires_at.clone(),
                    attempt_id: random_attempt_id()?,
                    enabled: true,
                };
                write_secret_json(&self.pending_path, &pending)?;
                pending
            };
            serde_json::to_string(&serde_json::json!({"attempt_id":pending.attempt_id}))?
        } else { "{}".to_owned() };
        if envelope.protocol_version == 2 { self.runtime.lock().await.status.pending_bootstrap = true; }
        let response = self
            .client_for_url(&envelope.bootstrap_url)
            .post(&envelope.bootstrap_url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", envelope.invite))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .context("请求 Relay bootstrap 失败")?;

        if response.status().is_redirection() {
            anyhow::bail!("Relay bootstrap 不允许 HTTP 重定向");
        }
        if response.status() != StatusCode::OK {
            let status = response.status();
            let code = response.json::<Value>().await.ok().and_then(|value| value.get("code").and_then(Value::as_str).map(str::to_owned));
            let (error_code, message) = match code.as_deref() {
                Some("enrol_expired") => ("RELAY_ENROL_EXPIRED", "Relay 入网凭据已过期；请解除原配对，再生成新配对码"),
                Some("invite_expired") => ("RELAY_INVITE_EXPIRED", "Relay 配对邀请已过期；请解除待确认请求，再生成新配对码"),
                Some("invite_used") => ("RELAY_INVITE_USED", "Relay 邀请已用于其他连接；请检查原配对或生成新的邀请"),
                Some("connection_revoked") => ("RELAY_CONNECTION_REVOKED", "Relay 配对已经撤销；请解除本地配对后重新连接"),
                _ => ("RELAY_BOOTSTRAP_REJECTED", "Relay 未确认配对请求；保留原请求，可重试或解除"),
            };
            self.runtime.lock().await.status.action_error_code = Some(error_code);
            anyhow::bail!(
                "{message}（HTTP {status}）"
            );
        }
        response
            .json::<BootstrapResponse>()
            .await
            .context("Relay bootstrap 响应格式无效")
    }

    /// Runtime preparation entry point for the native acceptance harness.
    pub(crate) async fn download_nfagent(&self) -> Result<PathBuf> {
        self.prepare_nfagent().await
    }

    /// Verify the shipped runtime and automatically prepare it when needed.
    async fn prepare_nfagent(&self) -> Result<PathBuf> {
        let interrupted = self.op_interrupt.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        let result = async {
            if let Some(path) = explicit_nfagent_path()? {
                ensure_executable(&path)?;
                return Ok(path);
            }
            let spec = configured_nfagent_download_spec()?;
            let _install_guard = tokio::select! {
                biased;
                _ = self.shutdown_cancel.cancelled() => anyhow::bail!("Desktop 关闭已取消网络代理端安装"),
                _ = interrupted.cancelled() => anyhow::bail!("Relay 操作已取消网络代理端安装"),
                guard = self.nfagent_install_lock.lock() => guard,
            };
            // Shutdown also interrupts the current operation. Passing its token
            // into the installer lets cancellation remove its staging file.
            prepare_nfagent_cached(&self.data_dir, &spec, &self.download_client, Some(&interrupted)).await
        }.await;
        self.runtime.lock().await.status.runtime_available = result.is_ok();
        result
    }
}

async fn prepare_nfagent_cached(
    data_dir: &Path,
    spec: &NfagentDownloadSpec,
    client: &reqwest::Client,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<PathBuf> {
    let destination = nfagent_cache_destination(data_dir, spec)?;
    let install_dir = destination.parent().context("nfagent cache destination has no parent")?;

    if verify_file_sha256(&destination, &spec.sha256)? {
        ensure_executable(&destination)?;
        return Ok(destination);
    }
    if destination.exists() {
        fs::remove_file(&destination).with_context(|| {
            format!(
                "remove corrupt cached nfagent executable {}",
                destination.display()
            )
        })?;
    }

    fs::create_dir_all(&install_dir).with_context(|| {
        format!(
            "create managed nfagent directory {}",
            install_dir.display()
        )
    })?;
    let temp_path = install_dir.join(format!(
        ".{}.{}.part",
        spec.file_name,
        unique_install_suffix()
    ));
    let result = if let Some(cancel) = cancel {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(anyhow::anyhow!("Relay 操作已取消网络代理端下载")),
            result = download_verified_nfagent(client, spec, &temp_path, &destination) => result,
        }
    } else {
        download_verified_nfagent(client, spec, &temp_path, &destination).await
    };
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp_path).await;
    }
    result?;
    Ok(destination)
}

async fn download_verified_nfagent(
    client: &reqwest::Client,
    spec: &NfagentDownloadSpec,
    temp_path: &Path,
    destination: &Path,
) -> Result<()> {
    let mut response = client
        .get(spec.url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .with_context(|| format!("download nfagent from {}", spec.url))?;
    if !response.status().is_success() {
        anyhow::bail!(
            "nfagent download returned HTTP {} from {}",
            response.status(),
            spec.url
        );
    }
    if response
        .content_length()
        .is_some_and(|length| length > NFAGENT_MAX_DOWNLOAD_BYTES)
    {
        anyhow::bail!(
            "nfagent download exceeds the {}-byte limit",
            NFAGENT_MAX_DOWNLOAD_BYTES
        );
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temp_path)
        .await
        .with_context(|| format!("create staged nfagent {}", temp_path.display()))?;
    let mut hasher = Sha256::new();
    let mut written = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("read nfagent response body from {}", spec.url))?
    {
        written = written
            .checked_add(chunk.len() as u64)
            .context("nfagent download length overflow")?;
        if written > NFAGENT_MAX_DOWNLOAD_BYTES {
            anyhow::bail!(
                "nfagent download exceeds the {}-byte limit",
                NFAGENT_MAX_DOWNLOAD_BYTES
            );
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .with_context(|| format!("write staged nfagent {}", temp_path.display()))?;
    }
    if written == 0 {
        anyhow::bail!("nfagent download was empty");
    }
    file.flush()
        .await
        .with_context(|| format!("flush staged nfagent {}", temp_path.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("sync staged nfagent {}", temp_path.display()))?;
    drop(file);

    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(&spec.sha256) {
        anyhow::bail!(
            "nfagent checksum mismatch: expected {}, received {}",
            spec.sha256,
            actual
        );
    }
    publish_staged_nfagent(temp_path, destination, &spec.sha256).await
}

impl RelayPairingManager {
    fn client_for_url(&self, raw: &str) -> &reqwest::Client {
        let loopback = Url::parse(raw).ok().is_some_and(|url| relay_url_is_loopback(&url));
        if loopback { &self.local_client } else { &self.client }
    }

    async fn revoke_control(&self, credential: &RelayControlCredential) -> Result<()> {
        let response = self.client_for_url(&credential.control_url).delete(&credential.control_url)
            .bearer_auth(&credential.control_token).send().await.context("Relay 解绑未确认，保留配对供重试")?;
        let revoked = response.status() == StatusCode::OK && response.json::<Value>().await?
            .get("revoked").and_then(Value::as_bool) == Some(true);
        if !revoked { anyhow::bail!("Relay 解绑未确认，保留配对供重试"); }
        Ok(())
    }

    async fn pending_enrol_token(&self, state: &PersistedPairing) -> Result<Option<String>> {
        if state.protocol_version != 2 || self.agent_state_dir.join("credential.json").is_file() {
            return Ok(None);
        }
        let credential: RelayControlCredential = self.read_protected(&self.control_path).await?;
        if credential.enrol_expires_at.is_some_and(|expiry| expiry > Utc::now()) {
            return Ok(credential.enrol_token);
        }
        Ok(None)
    }

    async fn check_health(&self, server: &Arc<DesktopServer>, state: &PersistedPairing) -> RelayHealth {
        let checked_at_ms = Some(Utc::now().timestamp_millis().max(0) as u64);
        if state.protocol_version == 1 {
            let endpoint = auth_status_url(if state.probe_url.is_empty() { &state.business_url } else { &state.probe_url });
            let tunnel_reachable = if let Ok(endpoint) = endpoint {
                self.client_for_url(endpoint.as_str()).get(endpoint.clone()).send().await.is_ok_and(|response| response.status() == StatusCode::OK)
            } else { false };
            return RelayHealth { backend_reachable: server.status().running, tunnel_reachable, checked_at_ms };
        }
        let local = format!("http://127.0.0.1:{}/api/connections/info", server.loopback_port());
        let public = format!("{}/api/connections/info", state.business_url.trim_end_matches('/'));
        let (local, public) = tokio::join!(
            self.local_client.get(local).header("x-nomi-local-trust", server.local_trust_secret()).send(),
            self.client_for_url(&public).get(public.clone()).send()
        );
        let read = async |response: std::result::Result<reqwest::Response, reqwest::Error>| -> Option<Value> {
            let response = response.ok()?;
            if response.status() != StatusCode::OK { return None; }
            let value = response.json::<Value>().await.ok()?;
            if value.get("success").and_then(Value::as_bool) != Some(true) { return None; }
            let data = value.get("data")?.clone();
            if data.get("protocol_version").and_then(Value::as_u64) != Some(1) { return None; }
            data.get("desktop_id")?.as_str().filter(|id| !id.is_empty())?;
            Some(data)
        };
        let local = read(local).await;
        let public = read(public).await;
        let mut tunnel_reachable = local.as_ref().zip(public.as_ref()).is_some_and(|(local, public)| local["desktop_id"] == public["desktop_id"]);
        let control = async {
            let credential: RelayControlCredential = read_secret_json(&self.control_path).ok()?;
            if credential.control_url != state.control_url { return None; }
            let response = self.client_for_url(&state.control_url).get(&state.control_url).bearer_auth(credential.control_token).send().await.ok()?;
            if response.status() != StatusCode::OK { return None; }
            let projection = response.json::<RelayConnectionProjection>().await.ok()?;
            if projection.protocol_version != 2 || projection.invite_id != state.invite_id || projection.tunnel_id != state.tunnel_id { return None; }
            Some(projection)
        }.await;
        if let Some(control) = control {
            tunnel_reachable &= !control.revoked && control.state == "transport_ready";
            let mut guard = self.runtime.lock().await;
            guard.status.relay_state = Some(control.state);
            guard.status.relay_reason = Some(control.reason);
        } else {
            tunnel_reachable = false;
        }
        RelayHealth { backend_reachable: local.is_some(), tunnel_reachable, checked_at_ms }
    }

    async fn start_agent(
        self: &Arc<Self>,
        state: PersistedPairing,
        enrol_token: Option<String>,
        path: PathBuf,
    ) -> Result<()> {
        let _child_owner = self.child_gate.lock().await;
        anyhow::ensure!(!self.closing.load(Ordering::Acquire), "Desktop 已开始关闭，不能启动网络代理端");
        ensure_child_path(&self.data_dir, &self.agent_state_dir)?;
        prepare_agent_state_dir(&self.agent_state_dir)?;
        let mut builder = ChildProcessBuilder::new(path);
        builder
            .args(["-relay", state.relay.as_str()])
            .args(["-pin", state.pin.as_str()])
            .args(["-state-dir", self.agent_state_dir.to_string_lossy().as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        builder.args(["-l2-port", state.l2_port.to_string().as_str()]);
        if let Some(token) = enrol_token.as_deref() {
            builder.args(["-token", token]);
        }

        let process = builder
            .spawn_managed()
            .context("启动 nfagent 失败")?;
        let pid = process.id();
        let mut guard = self.runtime.lock().await;
        if guard.process.is_some() {
            drop(guard);
            let mut process = process;
            let _ = process.shutdown().await;
            anyhow::bail!("nfagent 已在运行");
        }
        guard.generation = guard.generation.wrapping_add(1);
        let generation = guard.generation;
        guard.process = Some(process);
        guard.status = status_from_persisted(&state, "connecting", None);
        guard.status.runtime_available = true;
        guard.status.agent_pid = pid;
        guard.health_checked_at = None;
        drop(guard);

        let manager = Arc::clone(self);
        tokio::spawn(async move {
            manager.monitor_agent(generation).await;
        });
        Ok(())
    }

    async fn monitor_agent(self: Arc<Self>, generation: u64) {
        loop {
            let exited = {
                let mut guard = self.runtime.lock().await;
                if guard.generation != generation {
                    return;
                }
                let Some(process) = guard.process.as_mut() else {
                    return;
                };
                match process.child_mut().try_wait() {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(error) => {
                        guard.status.state = "error";
                        guard.status.error = Some(format!("agent 状态检查失败：{error}"));
                        return;
                    }
                }
            };
            if exited {
                let _child_owner = self.child_gate.lock().await;
                let mut process = {
                    let mut guard = self.runtime.lock().await;
                    if guard.generation != generation {
                        return;
                    }
                    guard.process.take()
                };
                if let Some(mut child) = process.take() {
                    if let Err(error) = child.shutdown().await {
                        let mut guard = self.runtime.lock().await;
                        guard.process = Some(child);
                        guard.status.state = "error";
                        guard.status.error_code = Some("RELAY_AGENT_CLEANUP_UNCONFIRMED");
                        guard.status.error = Some(format!("网络代理端已退出，但清理尚未确认：{error}"));
                        return;
                    }
                }
                let mut guard = self.runtime.lock().await;
                if guard.generation == generation {
                    guard.status.state = "error";
                    guard.status.agent_pid = None;
                    guard.status.error_code = Some("RELAY_AGENT_EXITED");
                    guard.status.error =
                        Some("nfagent 已退出；可点击“重新连接”恢复连接".to_owned());
                }
                return;
            }
            sleep(RESTORE_POLL_INTERVAL).await;
        }
    }

    async fn stop_agent(&self) -> Result<()> {
        let _child_owner = self.child_gate.lock().await;
        let mut process = {
            let mut guard = self.runtime.lock().await;
            guard.generation = guard.generation.wrapping_add(1);
            guard.process.take()
        };
        if let Some(mut child) = process.take() {
            if let Err(error) = child.shutdown().await {
                let mut guard = self.runtime.lock().await;
                guard.process = Some(child);
                guard.status.state = "error";
                guard.status.error_code = Some("RELAY_AGENT_CLEANUP_UNCONFIRMED");
                return Err(error).context("停止 nfagent 失败，保留进程 owner 供重试");
            }
        }
        Ok(())
    }

    async fn wait_for_agent_ready(&self, server: &Arc<DesktopServer>, state: &PersistedPairing) -> Result<()> {
        let probe_url = if state.probe_url.trim().is_empty() {
            &state.business_url
        } else {
            &state.probe_url
        };
        let endpoint = auth_status_url(probe_url)?;
        let deadline = Instant::now() + AGENT_POLL_TIMEOUT;
        loop {
            let credential_ready = self.agent_state_dir.join("credential.json").is_file();
            let http_ready = if state.protocol_version == 2 {
                let health = self.interruptible(async { Ok(self.check_health(server, state).await) }).await?;
                let ready = health.backend_reachable && health.tunnel_reachable;
                let mut guard = self.runtime.lock().await;
                guard.status.health = health;
                guard.health_checked_at = Some(Instant::now());
                drop(guard);
                ready
            } else {
                self.interruptible(async {
                    Ok(self
                        .client_for_url(endpoint.as_str())
                        .get(endpoint.clone())
                        .send()
                        .await
                        .is_ok_and(|response| response.status() == StatusCode::OK))
                }).await?
            };
            if credential_ready && http_ready {
                if state.protocol_version == 2 {
                    let mut credential: RelayControlCredential = self.read_protected(&self.control_path).await?;
                    credential.enrol_token = None;
                    credential.enrol_expires_at = None;
                    write_secret_json(&self.control_path, &credential)?;
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.set_action_error("RELAY_AGENT_NOT_READY").await;
                anyhow::bail!(
                    "等待 tunnel 就绪超时（credential.json={}，HTTP={}）",
                    credential_ready,
                    http_ready
                );
            }
            self.interruptible(async { sleep(Duration::from_millis(300)).await; Ok(()) }).await?;
        }
    }

    async fn mint_final_pairing_url(
        &self,
        server: &Arc<DesktopServer>,
        state: &PersistedPairing,
    ) -> Result<(String, u64)> {
        let endpoint = format!(
            "http://127.0.0.1:{}/api/connections/pairing",
            server.loopback_port()
        );
        let response = self
            .local_client
            .post(endpoint)
            .header("x-nomi-local-trust", server.local_trust_secret())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .context("生成 Desktop QR token 失败")?;
        if response.status() != StatusCode::OK {
            anyhow::bail!("生成 Desktop QR token 返回 HTTP {}", response.status());
        }
        let bytes = response
            .bytes()
            .await
            .context("读取 Desktop QR token 响应失败")?;
        let qr = parse_qr_token_response(&bytes)?;
        if !is_hex_token(&qr.qr_token) {
            anyhow::bail!("Desktop QR token 格式无效");
        }
        let nested = format!(
            "{}/device-pair?token={}",
            state.business_url.trim_end_matches('/'),
            qr.qr_token
        );
        let encoded: String = percent_encode_pairing_url(&nested);
        self.runtime.lock().await.status.ticket_id = Some(qr.ticket_id);
        Ok((format!("nomifun://pair?v=1&url={encoded}"), qr.expires_at_ms))
    }

    async fn set_status(&self, mut status: RelayPairingStatus) {
        let mut guard = self.runtime.lock().await;
        status.runtime_available = guard.status.runtime_available;
        guard.status = status;
    }

    async fn set_error(&self, message: String) {
        let mut guard = self.runtime.lock().await;
        guard.status.state = "error";
        guard.status.error = Some(message);
        guard.status.agent_pid = guard.process.as_ref().and_then(ManagedChildProcess::id);
        if guard.status.action_error_code.is_none() {
            guard.status.action_error_code = Some("RELAY_OPERATION_FAILED");
        }
    }

    async fn refresh_pairing_url(
        &self,
        server: &Arc<DesktopServer>,
        state: &PersistedPairing,
    ) -> Result<()> {
        let (pairing_url, expires) = self.mint_final_pairing_url(server, state).await?;
        let mut guard = self.runtime.lock().await;
        guard.status.pairing_url = Some(pairing_url);
        guard.status.qr_state = "ready";
        guard.status.qr_expires_at_ms = Some(expires);
        guard.status.webui_port = Some(desktop_port(state.protocol_version));
        guard.status.agent_pid = guard.process.as_ref().and_then(ManagedChildProcess::id);
        Ok(())
    }

    async fn mark_connected(&self, state: &PersistedPairing) {
        let mut guard = self.runtime.lock().await;
        let pair_url = guard.status.pairing_url.take();
        let expires = guard.status.qr_expires_at_ms.take();
        let mut status = status_from_persisted(state, "connected", None);
        status.agent_pid = guard.process.as_ref().and_then(ManagedChildProcess::id);
        status.pairing_url = pair_url;
        status.qr_expires_at_ms = expires;
        status.webui_port = Some(desktop_port(state.protocol_version));
        status.health = guard.status.health.clone();
        status.ticket_id = guard.status.ticket_id.clone();
        status.qr_state = guard.status.qr_state;
        status.relay_state = guard.status.relay_state.clone();
        status.relay_reason = guard.status.relay_reason.clone();
        status.runtime_available = guard.status.runtime_available;
        guard.status = status;
    }

}

fn parse_qr_token_response(bytes: &[u8]) -> Result<QrTokenResponse> {
    let envelope: QrTokenEnvelope =
        serde_json::from_slice(bytes).context("Desktop QR token 响应格式无效")?;
    if !envelope.success {
        anyhow::bail!(
            "{}",
            envelope
                .message
                .unwrap_or_else(|| "Desktop QR token 响应表示失败".to_owned())
        );
    }
    envelope
        .data
        .context("Desktop QR token 响应缺少 data")
}

fn parse_pairing_envelope(raw: &str) -> Result<PairingEnvelope> {
    let raw = raw.trim();
    let (encoded, protocol_version) = if let Some(encoded) = raw.strip_prefix(ENVELOPE_V2_PREFIX) {
        (encoded, 2)
    } else {
        (raw.strip_prefix(ENVELOPE_PREFIX).context("不是受支持的 Relay Desktop pairing 串")?, 1)
    };
    if encoded.is_empty() || encoded.contains('=') {
        anyhow::bail!("Relay pairing 串编码无效");
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .context("Relay pairing 串不是合法 base64url")?;
    if bytes.len() > 16 * 1024 {
        anyhow::bail!("Relay pairing 串过大");
    }
    let mut envelope: PairingEnvelope =
        serde_json::from_slice(&bytes).context("Relay pairing 串 JSON 无效")?;
    envelope.protocol_version = protocol_version;
    validate_bootstrap_url(&envelope.bootstrap_url)?;
    let expected_path = if protocol_version == 2 { "/api/bootstrap/desktop/v2" } else { "/api/bootstrap/desktop" };
    if Url::parse(&envelope.bootstrap_url)?.path().trim_end_matches('/') != expected_path {
        anyhow::bail!("Relay 配对版本与 bootstrap URL 不匹配");
    }
    if protocol_version == 2 { validate_secure_relay_url(&envelope.bootstrap_url)?; }
    if envelope.invite.trim().is_empty() || envelope.invite.len() > 4096 {
        anyhow::bail!("Relay pairing 串 invite 无效");
    }
    Ok(envelope)
}

fn envelope_expires_at(envelope: &PairingEnvelope) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&envelope.expires_at)
        .map(|expires| expires.with_timezone(&Utc))
        .context("Relay pairing 串过期时间无效")
}

fn validate_envelope_expiry(envelope: &PairingEnvelope) -> Result<()> {
    if envelope_expires_at(envelope)? <= Utc::now() {
        anyhow::bail!("Relay pairing 串已过期");
    }
    Ok(())
}

fn validate_bootstrap_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw).context("bootstrap URL 无效")?;
    if url.scheme() != "http" && url.scheme() != "https" {
        anyhow::bail!("bootstrap URL 必须使用 HTTP(S)");
    }
    if url.host_str().is_none() || url.username() != "" || url.password().is_some() {
        anyhow::bail!("bootstrap URL 不得包含凭据");
    }
    if url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("bootstrap URL 不得包含 query 或 fragment");
    }
    if !matches!(url.path().trim_end_matches('/'), "/api/bootstrap/desktop" | "/api/bootstrap/desktop/v2") {
        anyhow::bail!("bootstrap URL 路径不是 /api/bootstrap/desktop");
    }
    Ok(())
}

fn validate_bootstrap_response(response: &BootstrapResponse) -> Result<()> {
    if !matches!(response.protocol_version, 1 | 2) { anyhow::bail!("不支持的 Relay bootstrap 版本"); }
    validate_relay_addr(&response.relay)?;
    validate_business_url(&response.business_url)?;
    let probe_url = if response.probe_url.trim().is_empty() {
        &response.business_url
    } else {
        &response.probe_url
    };
    validate_business_url(probe_url)?;
    if response.invite_id.trim().is_empty() || response.invite_id.len() > 256 {
        anyhow::bail!("Relay 返回的 invite_id 无效");
    }
    if !is_spki_pin(&response.pin) {
        anyhow::bail!("Relay 返回的证书指纹无效");
    }
    if response.enrol_token.trim().is_empty() || response.enrol_token.len() > 4096 {
        anyhow::bail!("Relay 返回的 agent 入网令牌无效");
    }
    let backend = response
        .tunnel
        .get("backend")
        .and_then(Value::as_object)
        .context("Relay tunnel 缺少 backend")?;
    if backend.get("kind").and_then(Value::as_str) != Some("addr")
        || backend.get("addr").and_then(Value::as_str) != Some(if response.protocol_version == 2 { V2_DESKTOP_BACKEND_ADDR } else { DESKTOP_BACKEND_ADDR })
    {
        anyhow::bail!("Relay tunnel backend 不是固定 Desktop loopback");
    }
    if response.protocol_version == 2 {
        if response.l2_port.is_none() { anyhow::bail!("Relay v2 未返回传输回退端口合同"); }
        validate_secure_relay_url(&response.business_url)?;
        validate_secure_relay_url(probe_url)?;
        validate_secure_relay_url(&response.control_url)?;
        let control = Url::parse(&response.control_url)?;
        if control.path() != format!("/api/desktop-connections/{}", response.invite_id)
            || control.query().is_some() || control.fragment().is_some()
            || !response.control_token.strip_prefix("nf_desktop_").is_some_and(|token| token.len() == 64 && is_hex_token(token)) {
            anyhow::bail!("Relay 管理合同无效");
        }
        if !matches!(response.state.as_str(), "pending" | "transport_ready" | "error") {
            anyhow::bail!("Relay 接入状态合同无效");
        }
    } else {
    let listen_port = response
        .tunnel
        .get("listen_port")
        .and_then(Value::as_u64)
        .context("Relay tunnel 缺少 listen_port")?;
    let probe_port = Url::parse(probe_url)
        .ok()
        .and_then(|url| url.port_or_known_default())
        .context("probe_url 缺少端口")?;
    if listen_port != u64::from(probe_port) {
        anyhow::bail!("Relay tunnel listen_port 与 probe_url 不一致");
    }
    }
    for field in ["id", "slug"] {
        if tunnel_string(&response.tunnel, field)?.trim().is_empty() {
            anyhow::bail!("Relay tunnel 缺少 {field}");
        }
    }
    Ok(())
}

fn validate_relay_addr(raw: &str) -> Result<()> {
    if raw.is_empty() || raw.len() > 512 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        anyhow::bail!("Relay 地址无效");
    }
    let (host, port) = raw
        .rsplit_once(':')
        .context("Relay 地址必须是 host:port")?;
    if host.is_empty() || port.parse::<u16>().is_err() {
        anyhow::bail!("Relay 地址必须是合法 host:port");
    }
    if host.parse::<IpAddr>().is_err()
        && !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    {
        anyhow::bail!("Relay 主机名包含非法字符");
    }
    Ok(())
}

fn validate_business_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw).context("business_url 无效")?;
    if url.scheme() != "http" && url.scheme() != "https" {
        anyhow::bail!("business_url 必须使用 HTTP(S)");
    }
    if url.host_str().is_none() || url.username() != "" || url.password().is_some() {
        anyhow::bail!("business_url 不得包含凭据");
    }
    if url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("business_url 不得包含 query 或 fragment");
    }
    if !url.path().is_empty() && url.path() != "/" {
        anyhow::bail!("business_url 不得包含路径");
    }
    Ok(())
}

fn auth_status_url(raw: &str) -> Result<Url> {
    let mut url = Url::parse(raw).context("business_url 无效")?;
    url.set_path("/api/auth/status");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn tunnel_string(tunnel: &Value, key: &str) -> Result<String> {
    tunnel
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .with_context(|| format!("Relay tunnel 缺少 {key}"))
}

fn is_hex_token(token: &str) -> bool {
    (32..=128).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

fn percent_encode_pairing_url(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                vec![byte as char]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect()
}

fn is_spki_pin(pin: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(pin)
        .is_ok_and(|bytes| bytes.len() == 32)
}

fn status_from_persisted(
    state: &PersistedPairing,
    phase: &'static str,
    message: Option<String>,
) -> RelayPairingStatus {
    RelayPairingStatus {
        state: phase,
        error: message,
        relay: Some(state.relay.clone()),
        business_url: Some(state.business_url.clone()),
        invite_id: Some(state.invite_id.clone()),
        tunnel_id: Some(state.tunnel_id.clone()),
        tunnel_slug: Some(state.tunnel_slug.clone()),
        agent_state_dir: Some(state.agent_state_dir.clone()),
        enabled: state.enabled,
        protocol_version: state.protocol_version,
        ..Default::default()
    }
}

// A recovered tunnel also settles a stale RELAY_AGENT_NOT_READY left by a timed-out
// readiness wait, because the agent's own reconnect loop can recover after it.
fn apply_health(status: &mut RelayPairingStatus, health: RelayHealth) {
    status.health = health;
    if status.health.backend_reachable && status.health.tunnel_reachable {
        status.state = "connected";
        status.error = None;
        if status.error_code == Some("RELAY_TUNNEL_UNREACHABLE") {
            status.error_code = None;
        }
        if status.action_error_code == Some("RELAY_AGENT_NOT_READY") {
            status.action_error_code = None;
        }
    } else {
        status.state = "reconnecting";
        status.error_code = Some("RELAY_TUNNEL_UNREACHABLE");
    }
}

fn load_persisted(path: &Path) -> Result<Option<PersistedPairing>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("读取配对状态 {}", path.display()))?;
    let state: PersistedPairing =
        serde_json::from_slice(&bytes).context("Relay 配对状态格式无效")?;
    if !matches!(state.version, 1 | STATE_VERSION)
        || !matches!(state.protocol_version, 1 | 2)
        || state.relay.is_empty()
        || state.pin.is_empty()
        || state.business_url.is_empty()
        || state.invite_id.is_empty()
        || state.tunnel_id.is_empty()
        || state.tunnel_slug.is_empty()
        || state.agent_state_dir != AGENT_STATE_DIR_NAME
    {
        anyhow::bail!("Relay 配对状态版本或字段无效");
    }
    validate_relay_addr(&state.relay)?;
    validate_business_url(&state.business_url)?;
    if !state.probe_url.is_empty() {
        validate_business_url(&state.probe_url)?;
    }
    if !is_spki_pin(&state.pin) {
        anyhow::bail!("Relay 配对状态中的证书指纹无效");
    }
    Ok(Some(state))
}

fn desktop_port(protocol_version: u8) -> u16 {
    if protocol_version == 2 { V2_DESKTOP_PORT } else { REQUIRED_DESKTOP_PORT }
}

fn validate_secure_relay_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw).context("Relay URL 无效")?;
    if url.username() != "" || url.password().is_some() { anyhow::bail!("Relay URL 不得包含凭据"); }
    let loopback = relay_url_is_loopback(&url);
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        anyhow::bail!("公网 Relay 必须使用 HTTPS；HTTP 仅允许本机开发地址");
    }
    Ok(())
}

fn relay_url_is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn write_persisted(path: &Path, state: &PersistedPairing) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state)?;
    fs::write(&temp, bytes).with_context(|| format!("写入临时配对状态 {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("提交配对状态 {}", path.display()))?;
    Ok(())
}

fn random_attempt_id() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| anyhow::anyhow!("无法生成安全配对请求标识"))?;
    Ok(hex::encode(bytes))
}

fn pending_envelope(pending: &PendingBootstrap) -> Result<String> {
    let bytes = serde_json::to_vec(&serde_json::json!({"bootstrap_url":pending.bootstrap_url,
        "invite":pending.invite,"expires_at":pending.expires_at}))?;
    Ok(format!("{ENVELOPE_V2_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)))
}

/// Secrets never enter the public pairing metadata. Windows seals them to the
/// current OS user with DPAPI; Unix creates the atomic replacement with 0600.
fn write_secret_json(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let plaintext = serde_json::to_vec(value)?;
    #[cfg(windows)]
    let bytes = windows_user_secret(&plaintext, true)?;
    #[cfg(not(windows))]
    let bytes = plaintext;
    let parent = path.parent().context("secret file has no parent")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    staged.write_all(&bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| anyhow::anyhow!("保存配对凭据失败: {}", error.error))?;
    Ok(())
}

fn read_secret_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).context("读取受保护的 Relay 配对凭据失败")?;
    #[cfg(windows)]
    let bytes = windows_user_secret(&bytes, false)?;
    serde_json::from_slice(&bytes).context("受保护的 Relay 配对凭据格式无效")
}

#[cfg(windows)]
fn windows_user_secret(bytes: &[u8], protect: bool) -> Result<Vec<u8>> {
    use std::ffi::c_void;
    #[repr(C)]
    struct Blob { size: u32, data: *mut u8 }
    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(input: *const Blob, description: *const u16, entropy: *const Blob,
            reserved: *const c_void, prompt: *const c_void, flags: u32, output: *mut Blob) -> i32;
        fn CryptUnprotectData(input: *const Blob, description: *mut *mut u16, entropy: *const Blob,
            reserved: *const c_void, prompt: *const c_void, flags: u32, output: *mut Blob) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" { fn LocalFree(pointer: *mut c_void) -> *mut c_void; }
    let input = Blob { size: u32::try_from(bytes.len()).context("配对凭据过大")?, data: bytes.as_ptr().cast_mut() };
    let mut output = Blob { size: 0, data: std::ptr::null_mut() };
    // SAFETY: DPAPI reads the bounded input, allocates output with LocalAlloc,
    // and requires no UI. Output is copied and always released with LocalFree.
    let success = unsafe {
        if protect {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 1, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 1, &mut output)
        }
    };
    if success == 0 { return Err(std::io::Error::last_os_error()).context("Windows 用户凭据保护失败"); }
    let result = unsafe { std::slice::from_raw_parts(output.data, output.size as usize).to_vec() };
    unsafe { LocalFree(output.data.cast()); }
    Ok(result)
}

fn prepare_agent_state_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("创建 agent 状态目录 {}", path.display()))?;
    let probe = path.join(".write-probe");
    fs::write(&probe, b"ok").with_context(|| format!("写入 agent 状态目录 {}", path.display()))?;
    let _ = fs::remove_file(probe);
    Ok(())
}

fn ensure_child_path(root: &Path, child: &Path) -> Result<()> {
    let requested_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve relative data directory")?
            .join(root)
    };
    let canonical_root = dunce::canonicalize(&requested_root).with_context(|| {
        format!("canonicalize data directory {}", requested_root.display())
    })?;
    let child = if child.is_absolute() {
        child.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve relative managed path")?
            .join(child)
    };
    let relative = child
        .strip_prefix(&requested_root)
        .or_else(|_| child.strip_prefix(&canonical_root))
        .context("resolve managed path relative to data directory")?;
    if relative.as_os_str().is_empty() {
        anyhow::bail!("拒绝操作数据目录之外的 agent 状态路径");
    }
    let mut current = canonical_root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            anyhow::bail!("managed agent path contains an unsafe path component");
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    anyhow::bail!(
                        "拒绝通过符号链接或 junction 操作 agent 状态路径 {}",
                        current.display()
                    );
                }
                let canonical = dunce::canonicalize(&current).with_context(|| {
                    format!("canonicalize managed agent path {}", current.display())
                })?;
                if !canonical.starts_with(&canonical_root) {
                    anyhow::bail!("拒绝操作数据目录之外的 agent 状态路径");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect managed agent path {}", current.display())
                });
            }
        }
    }
    Ok(())
}

fn explicit_nfagent_path() -> Result<Option<PathBuf>> {
    let Some(raw) = std::env::var_os("NOMIFUN_NFAGENT_PATH") else {
        return Ok(None);
    };
    let spec = configured_nfagent_download_spec()?;
    Ok(Some(validate_explicit_nfagent_path(
        &PathBuf::from(raw),
        &spec,
    )?))
}

/// Resolve approved local bytes without installing, repairing, or changing
/// permissions. A general PATH lookup cannot establish nfagent identity.
fn existing_nfagent_path(data_dir: &Path) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit_nfagent_path()? {
        return Ok(is_executable(&path)?.then_some(path));
    }
    let spec = configured_nfagent_download_spec()?;
    let path = nfagent_cache_destination(data_dir, &spec)?;
    Ok((verify_file_sha256(&path, &spec.sha256)? && is_executable(&path)?).then_some(path))
}

fn nfagent_cache_destination(data_dir: &Path, spec: &NfagentDownloadSpec) -> Result<PathBuf> {
    if !is_sha256_hex(&spec.sha256) {
        anyhow::bail!("nfagent runtime SHA-256 is invalid");
    }
    let destination = data_dir
        .join(NFAGENT_RUNTIME_DIR_NAME)
        .join(NFAGENT_CACHE_DIR_NAME)
        .join(cache_component(&spec.version))
        .join(&spec.sha256[..16])
        .join(&spec.file_name);
    ensure_child_path(data_dir, &destination)?;
    Ok(destination)
}

fn validate_explicit_nfagent_path(path: &Path, spec: &NfagentDownloadSpec) -> Result<PathBuf> {
    if !path.is_absolute() || !path.is_file() {
        anyhow::bail!("NOMIFUN_NFAGENT_PATH 必须是存在的绝对路径");
    }
    let canonical = dunce::canonicalize(path).context("canonicalize explicit nfagent path")?;
    if !is_sha256_hex(&spec.sha256) || !verify_file_sha256(&canonical, &spec.sha256)? {
        anyhow::bail!(
            "NOMIFUN_NFAGENT_PATH 与当前平台批准的 nfagent {} SHA-256 不匹配",
            spec.version
        );
    }
    Ok(canonical)
}

fn configured_nfagent_download_spec() -> Result<NfagentDownloadSpec> {
    let manifest: NfagentRuntimeManifest =
        serde_json::from_str(NFAGENT_MANIFEST).context("nfagent runtime manifest is invalid")?;
    nfagent_download_spec_from_manifest(&manifest, nfagent_target_key())
}

fn nfagent_download_spec_from_manifest(
    manifest: &NfagentRuntimeManifest,
    target: &str,
) -> Result<NfagentDownloadSpec> {
    validate_runtime_version(&manifest.version)?;
    let asset = manifest
        .assets
        .get(target)
        .with_context(|| format!("nfagent runtime has no asset for target {target}"))?;
    let url = Url::parse(&asset.url).context("nfagent runtime URL is invalid")?;
    validate_nfagent_download_url(&url)?;
    if !is_sha256_hex(&asset.sha256) {
        anyhow::bail!("nfagent runtime SHA-256 must be exactly 64 hexadecimal characters");
    }
    let file_name = url
        .path_segments()
        .and_then(|segments| segments.last())
        .context("nfagent runtime URL must end in a file name")?
        .to_owned();
    validate_nfagent_file_name(&file_name, target)?;
    Ok(NfagentDownloadSpec {
        version: manifest.version.clone(),
        file_name,
        url,
        sha256: asset.sha256.to_ascii_lowercase(),
    })
}

fn validate_runtime_version(version: &str) -> Result<()> {
    if version.is_empty() || version.len() > NFAGENT_RUNTIME_VERSION_MAX_LEN {
        anyhow::bail!(
            "nfagent runtime version must be 1-{} characters",
            NFAGENT_RUNTIME_VERSION_MAX_LEN
        );
    }
    if version.eq_ignore_ascii_case("latest")
        || version
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')))
    {
        anyhow::bail!("nfagent runtime version must be an immutable safe identifier");
    }
    Ok(())
}

fn validate_nfagent_download_url(url: &Url) -> Result<()> {
    if url.scheme() != "https" || url.host_str().is_none() {
        anyhow::bail!("nfagent runtime URL must be an absolute HTTPS URL");
    }
    if url.username() != "" || url.password().is_some() {
        anyhow::bail!("nfagent runtime URL must not contain credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("nfagent runtime URL must not contain a query or fragment");
    }
    if url
        .path_segments()
        .into_iter()
        .flatten()
        .any(|segment| segment.eq_ignore_ascii_case("latest"))
    {
        anyhow::bail!("nfagent runtime URL must be immutable and must not use latest");
    }
    Ok(())
}

fn validate_nfagent_file_name(file_name: &str, target: &str) -> Result<()> {
    if file_name.is_empty()
        || file_name.len() > NFAGENT_FILE_NAME_MAX_LEN
        || matches!(file_name, "." | "..")
        || file_name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
    {
        anyhow::bail!("nfagent runtime URL must end in a safe file name");
    }
    if file_name.to_ascii_lowercase().contains("latest") {
        anyhow::bail!("nfagent runtime URL must be immutable and must not use latest");
    }
    if target.starts_with("windows-") && !file_name.to_ascii_lowercase().ends_with(".exe") {
        anyhow::bail!("Windows nfagent runtime assets must end in .exe");
    }
    let expected = if target.starts_with("windows-") {
        format!("nfagent-{target}.exe")
    } else {
        format!("nfagent-{target}")
    };
    if file_name != expected {
        anyhow::bail!("nfagent runtime asset for {target} must be named {expected}");
    }
    Ok(())
}

fn nfagent_target_key() -> &'static str {
    if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows-amd64"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "aarch64") {
        "windows-arm64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "darwin-amd64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-arm64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-amd64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else {
        "unsupported"
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn cache_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn unique_install_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{}", std::process::id(), nanos)
}

fn verify_file_sha256(path: &Path, expected: &str) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let mut file = fs::File::open(path)
        .with_context(|| format!("open cached nfagent {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("read cached nfagent {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()).eq_ignore_ascii_case(expected))
}

fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("stat nfagent {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("set executable permission on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn is_executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(path)
            .with_context(|| format!("stat nfagent {}", path.display()))?;
        Ok(metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(true)
    }
}

async fn publish_staged_nfagent(
    temp_path: &Path,
    destination: &Path,
    expected_sha256: &str,
) -> Result<()> {
    ensure_executable(temp_path)?;
    match tokio::fs::hard_link(temp_path, destination).await {
        Ok(()) => {
            tokio::fs::remove_file(temp_path).await.with_context(|| {
                format!("remove published staging link {}", temp_path.display())
            })?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            // A second Desktop instance may have completed the same download
            // between our cache check and publication. Keep a valid existing
            // cache and discard only our redundant staged copy.
            if verify_file_sha256(destination, expected_sha256)? {
                tokio::fs::remove_file(temp_path).await.with_context(|| {
                    format!("remove redundant staged nfagent {}", temp_path.display())
                })?;
                return Ok(());
            }

            // The destination is known to be corrupt, so replacing it is safe.
            tokio::fs::remove_file(destination)
                .await
                .with_context(|| {
                    format!(
                        "remove corrupt managed nfagent executable {}",
                        destination.display()
                    )
                })?;
            tokio::fs::hard_link(temp_path, destination)
                .await
                .with_context(|| {
                    format!(
                        "publish managed nfagent {} -> {}",
                        temp_path.display(),
                        destination.display()
                    )
                })?;
            tokio::fs::remove_file(temp_path).await.with_context(|| {
                format!("remove published staging link {}", temp_path.display())
            })?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "publish managed nfagent {} -> {}",
                    temp_path.display(),
                    destination.display()
                )
            });
        }
    }
    ensure_executable(destination)?;
    Ok(())
}

fn https_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many nfagent download redirects")
        } else if attempt.url().scheme() == "https" {
            attempt.follow()
        } else {
            attempt.error("nfagent download redirect must remain on HTTPS")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_manifest(version: &str, target: &str, url: &str, sha256: &str) -> NfagentRuntimeManifest {
        NfagentRuntimeManifest {
            version: version.to_owned(),
            assets: std::collections::HashMap::from([(
                target.to_owned(),
                NfagentRuntimeAsset {
                    url: url.to_owned(),
                    sha256: sha256.to_owned(),
                },
            )]),
        }
    }

    fn test_download_spec(url: &str, bytes: &[u8]) -> NfagentDownloadSpec {
        NfagentDownloadSpec {
            version: "0.1.0-test".to_owned(),
            file_name: "nfagent-windows-amd64.exe".to_owned(),
            url: Url::parse(url).unwrap(),
            sha256: hex::encode(Sha256::digest(bytes)),
        }
    }

    async fn spawn_test_http_server(
        status: u16,
        body: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut request).await;
            let reason = if status == 200 { "OK" } else { "Error" };
            let header = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        (format!("http://{address}/nfagent-windows-amd64.exe"), handle)
    }

    fn envelope(value: Value) -> String {
        let bytes = serde_json::to_vec(&value).unwrap();
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        format!("{ENVELOPE_PREFIX}{encoded}")
    }

    #[test]
    fn pairing_envelope_is_strict_and_versioned() {
        let raw = envelope(serde_json::json!({
            "bootstrap_url": "http://127.0.0.1:19445/api/bootstrap/desktop",
            "invite": "invite-value",
            "expires_at": "2999-01-01T00:00:00Z"
        }));
        let parsed = parse_pairing_envelope(&raw).unwrap();
        assert_eq!(parsed.invite, "invite-value");

        let unknown = envelope(serde_json::json!({
            "bootstrap_url": "http://127.0.0.1:19445/api/bootstrap/desktop",
            "invite": "invite-value",
            "expires_at": "2999-01-01T00:00:00Z",
            "backend": "127.0.0.1:25808"
        }));
        assert!(parse_pairing_envelope(&unknown).is_err());
        assert!(parse_pairing_envelope("nomifun-relay-pair:v2:abc").is_err());
    }

    #[test]
    fn v2_envelope_requires_its_endpoint_and_public_https() {
        let raw = |url: &str| {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "bootstrap_url": url, "invite": "opaque-invite", "expires_at": "2999-01-01T00:00:00Z"
            })).unwrap();
            format!("{ENVELOPE_V2_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
        };
        assert_eq!(parse_pairing_envelope(&raw("https://relay.example.test/api/bootstrap/desktop/v2")).unwrap().protocol_version, 2);
        assert!(parse_pairing_envelope(&raw("http://relay.example.test/api/bootstrap/desktop/v2")).is_err());
        assert!(parse_pairing_envelope(&raw("https://relay.example.test/api/bootstrap/desktop")).is_err());
        assert!(parse_pairing_envelope(&raw("http://127.0.0.1:19001/api/bootstrap/desktop/v2")).is_ok());
    }

    #[test]
    fn secret_sidecar_round_trips_and_tampering_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("relay.secret");
        let credential = RelayControlCredential { control_url: "https://relay.example.test/api/desktop-connections/p1".into(),
            control_token: "test-secret-never-in-public-state".into(), enrol_token: None, enrol_expires_at: None };
        write_secret_json(&path, &credential).unwrap();
        let restored: RelayControlCredential = read_secret_json(&path).unwrap();
        assert_eq!(restored.control_token, credential.control_token);
        #[cfg(windows)]
        assert!(!fs::read(&path).unwrap().windows(credential.control_token.len()).any(|bytes| bytes == credential.control_token.as_bytes()));
        fs::write(&path, b"invalid-secret-sidecar").unwrap();
        assert!(read_secret_json::<RelayControlCredential>(&path).is_err());
    }

    fn v2_persisted(enabled: bool) -> PersistedPairing {
        PersistedPairing { version: 2, protocol_version: 2, enabled, l2_port: 443,
            control_url: "https://relay.example.test/api/desktop-connections/p1".into(),
            relay: "relay.example.test:18445".into(), pin: base64::engine::general_purpose::STANDARD.encode([7u8;32]),
            business_url: "https://desktop.example.test".into(), probe_url: "https://desktop.example.test".into(),
            invite_id: "p1".into(), tunnel_id: "t1".into(), tunnel_slug: "desktop-1".into(),
            agent_state_dir: AGENT_STATE_DIR_NAME.into() }
    }

    fn v2_control() -> RelayControlCredential {
        RelayControlCredential {
            control_url: "https://relay.example.test/api/desktop-connections/p1".into(),
            control_token: "test-control-never-in-public-state".into(),
            enrol_token: None,
            enrol_expires_at: None,
        }
    }

    fn raw_v2_envelope(invite: &str, expires_at: &str) -> String {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "bootstrap_url": "https://relay.example.test/api/bootstrap/desktop/v2",
            "invite": invite, "expires_at": expires_at,
        })).unwrap();
        format!("{ENVELOPE_V2_PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    }

    #[tokio::test]
    async fn precheck_bootstrap_rejects_a_second_pairing_without_touching_state() {
        let root = tempfile::tempdir().unwrap();
        write_persisted(&root.path().join(STATE_FILE_NAME), &v2_persisted(false)).unwrap();
        write_secret_json(&root.path().join("relay-control.secret"), &v2_control()).unwrap();
        let manager = RelayPairingManager::new(root.path().to_path_buf()).unwrap();
        let before = manager.runtime.lock().await.status.state;
        let (code, _) = manager.precheck_bootstrap("garbage").await.unwrap_err();
        assert_eq!(code, "RELAY_ALREADY_PAIRED");
        let status = manager.runtime.lock().await.status.clone();
        assert_eq!(status.state, before);
        assert_eq!(status.operation, None);
    }

    #[tokio::test]
    async fn precheck_bootstrap_maps_invalid_envelope_and_expired_invite() {
        let root = tempfile::tempdir().unwrap();
        let manager = RelayPairingManager::new(root.path().to_path_buf()).unwrap();
        let (code, _) = manager.precheck_bootstrap("not-a-pairing-envelope").await.unwrap_err();
        assert_eq!(code, "RELAY_ENVELOPE_INVALID");
        let (code, _) = manager
            .precheck_bootstrap(&raw_v2_envelope("opaque-invite", "2000-01-01T00:00:00Z"))
            .await
            .unwrap_err();
        assert_eq!(code, "RELAY_INVITE_EXPIRED");
    }

    #[tokio::test]
    async fn precheck_bootstrap_rejects_a_different_envelope_while_pending() {
        let root = tempfile::tempdir().unwrap();
        write_secret_json(&root.path().join("relay-bootstrap.pending.secret"), &PendingBootstrap {
            bootstrap_url: "https://relay.example.test/api/bootstrap/desktop/v2".into(),
            invite: "other-invite".into(), expires_at: "2999-01-01T00:00:00Z".into(),
            attempt_id: "a".repeat(64), enabled: true,
        }).unwrap();
        let manager = RelayPairingManager::new(root.path().to_path_buf()).unwrap();
        let (code, _) = manager
            .precheck_bootstrap(&raw_v2_envelope("opaque-invite", "2999-01-01T00:00:00Z"))
            .await
            .unwrap_err();
        assert_eq!(code, "RELAY_PENDING_EXISTS");
    }

    #[test]
    fn persisted_pause_survives_manager_restart_without_agent_or_lan() {
        let root = tempfile::tempdir().unwrap();
        let state = v2_persisted(false);
        write_persisted(&root.path().join(STATE_FILE_NAME), &state).unwrap();
        write_secret_json(&root.path().join("relay-control.secret"), &v2_control()).unwrap();
        let manager = RelayPairingManager::new(root.path().to_path_buf()).unwrap();
        let runtime = manager.runtime.try_lock().unwrap();
        assert_eq!(runtime.status.state, "paused");
        assert!(!runtime.status.enabled);
        assert!(runtime.process.is_none());
        assert_eq!(runtime.persisted.as_ref().unwrap().protocol_version, 2);
    }

    #[test]
    fn bootstrap_v2_accepts_shared_https_but_only_the_fixed_relay_service() {
        let mut value = serde_json::json!({
            "protocol_version": 2, "invite_id": "p1", "relay": "relay.example.test:18445",
            "business_url":"https://desktop.example.test", "probe_url":"https://desktop.example.test",
            "pin":base64::engine::general_purpose::STANDARD.encode([7u8;32]),
            "enrol_token":"opaque-enrol", "enrol_expires_at":"2999-01-01T00:00:00Z", "install_cmd":"ignored",
            "control_url":"https://relay.example.test/api/desktop-connections/p1", "control_token":format!("nf_desktop_{}", "a".repeat(64)),
            "state":"pending","reason":"waiting for agent",
            "l2_port":443,
            "tunnel":{"id":"t1","slug":"desktop-1","backend":{"kind":"addr","addr":V2_DESKTOP_BACKEND_ADDR}}
        });
        validate_bootstrap_response(&serde_json::from_value(value.clone()).unwrap()).unwrap();
        value["tunnel"]["backend"]["addr"] = Value::String(DESKTOP_BACKEND_ADDR.into());
        assert!(validate_bootstrap_response(&serde_json::from_value(value).unwrap()).is_err());
    }

    #[test]
    fn unsafe_bootstrap_urls_are_rejected() {
        for url in [
            "http://127.0.0.1:19445/api/bootstrap/desktop?invite=x",
            "http://127.0.0.1:19445/redirect?to=/api/bootstrap/desktop",
            "http://user:pass@127.0.0.1:19445/api/bootstrap/desktop",
            "ftp://127.0.0.1:19445/api/bootstrap/desktop",
        ] {
            let raw = envelope(serde_json::json!({
                "bootstrap_url": url,
                "invite": "invite-value",
                "expires_at": "2999-01-01T00:00:00Z"
            }));
            assert!(parse_pairing_envelope(&raw).is_err(), "{url}");
        }
    }

    #[test]
    fn response_backend_is_fixed_to_desktop_loopback() {
        let base = serde_json::json!({
            "invite_id": "pinv-1",
            "relay": "127.0.0.1:18445",
            "business_url": "https://desktop.example.com",
            "probe_url": "http://127.0.0.1:19092",
            "pin": base64::engine::general_purpose::STANDARD.encode([7u8; 32]),
            "enrol_token": "enrol-value",
            "enrol_expires_at": "2999-01-01T00:00:00Z",
            "install_cmd": "nfagent -relay example.invalid:18445 -token omitted",
            "tunnel": {
                "id": "t-desktop-1",
                "slug": "desktop-1",
                "listen_port": 19092,
                "backend": {"kind": "addr", "addr": DESKTOP_BACKEND_ADDR}
            }
        });
        let parsed: BootstrapResponse = serde_json::from_value(base.clone()).unwrap();
        validate_bootstrap_response(&parsed).unwrap();
        assert!(validate_business_url("https://user:pass@desktop.example.com").is_err());
        let mut unsafe_value = base;
        unsafe_value["tunnel"]["backend"]["addr"] = Value::String("169.254.169.254:80".into());
        let parsed: BootstrapResponse = serde_json::from_value(unsafe_value).unwrap();
        assert!(validate_bootstrap_response(&parsed).is_err());
    }

    #[test]
    fn qr_token_response_uses_standard_api_envelope() {
        let token = "a".repeat(64);
        let wrapped = serde_json::json!({
            "success": true,
            "data": {
                "ticket_id": "01900000-0000-7000-8000-000000000001",
                "qr_token": token,
                "expires_at_ms": 4_102_444_800_000_u64
            }
        });
        let parsed = parse_qr_token_response(&serde_json::to_vec(&wrapped).unwrap()).unwrap();
        assert_eq!(parsed.qr_token.len(), 64);
        assert_eq!(parsed.expires_at_ms, 4_102_444_800_000);

        let flat = serde_json::json!({
            "token": "a".repeat(64),
            "expires_at_ms": 4_102_444_800_000_u64
        });
        assert!(parse_qr_token_response(&serde_json::to_vec(&flat).unwrap()).is_err());
    }

    #[test]
    fn state_path_is_confined_to_data_root() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("relay-agent");
        fs::create_dir_all(&child).unwrap();
        ensure_child_path(root.path(), &child).unwrap();
        assert!(ensure_child_path(root.path(), &root.path().join("..")).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn state_path_accepts_the_macos_var_private_var_alias() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = dunce::canonicalize(root.path()).unwrap();
        assert_ne!(
            root.path(),
            canonical_root,
            "the macOS temporary directory should exercise /var -> /private/var"
        );

        ensure_child_path(root.path(), &root.path().join("relay-agent/cache")).unwrap();
        ensure_child_path(
            root.path(),
            &canonical_root.join("relay-agent/canonical-cache"),
        )
        .unwrap();
    }

    #[test]
    fn configured_nfagent_manifest_is_valid_for_this_target() {
        let spec = configured_nfagent_download_spec().unwrap();
        assert_eq!(spec.version, "0.1.0");
        assert_eq!(spec.sha256.len(), 64);
        assert!(spec.url.as_str().contains("/releases/download/nfagent-v0.1.0/"));
        assert!(!spec.url.as_str().to_ascii_lowercase().contains("/latest/"));

        let manifest: NfagentRuntimeManifest = serde_json::from_str(NFAGENT_MANIFEST).unwrap();
        for target in [
            "windows-amd64",
            "windows-arm64",
            "darwin-amd64",
            "darwin-arm64",
            "linux-amd64",
            "linux-arm64",
        ] {
            nfagent_download_spec_from_manifest(&manifest, target).unwrap();
        }
    }

    #[test]
    fn nfagent_manifest_rejects_invalid_hash_urls_versions_and_names() {
        let hash = "a".repeat(64);
        let valid_url = "https://github.com/nomifun/nomifun-net-infra/releases/download/nfagent-v0.1.0/nfagent-windows-amd64.exe";

        for manifest in [
            runtime_manifest("0.1.0", "windows-amd64", valid_url, "not-a-sha256"),
            runtime_manifest(
                "0.1.0",
                "windows-amd64",
                "http://example.invalid/nfagent-windows-amd64.exe",
                &hash,
            ),
            runtime_manifest(
                "0.1.0",
                "windows-amd64",
                "https://example.invalid/releases/latest/nfagent-windows-amd64.exe",
                &hash,
            ),
            runtime_manifest(
                "latest",
                "windows-amd64",
                "https://example.invalid/releases/v1/nfagent-windows-amd64.exe",
                &hash,
            ),
            runtime_manifest(
                "0.1.0",
                "windows-amd64",
                "https://example.invalid/releases/v1/nfagent.exe",
                &hash,
            ),
            runtime_manifest(
                "0.1.0",
                "windows-amd64",
                "https://user:password@example.invalid/releases/v1/nfagent-windows-amd64.exe",
                &hash,
            ),
            runtime_manifest(
                "0.1.0",
                "windows-amd64",
                "https://example.invalid/releases/v1/nfagent-windows-amd64.exe?mutable=1",
                &hash,
            ),
        ] {
            assert!(nfagent_download_spec_from_manifest(&manifest, "windows-amd64").is_err());
        }

        assert!(
            nfagent_download_spec_from_manifest(
                &runtime_manifest("0.1.0", "windows-amd64", valid_url, &hash),
                "linux-amd64",
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_nfagent_override_requires_approved_bytes_and_canonical_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let explicit = tmp.path().join("nfagent.exe");
        let bytes = b"approved fixture executable";
        fs::write(&explicit, bytes).unwrap();
        let spec = test_download_spec(
            "https://github.com/nomifun/nomifun-net-infra/releases/download/nfagent-v0.1.0/nfagent-windows-amd64.exe",
            bytes,
        );
        assert_eq!(
            validate_explicit_nfagent_path(&explicit, &spec).unwrap(),
            dunce::canonicalize(&explicit).unwrap()
        );
        assert!(validate_explicit_nfagent_path(&tmp.path().join("missing.exe"), &spec).is_err());
        assert!(validate_explicit_nfagent_path(Path::new("nfagent.exe"), &spec).is_err());
        fs::write(&explicit, b"unrelated same-name executable").unwrap();
        let error = validate_explicit_nfagent_path(&explicit, &spec).unwrap_err();
        assert!(error.to_string().contains("SHA-256"));
    }

    #[cfg(unix)]
    #[test]
    fn explicit_nfagent_symlink_preserves_only_the_approved_canonical_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = b"approved fixture executable";
        let executable = tmp.path().join("approved-agent");
        fs::write(&executable, bytes).unwrap();
        let alias = tmp.path().join("nfagent");
        std::os::unix::fs::symlink(&executable, &alias).unwrap();
        let spec = test_download_spec(
            "https://github.com/nomifun/nomifun-net-infra/releases/download/nfagent-v0.1.0/nfagent-windows-amd64.exe",
            bytes,
        );
        assert_eq!(
            validate_explicit_nfagent_path(&alias, &spec).unwrap(),
            dunce::canonicalize(executable).unwrap()
        );
    }

    #[tokio::test]
    async fn valid_nfagent_cache_is_reused_without_downloading() {
        let root = tempfile::tempdir().unwrap();
        let bytes = b"cached nfagent";
        let spec = test_download_spec(
            "http://127.0.0.1:9/nfagent-windows-amd64.exe",
            bytes,
        );
        let destination = root
            .path()
            .join(NFAGENT_RUNTIME_DIR_NAME)
            .join(NFAGENT_CACHE_DIR_NAME)
            .join(&spec.version)
            .join(&spec.sha256[..16])
            .join(&spec.file_name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, bytes).unwrap();
        ensure_executable(&destination).unwrap();

        let client = reqwest::Client::new();
        let resolved = prepare_nfagent_cached(root.path(), &spec, &client, None)
            .await
            .unwrap();
        assert_eq!(resolved, destination);
        assert_eq!(fs::read(resolved).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn valid_nfagent_cache_repairs_execute_permission_without_downloading() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bytes = b"cached nfagent";
        let spec = test_download_spec("http://127.0.0.1:9/nfagent-test", bytes);
        let destination = nfagent_cache_destination(root.path(), &spec).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, bytes).unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!is_executable(&destination).unwrap());
        let resolved = prepare_nfagent_cached(root.path(), &spec, &reqwest::Client::new(), None).await.unwrap();
        assert_eq!(resolved, destination);
        assert!(is_executable(&resolved).unwrap());
        assert_eq!(fs::read(resolved).unwrap(), bytes);
    }

    #[tokio::test]
    async fn automatic_nfagent_preparation_downloads_missing_cache() {
        let root = tempfile::tempdir().unwrap();
        let bytes = b"approved nfagent";
        let (url, server) = spawn_test_http_server(200, bytes).await;
        let spec = test_download_spec(&url, bytes);
        let client = reqwest::Client::new();

        let resolved = prepare_nfagent_cached(root.path(), &spec, &client, None)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(fs::read(resolved).unwrap(), bytes);
    }

    #[tokio::test]
    async fn corrupt_nfagent_cache_is_replaced_by_verified_download() {
        let root = tempfile::tempdir().unwrap();
        let bytes = b"replacement nfagent";
        let (url, server) = spawn_test_http_server(200, bytes).await;
        let spec = test_download_spec(&url, bytes);
        let destination = root
            .path()
            .join(NFAGENT_RUNTIME_DIR_NAME)
            .join(NFAGENT_CACHE_DIR_NAME)
            .join(&spec.version)
            .join(&spec.sha256[..16])
            .join(&spec.file_name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"corrupt").unwrap();

        let client = reqwest::Client::new();
        let resolved = prepare_nfagent_cached(root.path(), &spec, &client, None)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(resolved, destination);
        assert_eq!(fs::read(resolved).unwrap(), bytes);
    }

    #[tokio::test]
    async fn failed_nfagent_download_removes_staging_file() {
        let root = tempfile::tempdir().unwrap();
        let bytes = b"unexpected body";
        let (url, server) = spawn_test_http_server(200, bytes).await;
        let spec = test_download_spec(&url, b"expected body");

        let client = reqwest::Client::new();
        assert!(
            prepare_nfagent_cached(root.path(), &spec, &client, None)
                .await
                .is_err()
        );
        server.await.unwrap();

        let install_dir = root
            .path()
            .join(NFAGENT_RUNTIME_DIR_NAME)
            .join(NFAGENT_CACHE_DIR_NAME)
            .join(&spec.version)
            .join(&spec.sha256[..16]);
        assert!(!install_dir.join(&spec.file_name).exists());
        let leftovers = fs::read_dir(install_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert!(leftovers.is_empty(), "staging files remained: {leftovers:?}");
    }

    #[tokio::test]
    async fn operation_cancellation_stops_runtime_download_and_removes_staging() {
        let root = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut request).await;
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nabc").await.unwrap();
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        let spec = test_download_spec(&format!("http://{address}/nfagent-test"), b"abcdefghi");
        let directory = root.path().to_path_buf();
        let manager = RelayPairingManager::new(directory.clone()).unwrap();
        manager.begin_operation("bootstrap").await;
        let cancel_download = manager.op_interrupt.lock().unwrap().clone();
        let task = tokio::spawn(async move {
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            prepare_nfagent_cached(&directory, &spec, &client, Some(&cancel_download)).await
        });
        started_rx.await.unwrap();
        manager.interrupt_waits();
        assert!(tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap().is_err());
        server.abort();
        fn no_staged_files(path: &Path) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() { no_staged_files(&entry.path()); }
                else { assert!(!entry.file_name().to_string_lossy().ends_with(".part")); }
            }
        }
        no_staged_files(root.path());
    }

    #[test]
    fn unreadable_pending_bootstrap_reports_typed_state_error() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("relay-bootstrap.pending.secret"), b"not-a-protected-relay-secret").unwrap();
        let manager = RelayPairingManager::new(root.path().to_path_buf()).unwrap();
        let runtime = manager.runtime.try_lock().unwrap();
        assert_eq!(runtime.status.state, "error");
        assert_eq!(runtime.status.error_code, Some("RELAY_STATE_UNREADABLE"));
        assert!(runtime.status.pending_bootstrap);
        assert!(!runtime.status.enabled);
        assert!(runtime.state_unreadable);
    }

    #[test]
    fn unreadable_persisted_state_reports_typed_state_error() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(STATE_FILE_NAME), b"not-json").unwrap();
        let manager = RelayPairingManager::new(root.path().to_path_buf()).unwrap();
        let runtime = manager.runtime.try_lock().unwrap();
        assert_eq!(runtime.status.state, "error");
        assert_eq!(runtime.status.error_code, Some("RELAY_STATE_UNREADABLE"));
        assert!(runtime.state_unreadable);
        assert!(runtime.persisted.is_none());
    }

    #[test]
    fn valid_v2_state_without_readable_control_reports_typed_state_error() {
        let root = tempfile::tempdir().unwrap();
        write_persisted(&root.path().join(STATE_FILE_NAME), &v2_persisted(true)).unwrap();
        let manager = RelayPairingManager::new(root.path().to_path_buf()).unwrap();
        let runtime = manager.runtime.try_lock().unwrap();
        assert_eq!(runtime.status.state, "error");
        assert_eq!(runtime.status.error_code, Some("RELAY_STATE_UNREADABLE"));
        assert_eq!(runtime.status.relay.as_deref(), Some("relay.example.test:18445"));
        assert!(runtime.state_unreadable);
    }

    #[test]
    fn remove_local_files_clears_every_relay_artifact_and_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        write_persisted(&dir.join(STATE_FILE_NAME), &v2_persisted(true)).unwrap();
        fs::write(dir.join("relay-pairing.json.tmp"), b"partial-write").unwrap();
        write_secret_json(&dir.join("relay-control.secret"), &v2_control()).unwrap();
        write_secret_json(&dir.join("relay-bootstrap.pending.secret"), &PendingBootstrap {
            bootstrap_url: "https://relay.example.test/api/bootstrap/desktop/v2".into(),
            invite: "opaque-invite".into(), expires_at: "2999-01-01T00:00:00Z".into(),
            attempt_id: "a".repeat(64), enabled: true,
        }).unwrap();
        fs::create_dir_all(dir.join(AGENT_STATE_DIR_NAME)).unwrap();
        fs::write(dir.join(AGENT_STATE_DIR_NAME).join("credential.json"), b"{}").unwrap();
        let manager = RelayPairingManager::new(dir.to_path_buf()).unwrap();
        manager.remove_local_files().unwrap();
        for path in [
            dir.join(STATE_FILE_NAME),
            dir.join("relay-pairing.json.tmp"),
            dir.join("relay-control.secret"),
            dir.join("relay-bootstrap.pending.secret"),
            dir.join(AGENT_STATE_DIR_NAME),
        ] {
            assert!(!path.exists(), "local forget left {}", path.display());
        }
        manager.remove_local_files().unwrap();
    }

    #[test]
    fn apply_health_clears_only_stale_readiness_failure_on_recovery() {
        let mut status = RelayPairingStatus {
            state: "reconnecting",
            error_code: Some("RELAY_TUNNEL_UNREACHABLE"),
            action_error_code: Some("RELAY_AGENT_NOT_READY"),
            ..Default::default()
        };
        apply_health(&mut status, RelayHealth { backend_reachable: true, tunnel_reachable: true, checked_at_ms: Some(1) });
        assert_eq!(status.state, "connected");
        assert_eq!(status.error, None);
        assert_eq!(status.error_code, None);
        assert_eq!(status.action_error_code, None);

        let mut status = RelayPairingStatus {
            state: "reconnecting",
            error_code: Some("RELAY_TUNNEL_UNREACHABLE"),
            action_error_code: Some("RELAY_REVOKE_UNCONFIRMED"),
            ..Default::default()
        };
        apply_health(&mut status, RelayHealth { backend_reachable: true, tunnel_reachable: true, checked_at_ms: Some(1) });
        assert_eq!(status.state, "connected");
        assert_eq!(status.error_code, None);
        assert_eq!(status.action_error_code, Some("RELAY_REVOKE_UNCONFIRMED"));

        let mut status = RelayPairingStatus {
            state: "connected",
            action_error_code: Some("RELAY_REVOKE_UNCONFIRMED"),
            ..Default::default()
        };
        apply_health(&mut status, RelayHealth { backend_reachable: true, tunnel_reachable: false, checked_at_ms: Some(1) });
        assert_eq!(status.state, "reconnecting");
        assert_eq!(status.error_code, Some("RELAY_TUNNEL_UNREACHABLE"));
        assert_eq!(status.action_error_code, Some("RELAY_REVOKE_UNCONFIRMED"));
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_preempts_a_gated_operation_waiting_on_relay() {
        let root = tempfile::tempdir().unwrap();
        let manager = Arc::new(RelayPairingManager::new(root.path().to_path_buf()).unwrap());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let worker = {
            let manager = manager.clone();
            tokio::spawn(async move {
                let _operation = manager.operation_gate.lock().await;
                manager.begin_operation("restore").await;
                let _ = ready_tx.send(());
                manager.interruptible(std::future::pending::<Result<()>>()).await
            })
        };
        ready_rx.await.unwrap();
        let started = Instant::now();
        manager.shutdown().await.unwrap();
        assert!(started.elapsed() < SHUTDOWN_OPERATION_GRACE);
        assert!(worker.await.unwrap().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_bounds_the_wait_behind_a_deaf_operation() {
        let root = tempfile::tempdir().unwrap();
        let manager = Arc::new(RelayPairingManager::new(root.path().to_path_buf()).unwrap());
        let _operation = manager.operation_gate.lock().await;
        let started = Instant::now();
        manager.shutdown().await.unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed >= SHUTDOWN_OPERATION_GRACE);
        assert!(elapsed < SHUTDOWN_OPERATION_GRACE + Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_operation_supersedes_the_previous_wait_and_then_runs() {
        let root = tempfile::tempdir().unwrap();
        let manager = Arc::new(RelayPairingManager::new(root.path().to_path_buf()).unwrap());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let superseded = {
            let manager = manager.clone();
            tokio::spawn(async move {
                let _operation = manager.operation_gate.lock().await;
                manager.begin_operation("restart").await;
                let _ = ready_tx.send(());
                manager.interruptible(std::future::pending::<Result<()>>()).await
            })
        };
        ready_rx.await.unwrap();
        manager.interrupt_waits();
        assert!(superseded.await.unwrap().is_err());
        let fresh = {
            let manager = manager.clone();
            tokio::spawn(async move {
                let _operation = manager.operation_gate.lock().await;
                manager.begin_operation("stop").await;
                manager.interruptible(async { Ok(()) }).await
            })
        };
        assert!(fresh.await.unwrap().is_ok());
        manager.interrupt_waits();
        assert!(manager.until_shutdown(async { Ok(()) }).await.is_ok());
    }
}
