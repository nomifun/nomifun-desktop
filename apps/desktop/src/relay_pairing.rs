//! Relay pairing for the native Desktop shell.
//!
//! The relay console gives Desktop a short-lived, one-shot envelope.  This
//! module is deliberately the only place that handles that bearer:
//! - parse and validate the envelope without accepting alternate credential
//!   shapes;
//! - bootstrap the fixed Desktop backend through the Relay;
//! - install/verify the managed nfagent and run it with the shared
//!   process-runtime owner;
//! - persist only restart-safe, non-secret state;
//! - mint the final one-shot `nomi://pair` URL from the Desktop QR endpoint.

use std::fs;
use std::io::{self, Read};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
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
const STATE_VERSION: u8 = 1;
const STATE_FILE_NAME: &str = "relay-pairing.json";
const AGENT_STATE_DIR_NAME: &str = "relay-agent";
const DESKTOP_BACKEND_ADDR: &str = "127.0.0.1:25808";
const REQUIRED_DESKTOP_PORT: u16 = 25808;
const AGENT_POLL_TIMEOUT: Duration = Duration::from_secs(45);
const RESTORE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const QR_REFRESH_SKEW_MS: u64 = 30_000;
const NFAGENT_RUNTIME_DIR_NAME: &str = "runtime";
const NFAGENT_CACHE_DIR_NAME: &str = "nfagent";
const NFAGENT_MAX_DOWNLOAD_BYTES: u64 = 32 * 1024 * 1024;
const NFAGENT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const NFAGENT_RUNTIME_VERSION_MAX_LEN: usize = 64;
const NFAGENT_FILE_NAME_MAX_LEN: usize = 128;
const NFAGENT_MANIFEST: &str = include_str!("../nfagent-runtime.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingEnvelope {
    bootstrap_url: String,
    invite: String,
    expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPairing {
    version: u8,
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
    #[serde(rename = "pairUrl")]
    pub pairing_url: Option<String>,
    pub qr_expires_at_ms: Option<u64>,
    pub webui_port: Option<u16>,
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
            pairing_url: None,
            qr_expires_at_ms: None,
            webui_port: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapResponse {
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
    // embedded enrol token is never logged or persisted.
    #[serde(rename = "install_cmd")]
    _install_cmd: String,
    tunnel: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct QrTokenResponse {
    token: String,
    expires_at_ms: u64,
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
}

/// Native pairing state.  The process slot is polled rather than awaited while
/// holding the mutex, so a stop/restart command can always take ownership and
/// perform the managed tree cleanup.
pub struct RelayPairingManager {
    data_dir: PathBuf,
    state_path: PathBuf,
    agent_state_dir: PathBuf,
    client: reqwest::Client,
    download_client: reqwest::Client,
    nfagent_install_lock: Mutex<()>,
    runtime: Mutex<PairingRuntime>,
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

        let persisted = load_persisted(&state_path)?;
        let mut status = RelayPairingStatus::default();
        if let Some(state) = &persisted {
            status.state = "connecting";
            status.relay = Some(state.relay.clone());
            status.business_url = Some(state.business_url.clone());
            status.invite_id = Some(state.invite_id.clone());
            status.tunnel_id = Some(state.tunnel_id.clone());
            status.tunnel_slug = Some(state.tunnel_slug.clone());
            status.agent_state_dir = Some(agent_state_dir.display().to_string());
        }

        Ok(Self {
            data_dir,
            state_path,
            agent_state_dir,
            client,
            download_client,
            nfagent_install_lock: Mutex::new(()),
            runtime: Mutex::new(PairingRuntime {
                persisted,
                process: None,
                status,
                generation: 0,
            }),
        })
    }

    /// Restore a previously paired agent after the embedded backend is ready.
    /// This is intentionally best-effort and non-blocking from the desktop
    /// startup supervisor: nfagent itself has a reconnect loop.
    pub async fn restore(self: Arc<Self>, server: Arc<DesktopServer>) {
        let state = {
            let guard = self.runtime.lock().await;
            guard.persisted.clone()
        };
        let Some(state) = state else {
            return;
        };

        if let Err(error) = self.ensure_webui_port(&server).await {
            self.set_error(format!("Relay 配对状态已保存，但 WebUI 端口未就绪：{error}"))
                .await;
            return;
        }

        let nfagent_path = match self.ensure_nfagent_installed().await {
            Ok(path) => path,
            Err(error) => {
                self.set_error(format!("恢复 Relay agent 失败：{error:#}"))
                    .await;
                return;
            }
        };
        if let Err(error) = self
            .start_agent(state.clone(), None, nfagent_path)
            .await
        {
            self.set_error(format!("恢复 Relay agent 失败：{error:#}")).await;
            return;
        }
        if let Err(error) = self.wait_for_agent_ready(&state).await {
            self.set_error(format!("Relay agent 正在重连：{error}")).await;
            return;
        }
        self.mark_connected(&state).await;
        if let Err(error) = self.refresh_pairing_url(&server, &state).await {
            self.set_error(format!("Relay agent 已恢复，但 Desktop QR 暂不可用：{error}"))
                .await;
        }
    }

    pub async fn status(&self, server: Arc<DesktopServer>) -> Result<RelayPairingStatus> {
        let (persisted, mut status) = {
            let guard = self.runtime.lock().await;
            (guard.persisted.clone(), guard.status.clone())
        };
        let Some(persisted) = persisted else {
            return Ok(status);
        };

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

        if status.state == "connected" {
            let now_ms = Utc::now().timestamp_millis().max(0) as u64;
            let cached_is_fresh = status.pairing_url.is_some()
                && status
                    .qr_expires_at_ms
                    .is_some_and(|expires| expires > now_ms.saturating_add(QR_REFRESH_SKEW_MS));
            if !cached_is_fresh {
                let _ = self.refresh_pairing_url(&server, &persisted).await;
                status = {
                    let guard = self.runtime.lock().await;
                    guard.status.clone()
                };
            }
        }
        Ok(status)
    }

    pub async fn bootstrap(
        self: &Arc<Self>,
        server: Arc<DesktopServer>,
        raw_envelope: &str,
    ) -> Result<RelayPairingStatus> {
        let envelope = parse_pairing_envelope(raw_envelope)?;
        validate_envelope_expiry(&envelope)?;

        {
            let guard = self.runtime.lock().await;
            if guard.persisted.is_some() || guard.process.is_some() {
                anyhow::bail!("Desktop 已存在 Relay 配对；请先断开现有配对");
            }
        }

        self.ensure_webui_port(&server).await?;
        // Install/verify the runtime before exchanging the one-shot invite.
        // A failed download must never consume a pairing invitation.
        let nfagent_path = self.ensure_nfagent_installed().await?;
        reset_agent_state_dir(&self.data_dir, &self.agent_state_dir)?;

        self.set_status(RelayPairingStatus {
            state: "connecting",
            error: None,
            webui_port: Some(REQUIRED_DESKTOP_PORT),
            ..Default::default()
        })
        .await;

        let response = self.exchange_bootstrap(&envelope).await?;
        validate_bootstrap_response(&response)?;
        if response.enrol_expires_at <= Utc::now() {
            anyhow::bail!("Relay 返回的 agent 入网令牌已过期");
        }

        let persisted = PersistedPairing {
            version: STATE_VERSION,
            relay: response.relay.clone(),
            pin: response.pin.clone(),
            business_url: response.business_url.clone(),
            probe_url: response.probe_url.clone(),
            invite_id: response.invite_id.clone(),
            tunnel_id: tunnel_string(&response.tunnel, "id")?,
            tunnel_slug: tunnel_string(&response.tunnel, "slug")?,
            agent_state_dir: AGENT_STATE_DIR_NAME.to_owned(),
        };

        self.set_status(status_from_persisted(&persisted, "connecting", None))
            .await;
        if let Err(error) = self
            .start_agent(
                persisted.clone(),
                Some(response.enrol_token),
                nfagent_path,
            )
            .await
        {
            self.set_error(format!(
                "Relay 已消费配对邀请，但 agent 启动失败：{error:#}。请重新生成配对串。"
            ))
            .await;
            return Err(error);
        }

        if let Err(error) = self.wait_for_agent_ready(&persisted).await {
            let _ = self.stop_agent().await;
            self.set_error(format!(
                "Relay 已消费配对邀请，但 agent 未能上线：{error}。请重新生成配对串。"
            ))
            .await;
            anyhow::bail!(
                "Relay bootstrap succeeded but nfagent did not become ready: {error:#}"
            );
        }

        self.mark_connected(&persisted).await;
        write_persisted(&self.state_path, &persisted)?;
        {
            let mut guard = self.runtime.lock().await;
            guard.persisted = Some(persisted.clone());
        }

        self.refresh_pairing_url(&server, &persisted).await?;
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
        let persisted = {
            let guard = self.runtime.lock().await;
            guard
                .persisted
                .clone()
                .context("没有可恢复的 Relay 配对状态")?
        };
        ensure_child_path(&self.data_dir, &self.agent_state_dir)?;
        prepare_agent_state_dir(&self.agent_state_dir)?;
        if !self.agent_state_dir.join("credential.json").is_file() {
            anyhow::bail!("agent 长期凭据不存在，不能安全重启；请重新配对");
        }
        self.ensure_webui_port(&server).await?;
        let nfagent_path = self.ensure_nfagent_installed().await?;
        self.stop_agent().await?;
        self.start_agent(persisted.clone(), None, nfagent_path)
            .await?;
        if let Err(error) = self.wait_for_agent_ready(&persisted).await {
            let _ = self.stop_agent().await;
            self.set_error(format!("重启 Relay agent 后未能恢复连接：{error}"))
                .await;
            anyhow::bail!("Relay agent restart did not become ready: {error:#}");
        }
        self.mark_connected(&persisted).await;
        self.refresh_pairing_url(&server, &persisted).await?;
        let status = {
            let guard = self.runtime.lock().await;
            guard.status.clone()
        };
        self.set_status(status.clone()).await;
        Ok(status)
    }

    pub async fn disconnect(&self) -> Result<RelayPairingStatus> {
        self.stop_agent().await?;
        let target = self.agent_state_dir.clone();
        ensure_child_path(&self.data_dir, &target)?;
        if self.state_path.exists() {
            tokio::fs::remove_file(&self.state_path)
                .await
                .with_context(|| format!("remove pairing state {}", self.state_path.display()))?;
        }
        if target.exists() {
            tokio::fs::remove_dir_all(&target)
                .await
                .with_context(|| format!("remove agent state directory {}", target.display()))?;
        }
        let status = RelayPairingStatus::default();
        let mut guard = self.runtime.lock().await;
        guard.persisted = None;
        guard.status = status.clone();
        Ok(status)
    }

    /// Stop the managed agent while retaining pairing metadata and
    /// `credential.json` for an explicit restart.
    pub async fn stop(&self) -> Result<RelayPairingStatus> {
        self.stop_agent().await?;
        let mut guard = self.runtime.lock().await;
        let mut status = guard.status.clone();
        status.state = "disconnected";
        status.agent_pid = None;
        status.pairing_url = None;
        status.qr_expires_at_ms = None;
        status.error = None;
        guard.status = status.clone();
        Ok(status)
    }

    async fn ensure_webui_port(&self, server: &Arc<DesktopServer>) -> Result<()> {
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
        let response = self
            .client
            .post(&envelope.bootstrap_url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", envelope.invite))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .context("请求 Relay bootstrap 失败")?;

        if response.status().is_redirection() {
            anyhow::bail!("Relay bootstrap 不允许 HTTP 重定向");
        }
        if response.status() != StatusCode::OK {
            anyhow::bail!(
                "Relay bootstrap 返回 HTTP {}（配对邀请可能已被消费，请重新生成）",
                response.status()
            );
        }
        response
            .json::<BootstrapResponse>()
            .await
            .context("Relay bootstrap 响应格式无效")
    }

    async fn ensure_nfagent_installed(&self) -> Result<PathBuf> {
        if let Some(path) = explicit_nfagent_path()? {
            return Ok(path);
        }

        let spec = configured_nfagent_download_spec()?;
        let _install_guard = self.nfagent_install_lock.lock().await;
        ensure_nfagent_cached(&self.data_dir, &spec, &self.download_client).await
    }
}

async fn ensure_nfagent_cached(
    data_dir: &Path,
    spec: &NfagentDownloadSpec,
    client: &reqwest::Client,
) -> Result<PathBuf> {
    if !is_sha256_hex(&spec.sha256) {
        anyhow::bail!("nfagent runtime SHA-256 is invalid");
    }
    let install_dir = data_dir
        .join(NFAGENT_RUNTIME_DIR_NAME)
        .join(NFAGENT_CACHE_DIR_NAME)
        .join(cache_component(&spec.version))
        .join(&spec.sha256[..16]);
    let destination = install_dir.join(&spec.file_name);
    ensure_child_path(data_dir, &install_dir)?;
    ensure_child_path(data_dir, &destination)?;

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
    let result = download_verified_nfagent(client, spec, &temp_path, &destination).await;
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
    async fn start_agent(
        self: &Arc<Self>,
        state: PersistedPairing,
        enrol_token: Option<String>,
        path: PathBuf,
    ) -> Result<()> {
        ensure_child_path(&self.data_dir, &self.agent_state_dir)?;
        prepare_agent_state_dir(&self.agent_state_dir)?;
        let mut builder = ChildProcessBuilder::new(path);
        builder
            .args(["-relay", state.relay.as_str()])
            .args(["-pin", state.pin.as_str()])
            .args(["-state-dir", self.agent_state_dir.to_string_lossy().as_ref()])
            .args(["-l2-port", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
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
        guard.status.agent_pid = pid;
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
                let mut process = {
                    let mut guard = self.runtime.lock().await;
                    if guard.generation != generation {
                        return;
                    }
                    guard.process.take()
                };
                if let Some(ref mut process) = process {
                    let _ = process.shutdown().await;
                }
                let mut guard = self.runtime.lock().await;
                if guard.generation == generation {
                    guard.status.state = "error";
                    guard.status.agent_pid = None;
                    guard.status.error =
                        Some("nfagent 已退出；可点击“重启 agent”恢复连接".to_owned());
                }
                return;
            }
            sleep(RESTORE_POLL_INTERVAL).await;
        }
    }

    async fn stop_agent(&self) -> Result<()> {
        let mut process = {
            let mut guard = self.runtime.lock().await;
            guard.generation = guard.generation.wrapping_add(1);
            guard.process.take()
        };
        if let Some(ref mut process) = process {
            process.shutdown().await.context("停止 nfagent 失败")?;
        }
        Ok(())
    }

    async fn wait_for_agent_ready(&self, state: &PersistedPairing) -> Result<()> {
        let probe_url = if state.probe_url.trim().is_empty() {
            &state.business_url
        } else {
            &state.probe_url
        };
        let endpoint = auth_status_url(probe_url)?;
        let deadline = Instant::now() + AGENT_POLL_TIMEOUT;
        loop {
            let credential_ready = self.agent_state_dir.join("credential.json").is_file();
            let http_ready = match self.client.get(endpoint.clone()).send().await {
                Ok(response) => response.status() == StatusCode::OK,
                Err(_) => false,
            };
            if credential_ready && http_ready {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "等待 tunnel 就绪超时（credential.json={}，HTTP={}）",
                    credential_ready,
                    http_ready
                );
            }
            sleep(Duration::from_millis(300)).await;
        }
    }

    async fn mint_final_pairing_url(
        &self,
        server: &Arc<DesktopServer>,
        state: &PersistedPairing,
    ) -> Result<(String, u64)> {
        let endpoint = format!(
            "http://127.0.0.1:{}/api/webui/generate-qr-token",
            server.loopback_port()
        );
        let response = self
            .client
            .post(endpoint)
            .header("x-nomi-local-trust", server.local_trust_secret())
            .body("")
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
        if !is_hex_token(&qr.token) {
            anyhow::bail!("Desktop QR token 格式无效");
        }
        let nested = format!(
            "{}/qr-login?token={}",
            state.business_url.trim_end_matches('/'),
            qr.token
        );
        let encoded: String = percent_encode_pairing_url(&nested);
        Ok((format!("nomi://pair?v=1&url={encoded}"), qr.expires_at_ms))
    }

    async fn set_status(&self, status: RelayPairingStatus) {
        let mut guard = self.runtime.lock().await;
        guard.status = status;
    }

    async fn set_error(&self, message: String) {
        let mut guard = self.runtime.lock().await;
        guard.status.state = "error";
        guard.status.error = Some(message);
        guard.status.agent_pid = guard.process.as_ref().and_then(ManagedChildProcess::id);
    }

    async fn refresh_pairing_url(
        &self,
        server: &Arc<DesktopServer>,
        state: &PersistedPairing,
    ) -> Result<()> {
        let (pairing_url, expires) = self.mint_final_pairing_url(server, state).await?;
        let mut guard = self.runtime.lock().await;
        guard.status.pairing_url = Some(pairing_url);
        guard.status.qr_expires_at_ms = Some(expires);
        guard.status.webui_port = Some(REQUIRED_DESKTOP_PORT);
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
        status.webui_port = Some(REQUIRED_DESKTOP_PORT);
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
    let encoded = raw
        .strip_prefix(ENVELOPE_PREFIX)
        .context("不是受支持的 Relay Desktop pairing 串")?;
    if encoded.is_empty() || encoded.contains('=') {
        anyhow::bail!("Relay pairing 串编码无效");
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .context("Relay pairing 串不是合法 base64url")?;
    if bytes.len() > 16 * 1024 {
        anyhow::bail!("Relay pairing 串过大");
    }
    let envelope: PairingEnvelope =
        serde_json::from_slice(&bytes).context("Relay pairing 串 JSON 无效")?;
    validate_bootstrap_url(&envelope.bootstrap_url)?;
    if envelope.invite.trim().is_empty() || envelope.invite.len() > 4096 {
        anyhow::bail!("Relay pairing 串 invite 无效");
    }
    Ok(envelope)
}

fn validate_envelope_expiry(envelope: &PairingEnvelope) -> Result<()> {
    let expires = DateTime::parse_from_rfc3339(&envelope.expires_at)
        .context("Relay pairing 串过期时间无效")?
        .with_timezone(&Utc);
    if expires <= Utc::now() {
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
    if url.path().trim_end_matches('/') != "/api/bootstrap/desktop" {
        anyhow::bail!("bootstrap URL 路径不是 /api/bootstrap/desktop");
    }
    Ok(())
}

fn validate_bootstrap_response(response: &BootstrapResponse) -> Result<()> {
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
        || backend.get("addr").and_then(Value::as_str) != Some(DESKTOP_BACKEND_ADDR)
    {
        anyhow::bail!("Relay tunnel backend 不是固定 Desktop loopback");
    }
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
        ..Default::default()
    }
}

fn load_persisted(path: &Path) -> Result<Option<PersistedPairing>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("读取配对状态 {}", path.display()))?;
    let state: PersistedPairing =
        serde_json::from_slice(&bytes).context("Relay 配对状态格式无效")?;
    if state.version != STATE_VERSION
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

fn write_persisted(path: &Path, state: &PersistedPairing) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state)?;
    fs::write(&temp, bytes).with_context(|| format!("写入临时配对状态 {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("提交配对状态 {}", path.display()))?;
    Ok(())
}

fn prepare_agent_state_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("创建 agent 状态目录 {}", path.display()))?;
    let probe = path.join(".write-probe");
    fs::write(&probe, b"ok").with_context(|| format!("写入 agent 状态目录 {}", path.display()))?;
    let _ = fs::remove_file(probe);
    Ok(())
}

fn reset_agent_state_dir(root: &Path, path: &Path) -> Result<()> {
    ensure_child_path(root, path)?;
    if path.exists() {
        fs::remove_dir_all(path)
            .with_context(|| format!("清理旧 agent 状态目录 {}", path.display()))?;
    }
    prepare_agent_state_dir(path)
}

fn ensure_child_path(root: &Path, child: &Path) -> Result<()> {
    let root = dunce::canonicalize(root)
        .with_context(|| format!("canonicalize data directory {}", root.display()))?;
    let child = if child.is_absolute() {
        child.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve relative managed path")?
            .join(child)
    };
    if !child.starts_with(&root) || child == root {
        anyhow::bail!("拒绝操作数据目录之外的 agent 状态路径");
    }

    let relative = child
        .strip_prefix(&root)
        .context("resolve managed path relative to data directory")?;
    let mut current = root.clone();
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
                if !canonical.starts_with(&root) {
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
    Ok(Some(validate_explicit_nfagent_path(&PathBuf::from(raw))?))
}

fn validate_explicit_nfagent_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || !path.is_file() {
        anyhow::bail!("NOMIFUN_NFAGENT_PATH 必须是存在的绝对路径");
    }
    ensure_executable(path)?;
    Ok(path.to_path_buf())
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
                "token": token,
                "expires_at_ms": 4_102_444_800_000_u64
            }
        });
        let parsed = parse_qr_token_response(&serde_json::to_vec(&wrapped).unwrap()).unwrap();
        assert_eq!(parsed.token.len(), 64);
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
    fn explicit_nfagent_override_requires_an_existing_absolute_file() {
        let root = tempfile::tempdir().unwrap();
        let explicit = root.path().join("nfagent.exe");
        fs::write(&explicit, b"explicit").unwrap();
        assert_eq!(
            validate_explicit_nfagent_path(&explicit).unwrap(),
            explicit
        );

        let missing = root.path().join("missing.exe");
        assert!(validate_explicit_nfagent_path(&missing).is_err());
        assert!(validate_explicit_nfagent_path(Path::new("nfagent.exe")).is_err());
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

        let client = reqwest::Client::new();
        let resolved = ensure_nfagent_cached(root.path(), &spec, &client)
            .await
            .unwrap();
        assert_eq!(resolved, destination);
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
        let resolved = ensure_nfagent_cached(root.path(), &spec, &client)
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
            ensure_nfagent_cached(root.path(), &spec, &client)
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
}
