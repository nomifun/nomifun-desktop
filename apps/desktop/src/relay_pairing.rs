//! Relay pairing for the native Desktop shell.
//!
//! The relay console gives Desktop a short-lived, one-shot envelope.  This
//! module is deliberately the only place that handles that bearer:
//! - parse and validate the envelope without accepting alternate credential
//!   shapes;
//! - bootstrap the fixed Desktop backend through the Relay;
//! - run the bundled/explicit nfagent with the shared process-runtime owner;
//! - persist only restart-safe, non-secret state;
//! - mint the final one-shot `nomi://pair` URL from the Desktop QR endpoint.

use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
            runtime: Mutex::new(PairingRuntime {
                persisted,
                process: None,
                status,
                generation: 0,
            }),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
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

        if let Err(error) = self.start_agent(state.clone(), None).await {
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

        reset_agent_state_dir(&self.data_dir, &self.agent_state_dir)?;
        self.ensure_webui_port(&server).await?;

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
            .start_agent(persisted.clone(), Some(response.enrol_token))
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
        prepare_agent_state_dir(&self.agent_state_dir)?;
        if !self.agent_state_dir.join("credential.json").is_file() {
            anyhow::bail!("agent 长期凭据不存在，不能安全重启；请重新配对");
        }
        self.ensure_webui_port(&server).await?;
        self.stop_agent().await?;
        self.start_agent(persisted.clone(), None).await?;
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

    async fn start_agent(
        self: &Arc<Self>,
        state: PersistedPairing,
        enrol_token: Option<String>,
    ) -> Result<()> {
        let path = resolve_nfagent_path()?;
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
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let child = dunce::canonicalize(child).unwrap_or_else(|_| child.to_path_buf());
    if !child.starts_with(&root) || child == root {
        anyhow::bail!("拒绝操作数据目录之外的 agent 状态路径");
    }
    Ok(())
}

fn resolve_nfagent_path() -> Result<PathBuf> {
    if let Ok(raw) = std::env::var("NOMIFUN_NFAGENT_PATH") {
        let path = PathBuf::from(raw);
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        anyhow::bail!("NOMIFUN_NFAGENT_PATH 必须是存在的绝对路径");
    }
    let exe = std::env::current_exe().context("解析 Desktop 可执行文件路径失败")?;
    let resource_dir = std::env::var_os("NOMIFUN_RESOURCE_DIR").map(PathBuf::from);
    resolve_nfagent_from_locations(
        None,
        resource_dir.as_deref(),
        exe.parent(),
        Some(Path::new(".")),
    )
}

fn resolve_nfagent_from_locations(
    explicit: Option<&Path>,
    resource_dir: Option<&Path>,
    exe_dir: Option<&Path>,
    cwd: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if path.is_absolute() && path.is_file() {
            return Ok(path.to_path_buf());
        }
        anyhow::bail!("NOMIFUN_NFAGENT_PATH 必须是存在的绝对路径");
    }
    let candidates = [
        resource_dir.map(|path| path.join("nfagent").join("nfagent.exe")),
        resource_dir.map(|path| path.join("nfagent").join("nfagent")),
        resource_dir.map(|path| path.join("nfagent.exe")),
        resource_dir.map(|path| path.join("nfagent")),
        exe_dir.map(|path| path.join("nfagent.exe")),
        exe_dir.map(|path| path.join("nfagent")),
        cwd.map(|path| path.join("nfagent.exe")),
        cwd.map(|path| path.join("nfagent")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
        .context("找不到 nfagent；请设置 NOMIFUN_NFAGENT_PATH")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn nfagent_resolution_prefers_packaged_resource() {
        let root = tempfile::tempdir().unwrap();
        let resource_dir = root.path().join("resources");
        let exe_dir = root.path().join("bin");
        fs::create_dir_all(resource_dir.join("nfagent")).unwrap();
        fs::create_dir_all(&exe_dir).unwrap();
        let bundled = resource_dir.join("nfagent").join("nfagent.exe");
        let adjacent = exe_dir.join("nfagent.exe");
        fs::write(&bundled, b"bundled").unwrap();
        fs::write(&adjacent, b"adjacent").unwrap();

        let resolved =
            resolve_nfagent_from_locations(None, Some(&resource_dir), Some(&exe_dir), None)
                .unwrap();
        assert_eq!(resolved, bundled);
    }

    #[test]
    fn nfagent_resolution_validates_explicit_override() {
        let root = tempfile::tempdir().unwrap();
        let relative = Path::new("nfagent.exe");
        assert!(
            resolve_nfagent_from_locations(Some(relative), Some(root.path()), None, None).is_err()
        );
        let explicit = root.path().join("nfagent.exe");
        fs::write(&explicit, b"explicit").unwrap();
        let resolved =
            resolve_nfagent_from_locations(Some(&explicit), Some(root.path()), None, None).unwrap();
        assert_eq!(resolved, explicit);
    }
}
