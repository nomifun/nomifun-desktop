# Plan A — 机器人网关后端（nomifun-robot） Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建 `nomifun-robot` crate，让 ESP32 机器人（xiaozhi 固件）通过局域网直连桌面应用，作为某个桌面伙伴的物理化身完成语音对话、表情联动与云台控制。

**Architecture:** 一个内嵌 axum 子路由（`/robot/*`，挂在会话鉴权之外）+ 每设备一个 `RobotSession` actor。字节来源被抽象为 `RobotLink`/`RobotLinkSource`（本期只实现局域网 WS，未来中继复用同一接口）；模型能力被抽象为 `SpeechServices`/`CompanionTurnDispatcher` 两个 trait seam，核心管线因此可用 mock 完整测试，真实接线放在同 crate 的 `wiring/` 模块。

**Tech Stack:** Rust 2024 / axum 0.8（ws feature 已开）/ tokio / audiopus（libopus 绑定，Opus 编解码）/ ort（onnxruntime，Silero VAD）/ symphonia（音频容器解码）/ serde / nomifun-model-invoke（ASR/TTS）/ nomi-providers（视觉）/ nomi-mcp（设备工具桥）

## Global Constraints

- **Git 署名**：绝不让 AI 出现在 author / committer / co-author；绝不添加 `Co-authored-by`、`Generated-by`、`Assisted-by` 等 trailer；不得使用 `--no-verify`。作者与提交者统一为 `NomiFun Contributor <nomifun@users.noreply.github.com>`。
- **绝不**在 `.github/workflows/` 下创建、恢复或重命名任何文件。
- commit message 用英文 conventional commits；本计划正文为中文，代码、标识符、commit message 为英文。
- **测试范围**：本仓库 Linux 全量测试有 14 个既有失败，且全量需 `--features nomifun-ai-agent/test-support` 并限制并发。**任务内只跑目标包**：`cargo test -p nomifun-robot`。禁止在任务步骤里跑全量测试。
- **固件协议硬约束（违反即设备不工作）**：
  - OTA 响应**永远**包含 `websocket` 对象；**绝不**包含 `mqtt` 键（只要 `mqtt` 是对象，固件就选中 MQTT 且无回落）。
  - 上行音频硬编码 Opus / 16000 Hz / 单声道 / 60 ms，**不可协商**。
  - 服务端 hello 声明下行 `sample_rate: 24000`、`frame_duration: 60`、`format: "opus"`、`channels: 1`。
  - 二进制帧用 v1 = **裸 Opus 负载，无任何头部字节**。
  - 不发 `{"type":"tts","state":"start"}` 时，设备丢弃所有下行音频包。
  - 下行必须按实时节奏 pacing（约每 60 ms 一帧）：设备解码队列仅 40 包（约 2.4 秒），**满则静默丢包**。
  - 收到设备 `abort` 后必须**立即**停止推流并下发 `tts stop`（设备本地不清播放队列，否则有约 2.4 秒拖尾）。
  - 设备 120 秒无入站消息判超时 → 网关每 60 秒发一条 `{"type":"ping"}`（固件对未知 type 仅记日志）。
  - 设备 MCP server：请求 `id` **必须是数字**（字符串 id 被静默丢弃）；`notifications*` 方法一律被忽略（不可依赖通知送达）；`tools/list` 单响应上限 8000 字节并以 `nextCursor=<tool名>` 分页。
  - `/robot/vision/explain` 收到的是 **chunked multipart，无 `Content-Length`**，boundary 以请求头为准（不得硬编码），且必须在 **30 秒**内返回 200。
- **`ConversationService::cancel_with_origin` 是 crate 私有的**（`CancelOrigin` 枚举未导出）。外部只能调用公开包装 `cancel(user_id, conversation_id, runtime_registry)`。
- **设备身份继承安装所有者**：所有模型调用与会话操作的 `user_id` 取 `GatewayDeps.authoritative_user_id`，设备不登录。
- **绝不**经 `/api/tts` 与 `/api/stt` HTTP 端点（它们在会话鉴权层内，且 `/api/stt` 会被 `tools.speechToText.enabled` 开关连坐）。一律直调 invoke 层。
- 新增依赖必须加到根 `Cargo.toml` 的 `[workspace.dependencies]`，crate 内用 `{ workspace = true }` 引用（沿用仓库既有做法）。

## 跨计划依赖

本计划消费 **Plan B Task 1** 落地的伙伴档案字段（`fallback_model`、`vision_model`、`voice.{asr,tts,vad}`）。执行顺序：Plan B Task 1 先落 main，本计划 Task 15/16 再开始。若 B1 尚未落地，Task 15/16 的实现走「字段缺省回落」路径（全局偏好 + 主对话模型），并在 B1 落地后补一次消费提交。

工具名契约来自 **Plan C**：`self.gimbal.look(direction)`、`self.gimbal.set(pan, tilt)`、`self.gimbal.get_position`。本计划的 MCP 桥按名字前缀通用转换，不硬编码这三个名字（固件未升级时它们只是不出现在 `tools/list` 里）。

## 文件结构

```
crates/backend/nomifun-robot/
  Cargo.toml
  assets/silero_vad.onnx        # Silero VAD 权重（约 2 MB，include_bytes! 内嵌）
  src/
    lib.rs                      # 模块树 + 再导出 + RobotGateway
    registry.rs                 # RobotRegistry / RobotRecord / ClaimError（robots.json 原子写）
    dto.rs                      # RobotDto / RobotStatusDto / OTA 请求与响应线上形状
    events.rs                   # RobotEventEmitter（robot.status）
    status.rs                   # RobotPhase / RobotStatusRegistry
    link.rs                     # Frame / RobotIdentity / RobotLink / RobotLinkSource / LinkError
    endpoint.rs                 # EndpointAdvertiser / LanEndpointSnapshot / LanAdvertiser
    protocol/mod.rs             # 协议模块入口
    protocol/messages.rs        # DeviceMessage / ServerMessage 及解析
    protocol/binary.rs          # v1 二进制帧编解码
    audio/mod.rs                # AudioBuffer
    audio/opus.rs               # OpusStreamDecoder / OpusStreamEncoder
    audio/wav.rs                # pcm_to_wav
    audio/resample.rs           # resample_linear
    audio/container.rs          # decode_container（symphonia）
    vad/mod.rs                  # VadEngine trait / VadDecision / VadTuning
    vad/energy.rs               # EnergyVad
    vad/silero.rs               # SileroVad（ort）
    pipeline/mod.rs
    pipeline/sentence.rs        # SentenceSplitter / strip_emotion / normalize_emotion
    pipeline/uplink.rs          # UplinkPipeline
    pipeline/downlink.rs        # DownlinkPipeline（pacing + 冲刷）
    session.rs                  # RobotSession actor
    mcp_bridge.rs               # RobotMcpTransport + 工具发现
    services.rs                 # trait seam：SpeechServices / CompanionTurnDispatcher / TurnEvent
    wiring/mod.rs
    wiring/speech.rs            # 真实 SpeechServices（invoke 层 + nomi-providers 视觉）
    wiring/dispatcher.rs        # 真实 CompanionTurnDispatcher
    lan_source.rs               # LanWsSource（axum WS → RobotLink）
    routes/mod.rs
    routes/device.rs            # /robot/ota、/robot/ota/activate、/robot/v1、/robot/vision/explain
    routes/admin.rs             # /api/robots*
  tests/
    fake_device.rs              # 模拟设备集成测试
```

---

### Task 1: crate 脚手架与空挂载

**Files:**
- Create: `crates/backend/nomifun-robot/Cargo.toml`
- Create: `crates/backend/nomifun-robot/src/lib.rs`
- Modify: `Cargo.toml`（根，`[workspace.dependencies]` 内部 crate 区加一行）
- Test: `crates/backend/nomifun-robot/src/lib.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Produces: `pub fn robot_domain_name() -> &'static str`（占位，确认 crate 可编译可测试）

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/lib.rs`：

```rust
//! Robot gateway: LAN-attached physical robots (xiaozhi firmware) acting as the
//! physical embodiment of a desktop companion.
//!
//! Byte sources are abstracted behind [`link::RobotLinkSource`] so a future
//! public relay reuses the same session core; model capabilities sit behind the
//! [`services`] trait seam so the pipeline is testable with mocks.

/// Domain name used in log fields and event prefixes.
pub fn robot_domain_name() -> &'static str {
    "robot"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_name_is_robot() {
        assert_eq!(robot_domain_name(), "robot");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot`
Expected: FAIL — `error: package ID specification 'nomifun-robot' did not match any packages`（Cargo.toml 尚未创建）

- [ ] **Step 3: 写最小实现**

创建 `crates/backend/nomifun-robot/Cargo.toml`：

```toml
[package]
name = "nomifun-robot"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
```

根 `Cargo.toml` 的 `[workspace.dependencies]` 内部 backend crate 区（`nomifun-ssh` 那一行附近）加：

```toml
nomifun-robot = { path = "crates/backend/nomifun-robot" }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot`
Expected: PASS — `test tests::domain_name_is_robot ... ok`

若报某个 workspace 依赖未定义（如 `thiserror` 不在 workspace deps），先在根 `Cargo.toml` 查证该键是否存在；不存在则从该 crate 的 `[dependencies]` 里删掉它（本任务只需能编译）。

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/backend/nomifun-robot/
git commit -m "feat(robot): scaffold nomifun-robot crate"
```

---

### Task 2: 设备注册表（robots.json）

**Files:**
- Create: `crates/backend/nomifun-robot/src/registry.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod registry;`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `sha2`、`rand`、`chrono`）

**Interfaces:**
- Produces:
  - `pub struct RobotRecord { pub robot_id: String, pub client_id: String, pub name: String, pub companion_id: Option<String>, pub token_hash: String, pub activation_code: Option<String>, pub board: String, pub firmware_version: String, pub last_seen: Option<i64>, pub created_at: i64 }`
  - `pub struct RobotReport { pub robot_id: String, pub client_id: String, pub board: String, pub firmware_version: String }`
  - `pub struct RobotRegistry`，方法：
    - `pub async fn load(data_dir: &std::path::Path) -> anyhow::Result<Self>`
    - `pub async fn upsert_on_report(&self, report: RobotReport, now_ms: i64) -> anyhow::Result<(RobotRecord, String)>` — 返回 `(记录, 明文 token)`，每次报到重新铸 token
    - `pub async fn resolve_token(&self, token: &str) -> Option<RobotRecord>`
    - `pub async fn claim(&self, code: &str, companion_id: &str) -> Result<RobotRecord, ClaimError>`
    - `pub async fn patch(&self, robot_id: &str, name: Option<String>, companion_id: Option<Option<String>>) -> Result<RobotRecord, ClaimError>`
    - `pub async fn remove(&self, robot_id: &str) -> anyhow::Result<bool>`
    - `pub async fn list(&self) -> Vec<RobotRecord>`
  - `pub enum ClaimError { NotFound, AlreadyBound { companion_id: String } }`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/registry.rs`，先只写测试模块（实现留空文件顶部的 `use`，测试会编译失败）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: &str) -> RobotReport {
        RobotReport {
            robot_id: id.to_owned(),
            client_id: "3f2b9c1e-0000-4000-8000-000000000001".to_owned(),
            board: "esp32-s3n16r8-emoji".to_owned(),
            firmware_version: "1.9.0".to_owned(),
        }
    }

    #[tokio::test]
    async fn first_report_mints_token_and_activation_code() {
        let dir = tempfile::tempdir().unwrap();
        let reg = RobotRegistry::load(dir.path()).await.unwrap();

        let (record, token) = reg
            .upsert_on_report(report("aa:bb:cc:dd:ee:ff"), 1_700_000_000_000)
            .await
            .unwrap();

        assert_eq!(record.robot_id, "aa:bb:cc:dd:ee:ff");
        assert!(record.companion_id.is_none());
        assert_eq!(record.activation_code.as_deref().map(str::len), Some(6));
        assert!(record.activation_code.as_deref().unwrap().chars().all(|c| c.is_ascii_digit()));
        assert_eq!(token.len(), 64, "token is 256-bit hex");
        assert_ne!(record.token_hash, token, "only the hash is persisted");
        assert_eq!(reg.resolve_token(&token).await.unwrap().robot_id, record.robot_id);
    }

    #[tokio::test]
    async fn re_report_rotates_token_and_keeps_activation_code() {
        let dir = tempfile::tempdir().unwrap();
        let reg = RobotRegistry::load(dir.path()).await.unwrap();
        let (first, token_a) = reg.upsert_on_report(report("aa:bb:cc:dd:ee:01"), 1).await.unwrap();
        let (second, token_b) = reg.upsert_on_report(report("aa:bb:cc:dd:ee:01"), 2).await.unwrap();

        assert_ne!(token_a, token_b, "each report mints a fresh token");
        assert_eq!(first.activation_code, second.activation_code, "code is stable while unbound");
        assert_eq!(first.created_at, second.created_at);
        assert_eq!(second.last_seen, Some(2));
        assert!(reg.resolve_token(&token_a).await.is_none(), "old token is invalidated");
        assert!(reg.resolve_token(&token_b).await.is_some());
    }

    #[tokio::test]
    async fn claim_binds_companion_and_clears_code() {
        let dir = tempfile::tempdir().unwrap();
        let reg = RobotRegistry::load(dir.path()).await.unwrap();
        let (record, _) = reg.upsert_on_report(report("aa:bb:cc:dd:ee:02"), 1).await.unwrap();
        let code = record.activation_code.clone().unwrap();

        let bound = reg.claim(&code, "0190f5fe-7c00-7a00-8000-0000000000aa").await.unwrap();
        assert_eq!(bound.companion_id.as_deref(), Some("0190f5fe-7c00-7a00-8000-0000000000aa"));
        assert!(bound.activation_code.is_none());

        assert!(matches!(reg.claim(&code, "0190f5fe-7c00-7a00-8000-0000000000bb").await, Err(ClaimError::NotFound)));
    }

    #[tokio::test]
    async fn state_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (code, token) = {
            let reg = RobotRegistry::load(dir.path()).await.unwrap();
            let (record, token) = reg.upsert_on_report(report("aa:bb:cc:dd:ee:03"), 1).await.unwrap();
            (record.activation_code.unwrap(), token)
        };
        let reg = RobotRegistry::load(dir.path()).await.unwrap();
        assert_eq!(reg.list().await.len(), 1);
        assert!(reg.resolve_token(&token).await.is_some());
        assert!(reg.claim(&code, "0190f5fe-7c00-7a00-8000-0000000000cc").await.is_ok());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot registry`
Expected: FAIL — `cannot find type RobotRegistry in this scope`（以及 `RobotReport`、`ClaimError` 同样未定义）

- [ ] **Step 3: 写最小实现**

`crates/backend/nomifun-robot/Cargo.toml` 的 `[dependencies]` 追加：

```toml
sha2 = { workspace = true }
rand = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

（先 `grep -n '^sha2\|^rand\|^tempfile' Cargo.toml` 确认这三个键在 workspace deps 里存在；缺哪个就用 `cargo add --package nomifun-robot <name>` 补，并把解析出的版本回填到根 `Cargo.toml` 的 `[workspace.dependencies]`。）

`registry.rs` 顶部写入（放在 `#[cfg(test)] mod tests` 之前）：

```rust
//! Robot registry: `{data_dir}/robot/robots.json`, atomic temp+rename writes.
//!
//! Tokens are persisted as SHA-256 only. A fresh token is minted on **every**
//! OTA report because the firmware re-reads `websocket.token` from each response
//! and persists it to NVS — so rotation-per-boot is transparent, and an already
//! authenticated WebSocket keeps working until it drops.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

/// Subdirectory of the backend data dir holding robot state.
pub const ROBOT_REL_DIR: &str = "robot";
/// Registry file name inside [`ROBOT_REL_DIR`].
pub const ROBOTS_FILE: &str = "robots.json";

/// One registered robot. `token_hash` is the SHA-256 of the last minted token;
/// the plaintext exists only in the OTA response that minted it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotRecord {
    pub robot_id: String,
    pub client_id: String,
    pub name: String,
    pub companion_id: Option<String>,
    pub token_hash: String,
    pub activation_code: Option<String>,
    pub board: String,
    pub firmware_version: String,
    pub last_seen: Option<i64>,
    pub created_at: i64,
}

/// The subset of a firmware device report the registry cares about.
#[derive(Debug, Clone)]
pub struct RobotReport {
    pub robot_id: String,
    pub client_id: String,
    pub board: String,
    pub firmware_version: String,
}

/// Why a claim / patch could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    #[error("no robot matches that activation code")]
    NotFound,
    #[error("robot is already bound to companion {companion_id}")]
    AlreadyBound { companion_id: String },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    robots: Vec<RobotRecord>,
}

/// Owns the on-disk registry and the in-memory token index.
pub struct RobotRegistry {
    path: PathBuf,
    inner: RwLock<BTreeMap<String, RobotRecord>>,
}

/// SHA-256 of `token`, lowercase hex (64 chars). Mirrors
/// `nomifun_auth::token_sha256_hex` — duplicated to keep this crate's
/// dependency surface minimal.
fn token_sha256_hex(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn mint_activation_code() -> String {
    let mut bytes = [0u8; 4];
    rand::rng().fill_bytes(&mut bytes);
    let n = u32::from_be_bytes(bytes) % 1_000_000;
    format!("{n:06}")
}

fn default_name(board: &str) -> String {
    match board {
        "esp32-s3n16r8-emoji" => "表情机器人".to_owned(),
        other => other.to_owned(),
    }
}

impl RobotRegistry {
    /// Load (or create) the registry under `data_dir/robot/robots.json`.
    pub async fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let dir = data_dir.join(ROBOT_REL_DIR);
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(ROBOTS_FILE);
        let robots = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<RegistryFile>(&bytes)
                .unwrap_or_default()
                .robots,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let map = robots
            .into_iter()
            .map(|r| (r.robot_id.clone(), r))
            .collect::<BTreeMap<_, _>>();
        Ok(Self { path, inner: RwLock::new(map) })
    }

    async fn persist(&self, map: &BTreeMap<String, RobotRecord>) -> anyhow::Result<()> {
        let file = RegistryFile { robots: map.values().cloned().collect() };
        let bytes = serde_json::to_vec_pretty(&file)?;
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }

    /// Upsert on device report. Always mints a fresh token and returns its
    /// plaintext for the OTA response.
    pub async fn upsert_on_report(
        &self,
        report: RobotReport,
        now_ms: i64,
    ) -> anyhow::Result<(RobotRecord, String)> {
        let token = mint_token();
        let token_hash = token_sha256_hex(&token);
        let mut map = self.inner.write().await;
        let record = match map.get_mut(&report.robot_id) {
            Some(existing) => {
                existing.client_id = report.client_id;
                existing.board = report.board;
                existing.firmware_version = report.firmware_version;
                existing.token_hash = token_hash;
                existing.last_seen = Some(now_ms);
                if existing.companion_id.is_none() && existing.activation_code.is_none() {
                    existing.activation_code = Some(mint_activation_code());
                }
                existing.clone()
            }
            None => {
                let record = RobotRecord {
                    name: default_name(&report.board),
                    robot_id: report.robot_id.clone(),
                    client_id: report.client_id,
                    companion_id: None,
                    token_hash,
                    activation_code: Some(mint_activation_code()),
                    board: report.board,
                    firmware_version: report.firmware_version,
                    last_seen: Some(now_ms),
                    created_at: now_ms,
                };
                map.insert(record.robot_id.clone(), record.clone());
                record
            }
        };
        self.persist(&map).await?;
        Ok((record, token))
    }

    /// Resolve a presented bearer token to its robot. Constant-time per entry.
    pub async fn resolve_token(&self, token: &str) -> Option<RobotRecord> {
        if token.is_empty() {
            return None;
        }
        let presented = token_sha256_hex(token);
        let map = self.inner.read().await;
        map.values().find(|r| ct_eq(&presented, &r.token_hash)).cloned()
    }

    /// Bind the robot holding `code` to `companion_id`, clearing the code.
    pub async fn claim(&self, code: &str, companion_id: &str) -> Result<RobotRecord, ClaimError> {
        let mut map = self.inner.write().await;
        let record = map
            .values_mut()
            .find(|r| r.activation_code.as_deref() == Some(code))
            .ok_or(ClaimError::NotFound)?;
        if let Some(bound) = &record.companion_id {
            return Err(ClaimError::AlreadyBound { companion_id: bound.clone() });
        }
        record.companion_id = Some(companion_id.to_owned());
        record.activation_code = None;
        let out = record.clone();
        let _ = self.persist(&map).await;
        Ok(out)
    }

    /// Rename and/or rebind. `companion_id = Some(None)` unbinds (and re-issues
    /// an activation code so the robot can be claimed again).
    pub async fn patch(
        &self,
        robot_id: &str,
        name: Option<String>,
        companion_id: Option<Option<String>>,
    ) -> Result<RobotRecord, ClaimError> {
        let mut map = self.inner.write().await;
        let record = map.get_mut(robot_id).ok_or(ClaimError::NotFound)?;
        if let Some(name) = name {
            record.name = name;
        }
        if let Some(binding) = companion_id {
            match binding {
                Some(id) => {
                    record.companion_id = Some(id);
                    record.activation_code = None;
                }
                None => {
                    record.companion_id = None;
                    record.activation_code = Some(mint_activation_code());
                }
            }
        }
        let out = record.clone();
        let _ = self.persist(&map).await;
        Ok(out)
    }

    /// Remove a robot (revokes its token). Returns whether it existed.
    pub async fn remove(&self, robot_id: &str) -> anyhow::Result<bool> {
        let mut map = self.inner.write().await;
        let existed = map.remove(robot_id).is_some();
        if existed {
            self.persist(&map).await?;
        }
        Ok(existed)
    }

    /// All records, ordered by `robot_id`.
    pub async fn list(&self) -> Vec<RobotRecord> {
        self.inner.read().await.values().cloned().collect()
    }
}
```

`lib.rs` 加 `pub mod registry;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot registry`
Expected: PASS — 4 个测试全过

若 `rand::rng()` 报错（rand 0.8 用 `thread_rng()`），按仓库解析到的 rand 版本改：0.8 用 `rand::thread_rng().fill_bytes(...)`，0.9 用 `rand::rng()`。以 `cargo tree -p nomifun-robot -i rand` 查证实际版本为准。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add robot registry with token rotation and activation codes"
```

---

### Task 3: 协议消息词汇

**Files:**
- Create: `crates/backend/nomifun-robot/src/protocol/mod.rs`
- Create: `crates/backend/nomifun-robot/src/protocol/messages.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod protocol;`）

**Interfaces:**
- Produces:
  - `pub enum DeviceMessage { Hello(DeviceHello), Listen { state: ListenState, mode: Option<ListeningMode>, text: Option<String> }, Abort { reason: Option<String> }, Mcp { payload: serde_json::Value }, Goodbye, Unknown { raw_type: String } }`
  - `pub enum ListenState { Start, Stop, Detect }`，`pub enum ListeningMode { Auto, Manual, Realtime }`
  - `pub struct DeviceHello { pub version: u32, pub transport: String, pub mcp: bool, pub aec: bool }`
  - `pub fn parse_device_message(raw: &str) -> Result<DeviceMessage, ProtocolError>`
  - `pub enum ServerMessage { Hello { session_id: String }, Stt { session_id: String, text: String }, Llm { session_id: String, emotion: String }, TtsStart { session_id: String }, TtsStop { session_id: String }, TtsSentence { session_id: String, text: String }, Mcp { session_id: String, payload: serde_json::Value }, Ping { session_id: String } }`
  - `pub fn serialize_server_message(msg: &ServerMessage) -> String`
  - `pub const DOWNLINK_SAMPLE_RATE: u32 = 24_000;` `pub const UPLINK_SAMPLE_RATE: u32 = 16_000;` `pub const FRAME_DURATION_MS: u32 = 60;`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/protocol/messages.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_hello() {
        let raw = r#"{"type":"hello","version":1,"features":{"mcp":true},"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}"#;
        let DeviceMessage::Hello(hello) = parse_device_message(raw).unwrap() else {
            panic!("expected hello");
        };
        assert_eq!(hello.version, 1);
        assert_eq!(hello.transport, "websocket");
        assert!(hello.mcp);
        assert!(!hello.aec);
    }

    #[test]
    fn parses_listen_variants() {
        let start = parse_device_message(r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#).unwrap();
        assert!(matches!(
            start,
            DeviceMessage::Listen { state: ListenState::Start, mode: Some(ListeningMode::Auto), .. }
        ));
        let stop = parse_device_message(r#"{"session_id":"s","type":"listen","state":"stop"}"#).unwrap();
        assert!(matches!(stop, DeviceMessage::Listen { state: ListenState::Stop, mode: None, .. }));
        let detect = parse_device_message(r#"{"session_id":"s","type":"listen","state":"detect","text":"你好小智"}"#).unwrap();
        let DeviceMessage::Listen { state: ListenState::Detect, text, .. } = detect else {
            panic!("expected detect");
        };
        assert_eq!(text.as_deref(), Some("你好小智"));
    }

    #[test]
    fn parses_abort_with_and_without_reason() {
        let with = parse_device_message(r#"{"session_id":"s","type":"abort","reason":"wake_word_detected"}"#).unwrap();
        assert!(matches!(with, DeviceMessage::Abort { reason: Some(ref r) } if r == "wake_word_detected"));
        let without = parse_device_message(r#"{"session_id":"s","type":"abort"}"#).unwrap();
        assert!(matches!(without, DeviceMessage::Abort { reason: None }));
    }

    #[test]
    fn unknown_type_is_tolerated_not_an_error() {
        let msg = parse_device_message(r#"{"type":"something_new","x":1}"#).unwrap();
        assert!(matches!(msg, DeviceMessage::Unknown { ref raw_type } if raw_type == "something_new"));
    }

    #[test]
    fn missing_type_is_an_error() {
        assert!(parse_device_message(r#"{"state":"start"}"#).is_err());
    }

    #[test]
    fn server_hello_declares_downlink_audio_params() {
        let json = serialize_server_message(&ServerMessage::Hello { session_id: "abc".into() });
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "hello");
        assert_eq!(value["transport"], "websocket");
        assert_eq!(value["session_id"], "abc");
        assert_eq!(value["audio_params"]["format"], "opus");
        assert_eq!(value["audio_params"]["sample_rate"], 24000);
        assert_eq!(value["audio_params"]["channels"], 1);
        assert_eq!(value["audio_params"]["frame_duration"], 60);
    }

    #[test]
    fn tts_messages_carry_state_and_session() {
        let start: serde_json::Value =
            serde_json::from_str(&serialize_server_message(&ServerMessage::TtsStart { session_id: "s".into() })).unwrap();
        assert_eq!(start["type"], "tts");
        assert_eq!(start["state"], "start");
        assert_eq!(start["session_id"], "s");

        let sentence: serde_json::Value = serde_json::from_str(&serialize_server_message(
            &ServerMessage::TtsSentence { session_id: "s".into(), text: "你好".into() },
        ))
        .unwrap();
        assert_eq!(sentence["state"], "sentence_start");
        assert_eq!(sentence["text"], "你好");

        let stop: serde_json::Value =
            serde_json::from_str(&serialize_server_message(&ServerMessage::TtsStop { session_id: "s".into() })).unwrap();
        assert_eq!(stop["state"], "stop");
    }

    #[test]
    fn llm_message_carries_emotion_only() {
        let value: serde_json::Value = serde_json::from_str(&serialize_server_message(
            &ServerMessage::Llm { session_id: "s".into(), emotion: "happy".into() },
        ))
        .unwrap();
        assert_eq!(value["type"], "llm");
        assert_eq!(value["emotion"], "happy");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot protocol`
Expected: FAIL — `cannot find function parse_device_message in this scope`

- [ ] **Step 3: 写最小实现**

`messages.rs` 顶部写入：

```rust
//! The xiaozhi JSON message vocabulary.
//!
//! Inbound parsing is deliberately tolerant: the firmware may add message types
//! at any time, and an unknown `type` must not kill the session — it becomes
//! [`DeviceMessage::Unknown`]. Outbound messages always carry `session_id`
//! first, matching the firmware's own hand-built strings.

use serde::Deserialize;

/// Uplink audio is hardcoded in firmware and cannot be negotiated.
pub const UPLINK_SAMPLE_RATE: u32 = 16_000;
/// Downlink rate we declare in the server hello.
pub const DOWNLINK_SAMPLE_RATE: u32 = 24_000;
/// Opus frame duration used in both directions, in milliseconds.
pub const FRAME_DURATION_MS: u32 = 60;

/// Failure to understand an inbound text frame.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("message has no string `type` field")]
    MissingType,
    #[error("`listen` message has no valid `state`")]
    MissingListenState,
}

/// `listen.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenState {
    Start,
    Stop,
    Detect,
}

/// `listen.mode` — how the turn ends. `Auto`/`Realtime` mean the **server**
/// must decide when the user stopped talking; `Manual` means the device will
/// send `listen stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListeningMode {
    Auto,
    Manual,
    Realtime,
}

/// Device hello payload (the parts we act on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceHello {
    pub version: u32,
    pub transport: String,
    pub mcp: bool,
    pub aec: bool,
}

/// A parsed inbound text frame.
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceMessage {
    Hello(DeviceHello),
    Listen {
        state: ListenState,
        mode: Option<ListeningMode>,
        text: Option<String>,
    },
    Abort {
        reason: Option<String>,
    },
    Mcp {
        payload: serde_json::Value,
    },
    Goodbye,
    Unknown {
        raw_type: String,
    },
}

#[derive(Deserialize)]
struct RawHello {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    features: RawFeatures,
}

fn default_version() -> u32 {
    1
}

#[derive(Deserialize, Default)]
struct RawFeatures {
    #[serde(default)]
    mcp: bool,
    #[serde(default)]
    aec: bool,
}

/// Parse an inbound text frame. Unknown `type` values are surfaced as
/// [`DeviceMessage::Unknown`] rather than an error.
pub fn parse_device_message(raw: &str) -> Result<DeviceMessage, ProtocolError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let msg_type = value.get("type").and_then(|t| t.as_str()).ok_or(ProtocolError::MissingType)?;
    match msg_type {
        "hello" => {
            let raw_hello: RawHello = serde_json::from_value(value)?;
            Ok(DeviceMessage::Hello(DeviceHello {
                version: raw_hello.version,
                transport: raw_hello.transport,
                mcp: raw_hello.features.mcp,
                aec: raw_hello.features.aec,
            }))
        }
        "listen" => {
            let state = match value.get("state").and_then(|s| s.as_str()) {
                Some("start") => ListenState::Start,
                Some("stop") => ListenState::Stop,
                Some("detect") => ListenState::Detect,
                _ => return Err(ProtocolError::MissingListenState),
            };
            let mode = match value.get("mode").and_then(|m| m.as_str()) {
                Some("auto") => Some(ListeningMode::Auto),
                Some("manual") => Some(ListeningMode::Manual),
                Some("realtime") => Some(ListeningMode::Realtime),
                _ => None,
            };
            let text = value.get("text").and_then(|t| t.as_str()).map(str::to_owned);
            Ok(DeviceMessage::Listen { state, mode, text })
        }
        "abort" => Ok(DeviceMessage::Abort {
            reason: value.get("reason").and_then(|r| r.as_str()).map(str::to_owned),
        }),
        "mcp" => Ok(DeviceMessage::Mcp {
            payload: value.get("payload").cloned().unwrap_or(serde_json::Value::Null),
        }),
        "goodbye" => Ok(DeviceMessage::Goodbye),
        other => Ok(DeviceMessage::Unknown { raw_type: other.to_owned() }),
    }
}

/// An outbound text frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    Hello { session_id: String },
    Stt { session_id: String, text: String },
    Llm { session_id: String, emotion: String },
    TtsStart { session_id: String },
    TtsStop { session_id: String },
    TtsSentence { session_id: String, text: String },
    Mcp { session_id: String, payload: serde_json::Value },
    Ping { session_id: String },
}

/// Render an outbound frame. Never fails: every variant is plain JSON.
pub fn serialize_server_message(msg: &ServerMessage) -> String {
    use serde_json::json;
    let value = match msg {
        ServerMessage::Hello { session_id } => json!({
            "type": "hello",
            "transport": "websocket",
            "session_id": session_id,
            "audio_params": {
                "format": "opus",
                "sample_rate": DOWNLINK_SAMPLE_RATE,
                "channels": 1,
                "frame_duration": FRAME_DURATION_MS,
            },
        }),
        ServerMessage::Stt { session_id, text } => {
            json!({ "session_id": session_id, "type": "stt", "text": text })
        }
        ServerMessage::Llm { session_id, emotion } => {
            json!({ "session_id": session_id, "type": "llm", "emotion": emotion })
        }
        ServerMessage::TtsStart { session_id } => {
            json!({ "session_id": session_id, "type": "tts", "state": "start" })
        }
        ServerMessage::TtsStop { session_id } => {
            json!({ "session_id": session_id, "type": "tts", "state": "stop" })
        }
        ServerMessage::TtsSentence { session_id, text } => {
            json!({ "session_id": session_id, "type": "tts", "state": "sentence_start", "text": text })
        }
        ServerMessage::Mcp { session_id, payload } => {
            json!({ "session_id": session_id, "type": "mcp", "payload": payload })
        }
        ServerMessage::Ping { session_id } => {
            json!({ "session_id": session_id, "type": "ping" })
        }
    };
    value.to_string()
}
```

创建 `crates/backend/nomifun-robot/src/protocol/mod.rs`：

```rust
//! xiaozhi wire protocol: JSON message vocabulary and binary frame codec.

pub mod messages;

pub use messages::{
    DeviceHello, DeviceMessage, ListenState, ListeningMode, ProtocolError, ServerMessage,
    parse_device_message, serialize_server_message, DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS,
    UPLINK_SAMPLE_RATE,
};
```

`lib.rs` 加 `pub mod protocol;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot protocol`
Expected: PASS — 8 个测试全过

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/
git commit -m "feat(robot): add xiaozhi JSON message vocabulary"
```

---

### Task 4: v1 二进制帧与帧层抽象

**Files:**
- Create: `crates/backend/nomifun-robot/src/protocol/binary.rs`
- Create: `crates/backend/nomifun-robot/src/link.rs`
- Modify: `crates/backend/nomifun-robot/src/protocol/mod.rs`（加 `pub mod binary;`）
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod link;`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `bytes`、`async-trait`）

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub fn encode_binary_v1(opus_packet: &[u8]) -> bytes::Bytes`
  - `pub fn decode_binary_v1(frame: &[u8]) -> &[u8]`
  - `pub enum Frame { Text(String), Binary(bytes::Bytes) }`
  - `pub struct RobotIdentity { pub robot_id: String, pub client_id: String, pub peer: String }`
  - `pub trait RobotLinkSink: Send { async fn send(&mut self, frame: Frame) -> Result<(), LinkError>; async fn close(&mut self); }`
  - `pub trait RobotLinkStream: Send { async fn next(&mut self) -> Option<Result<Frame, LinkError>>; }`
  - `pub struct AcceptedLink { pub identity: RobotIdentity, pub sink: Box<dyn RobotLinkSink>, pub stream: Box<dyn RobotLinkStream> }`
  - `pub trait RobotLinkSource: Send + Sync { fn name(&self) -> &'static str; async fn run(self: std::sync::Arc<Self>, accept: tokio::sync::mpsc::Sender<AcceptedLink>) -> anyhow::Result<()>; }`
  - `pub enum LinkError { Closed, Transport(String) }`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/protocol/binary.rs`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_adds_no_header_bytes() {
        let packet = [0xfc_u8, 0x01, 0x02, 0x03];
        let framed = encode_binary_v1(&packet);
        assert_eq!(
            framed.as_ref(),
            &packet,
            "protocol v1 is a bare Opus payload — any header breaks the firmware decoder"
        );
    }

    #[test]
    fn v1_decode_is_identity() {
        let frame = [0x11_u8, 0x22, 0x33];
        assert_eq!(decode_binary_v1(&frame), &frame);
    }

    #[test]
    fn empty_frame_round_trips() {
        assert!(encode_binary_v1(&[]).is_empty());
        assert!(decode_binary_v1(&[]).is_empty());
    }
}
```

创建 `crates/backend/nomifun-robot/src/link.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct CountingSource;

    #[async_trait::async_trait]
    impl RobotLinkSource for CountingSource {
        fn name(&self) -> &'static str {
            "counting"
        }

        async fn run(
            self: Arc<Self>,
            accept: tokio::sync::mpsc::Sender<AcceptedLink>,
        ) -> anyhow::Result<()> {
            let (sink, stream) = fake_pair();
            accept
                .send(AcceptedLink {
                    identity: RobotIdentity {
                        robot_id: "aa:bb:cc:dd:ee:ff".into(),
                        client_id: "cid".into(),
                        peer: "192.168.1.9".into(),
                    },
                    sink: Box::new(sink),
                    stream: Box::new(stream),
                })
                .await
                .map_err(|_| anyhow::anyhow!("receiver dropped"))?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_source_hands_accepted_links_to_the_gateway_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let source = Arc::new(CountingSource);
        assert_eq!(source.name(), "counting");
        source.clone().run(tx).await.unwrap();

        let link = rx.recv().await.expect("one link accepted");
        assert_eq!(link.identity.robot_id, "aa:bb:cc:dd:ee:ff");
        assert_eq!(link.identity.peer, "192.168.1.9");
    }

    #[tokio::test]
    async fn fake_pair_moves_frames_both_ways() {
        let (mut sink, mut stream) = fake_pair();
        sink.send(Frame::Text("hi".into())).await.unwrap();
        // The test double loops sink writes back into its own stream.
        let got = stream.next().await.unwrap().unwrap();
        assert_eq!(got, Frame::Text("hi".into()));

        sink.close().await;
        assert!(stream.next().await.is_none(), "closing ends the stream");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot -- binary link`
Expected: FAIL — `cannot find function encode_binary_v1`、`cannot find trait RobotLinkSource`、`cannot find function fake_pair`

- [ ] **Step 3: 写最小实现**

`Cargo.toml` 的 `[dependencies]` 追加（先确认 workspace 里有这两个键）：

```toml
async-trait = { workspace = true }
bytes = { workspace = true }
```

`binary.rs` 顶部写入：

```rust
//! Binary audio frame codec.
//!
//! The firmware picks the framing from the OTA-delivered `websocket.version`.
//! We always advertise **version 1**, which is a bare Opus payload with no
//! header at all — v2/v3 add headers this gateway deliberately does not
//! implement (see spec §1 non-goals). The identity functions exist so the call
//! sites read as framing decisions and a future v2 has one obvious home.

use bytes::Bytes;

/// Wrap an Opus packet for the wire. v1 = the packet itself.
pub fn encode_binary_v1(opus_packet: &[u8]) -> Bytes {
    Bytes::copy_from_slice(opus_packet)
}

/// Extract the Opus packet from a wire frame. v1 = the frame itself.
pub fn decode_binary_v1(frame: &[u8]) -> &[u8] {
    frame
}
```

`link.rs` 顶部写入：

```rust
//! Transport-agnostic robot links.
//!
//! The session core consumes [`Frame`]s and never learns whether they arrived
//! over a LAN WebSocket or (future) a multiplexed relay tunnel. A
//! [`RobotLinkSource`] owns its own accept loop and hands authenticated
//! [`AcceptedLink`]s to the gateway over a channel, so adding the relay is a
//! new source implementation and zero changes here.

use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::mpsc;

/// One wire frame in either direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Bytes),
}

/// Who is on the other end, resolved during authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotIdentity {
    /// Device-Id header (MAC address).
    pub robot_id: String,
    /// Client-Id header (firmware NVS UUID).
    pub client_id: String,
    /// Human-readable peer description for logs (IP, or relay tunnel id).
    pub peer: String,
}

/// Why a link operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("link closed")]
    Closed,
    #[error("transport error: {0}")]
    Transport(String),
}

/// Write half of a link.
#[async_trait::async_trait]
pub trait RobotLinkSink: Send {
    async fn send(&mut self, frame: Frame) -> Result<(), LinkError>;
    async fn close(&mut self);
}

/// Read half of a link.
#[async_trait::async_trait]
pub trait RobotLinkStream: Send {
    async fn next(&mut self) -> Option<Result<Frame, LinkError>>;
}

/// An authenticated link ready for a session actor.
pub struct AcceptedLink {
    pub identity: RobotIdentity,
    pub sink: Box<dyn RobotLinkSink>,
    pub stream: Box<dyn RobotLinkStream>,
}

/// A producer of authenticated links.
///
/// LAN is push-driven (axum hands us an upgraded socket), so `LanWsSource::run`
/// drains an internal queue; a relay source's `run` dials outbound and
/// demultiplexes. Both shapes fit this one interface.
#[async_trait::async_trait]
pub trait RobotLinkSource: Send + Sync {
    /// Stable name for logs.
    fn name(&self) -> &'static str;
    /// Run until shutdown, sending every accepted link to `accept`.
    async fn run(self: Arc<Self>, accept: mpsc::Sender<AcceptedLink>) -> anyhow::Result<()>;
}

/// An in-memory link pair for tests: writes to the sink surface on the stream.
#[cfg(test)]
pub(crate) fn fake_pair() -> (FakeSink, FakeStream) {
    let (tx, rx) = mpsc::channel(16);
    (FakeSink { tx: Some(tx) }, FakeStream { rx })
}

#[cfg(test)]
pub(crate) struct FakeSink {
    tx: Option<mpsc::Sender<Frame>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl RobotLinkSink for FakeSink {
    async fn send(&mut self, frame: Frame) -> Result<(), LinkError> {
        let tx = self.tx.as_ref().ok_or(LinkError::Closed)?;
        tx.send(frame).await.map_err(|_| LinkError::Closed)
    }

    async fn close(&mut self) {
        self.tx = None;
    }
}

#[cfg(test)]
pub(crate) struct FakeStream {
    rx: mpsc::Receiver<Frame>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl RobotLinkStream for FakeStream {
    async fn next(&mut self) -> Option<Result<Frame, LinkError>> {
        self.rx.recv().await.map(Ok)
    }
}
```

`protocol/mod.rs` 加 `pub mod binary;` 与 `pub use binary::{decode_binary_v1, encode_binary_v1};`；`lib.rs` 加 `pub mod link;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot -- binary link`
Expected: PASS — 5 个测试全过

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add v1 binary framing and transport-agnostic link traits"
```

---

### Task 5: 端点广告器（EndpointAdvertiser + LanAdvertiser）

**Files:**
- Create: `crates/backend/nomifun-robot/src/endpoint.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod endpoint;`）

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub struct LanEndpointSnapshot { pub enabled: bool, pub port: u16, pub ipv4s: Vec<std::net::Ipv4Addr> }`（`Default` 为 `enabled: false, port: 0, ipv4s: vec![]`）
  - `pub trait EndpointAdvertiser: Send + Sync { fn websocket_url(&self, peer: std::net::IpAddr) -> Option<String>; fn http_base(&self, peer: std::net::IpAddr) -> Option<String>; fn ota_urls(&self) -> Vec<String>; fn is_available(&self) -> bool; }`
  - `pub struct LanAdvertiser`，`pub fn new(status: tokio::sync::watch::Receiver<LanEndpointSnapshot>) -> Self`
  - `pub const WS_PATH: &str = "/robot/v1";` `pub const OTA_PATH: &str = "/robot/ota";` `pub const VISION_PATH: &str = "/robot/vision/explain";`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/endpoint.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn snapshot(enabled: bool, port: u16, ips: &[[u8; 4]]) -> LanEndpointSnapshot {
        LanEndpointSnapshot {
            enabled,
            port,
            ipv4s: ips.iter().map(|o| Ipv4Addr::new(o[0], o[1], o[2], o[3])).collect(),
        }
    }

    #[test]
    fn picks_the_interface_sharing_the_peer_prefix() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(
            true,
            25808,
            &[[10, 0, 0, 5], [192, 168, 1, 20]],
        ));
        let adv = LanAdvertiser::new(rx);

        let peer = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 77));
        assert_eq!(adv.websocket_url(peer).as_deref(), Some("ws://192.168.1.20:25808/robot/v1"));
        assert_eq!(adv.http_base(peer).as_deref(), Some("http://192.168.1.20:25808"));
    }

    #[test]
    fn falls_back_to_the_first_interface_when_no_prefix_matches() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(true, 25809, &[[10, 0, 0, 5]]));
        let adv = LanAdvertiser::new(rx);
        let peer = IpAddr::V4(Ipv4Addr::new(172, 20, 3, 4));
        assert_eq!(adv.websocket_url(peer).as_deref(), Some("ws://10.0.0.5:25809/robot/v1"));
    }

    #[test]
    fn unavailable_when_lan_listener_is_off() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(false, 25808, &[[192, 168, 1, 20]]));
        let adv = LanAdvertiser::new(rx);
        assert!(!adv.is_available());
        assert!(adv.websocket_url(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 77))).is_none());
        assert!(adv.ota_urls().is_empty());
    }

    #[test]
    fn unavailable_when_no_interface_was_detected() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(true, 25808, &[]));
        let adv = LanAdvertiser::new(rx);
        assert!(!adv.is_available());
    }

    #[test]
    fn ota_urls_list_every_candidate_interface() {
        let (_tx, rx) = tokio::sync::watch::channel(snapshot(
            true,
            25808,
            &[[192, 168, 1, 20], [10, 8, 0, 2]],
        ));
        let adv = LanAdvertiser::new(rx);
        assert_eq!(
            adv.ota_urls(),
            vec![
                "http://192.168.1.20:25808/robot/ota".to_owned(),
                "http://10.8.0.2:25808/robot/ota".to_owned(),
            ]
        );
    }

    #[test]
    fn reflects_live_snapshot_changes() {
        let (tx, rx) = tokio::sync::watch::channel(snapshot(false, 0, &[]));
        let adv = LanAdvertiser::new(rx);
        assert!(!adv.is_available());
        tx.send(snapshot(true, 25808, &[[192, 168, 1, 20]])).unwrap();
        assert!(adv.is_available());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot endpoint`
Expected: FAIL — `cannot find struct LanEndpointSnapshot in this scope`

- [ ] **Step 3: 写最小实现**

`endpoint.rs` 顶部写入：

```rust
//! What address to tell a device to connect to.
//!
//! The OTA response is the **only** channel that configures the firmware's
//! server address, and the vision URL rides MCP `initialize`, so both come from
//! one place. Today the only implementation is LAN; a future relay implements
//! the same trait and returns its public wss/https base, leaving the OTA handler
//! untouched.

use std::net::{IpAddr, Ipv4Addr};

use tokio::sync::watch;

/// WebSocket path devices connect to.
pub const WS_PATH: &str = "/robot/v1";
/// OTA report path (the one address a user types into the firmware).
pub const OTA_PATH: &str = "/robot/ota";
/// Vision explain path (delivered via MCP `initialize`, not OTA).
pub const VISION_PATH: &str = "/robot/vision/explain";

/// Live view of the desktop LAN listener. Fed by `nomifun-app` from its
/// `DesktopServer` status watch; this crate never depends on `nomifun-app`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanEndpointSnapshot {
    pub enabled: bool,
    pub port: u16,
    pub ipv4s: Vec<Ipv4Addr>,
}

/// Resolves the addresses a device should use.
pub trait EndpointAdvertiser: Send + Sync {
    /// `ws://…/robot/v1` for a device reporting from `peer`, or `None` when the
    /// transport is unavailable.
    fn websocket_url(&self, peer: IpAddr) -> Option<String>;
    /// Scheme+host+port for HTTP endpoints (vision), same origin family as
    /// [`websocket_url`](Self::websocket_url).
    fn http_base(&self, peer: IpAddr) -> Option<String>;
    /// Every OTA URL worth showing in the UI (multi-homed hosts get several).
    fn ota_urls(&self) -> Vec<String>;
    /// Whether a device could reach us at all right now.
    fn is_available(&self) -> bool;
}

/// LAN advertiser: picks the local interface that shares the longest prefix
/// with the reporting peer, so a multi-homed host (VPN + Wi-Fi + docker bridge)
/// hands the robot an address on the robot's own segment.
pub struct LanAdvertiser {
    status: watch::Receiver<LanEndpointSnapshot>,
}

impl LanAdvertiser {
    pub fn new(status: watch::Receiver<LanEndpointSnapshot>) -> Self {
        Self { status }
    }

    fn snapshot(&self) -> LanEndpointSnapshot {
        self.status.borrow().clone()
    }

    /// Interface with the most leading octets in common with `peer`; falls back
    /// to the first detected interface.
    fn best_host(&self, snap: &LanEndpointSnapshot, peer: IpAddr) -> Option<Ipv4Addr> {
        let peer_octets = match peer {
            IpAddr::V4(v4) => Some(v4.octets()),
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(|v4| v4.octets()),
        };
        let Some(peer_octets) = peer_octets else {
            return snap.ipv4s.first().copied();
        };
        snap.ipv4s
            .iter()
            .copied()
            .max_by_key(|candidate| {
                candidate
                    .octets()
                    .iter()
                    .zip(peer_octets.iter())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
            .or_else(|| snap.ipv4s.first().copied())
    }

    fn authority(&self, peer: IpAddr) -> Option<String> {
        let snap = self.snapshot();
        if !snap.enabled || snap.port == 0 {
            return None;
        }
        let host = self.best_host(&snap, peer)?;
        Some(format!("{host}:{}", snap.port))
    }
}

impl EndpointAdvertiser for LanAdvertiser {
    fn websocket_url(&self, peer: IpAddr) -> Option<String> {
        self.authority(peer).map(|a| format!("ws://{a}{WS_PATH}"))
    }

    fn http_base(&self, peer: IpAddr) -> Option<String> {
        self.authority(peer).map(|a| format!("http://{a}"))
    }

    fn ota_urls(&self) -> Vec<String> {
        let snap = self.snapshot();
        if !snap.enabled || snap.port == 0 {
            return Vec::new();
        }
        snap.ipv4s
            .iter()
            .map(|ip| format!("http://{ip}:{}{OTA_PATH}", snap.port))
            .collect()
    }

    fn is_available(&self) -> bool {
        let snap = self.snapshot();
        snap.enabled && snap.port != 0 && !snap.ipv4s.is_empty()
    }
}
```

`lib.rs` 加 `pub mod endpoint;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot endpoint`
Expected: PASS — 6 个测试全过

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/
git commit -m "feat(robot): add endpoint advertiser with LAN implementation"
```

---

### Task 6: OTA 与激活端点

**Files:**
- Create: `crates/backend/nomifun-robot/src/dto.rs`
- Create: `crates/backend/nomifun-robot/src/routes/mod.rs`
- Create: `crates/backend/nomifun-robot/src/routes/device.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod dto;`、`pub mod routes;`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `axum`、`chrono`；dev 加 `tower`）

**Interfaces:**
- Consumes: `RobotRegistry::{upsert_on_report, list}`、`RobotReport`、`EndpointAdvertiser::{websocket_url, is_available}`
- Produces:
  - `pub struct RobotDeviceState { pub registry: std::sync::Arc<crate::registry::RobotRegistry>, pub advertiser: std::sync::Arc<dyn crate::endpoint::EndpointAdvertiser> }`
  - `pub fn device_router(state: RobotDeviceState) -> axum::Router`（本任务只挂 `/ota` 与 `/ota/activate`，WS 与 vision 在 Task 8/17 追加）
  - `pub fn build_ota_response(record: &RobotRecord, token: &str, ws_url: Option<&str>, now_ms: i64, tz_offset_minutes: i32) -> serde_json::Value`
  - `pub struct DeviceReportBody`（反序列化固件设备报告的用到字段）

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/routes/device.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::{EndpointAdvertiser, LanAdvertiser, LanEndpointSnapshot};
    use crate::registry::{RobotRegistry, RobotReport};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;
    use tower::ServiceExt;

    const REPORT_BODY: &str = r#"{
        "version": 2,
        "mac_address": "aa:bb:cc:dd:ee:ff",
        "uuid": "3f2b9c1e-0000-4000-8000-000000000001",
        "application": { "name": "xiaozhi", "version": "1.9.0" },
        "board": { "type": "esp32-s3n16r8-emoji", "name": "ESP32-S3N16R8-EMOJI" }
    }"#;

    fn advertiser(enabled: bool) -> Arc<dyn EndpointAdvertiser> {
        let (_tx, rx) = tokio::sync::watch::channel(LanEndpointSnapshot {
            enabled,
            port: 25808,
            ipv4s: vec![Ipv4Addr::new(192, 168, 1, 20)],
        });
        // Keep the sender alive for the life of the receiver.
        std::mem::forget(_tx);
        Arc::new(LanAdvertiser::new(rx))
    }

    async fn state(enabled: bool) -> (RobotDeviceState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        (RobotDeviceState { registry, advertiser: advertiser(enabled) }, dir)
    }

    #[tokio::test]
    async fn ota_response_never_contains_mqtt_and_always_contains_websocket() {
        let (state, _dir) = state(true).await;
        let app = device_router(state);

        let response = app
            .oneshot(
                Request::post("/ota")
                    .header("Device-Id", "aa:bb:cc:dd:ee:ff")
                    .header("Client-Id", "3f2b9c1e-0000-4000-8000-000000000001")
                    .header("content-type", "application/json")
                    .body(Body::from(REPORT_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(value.get("mqtt").is_none(), "an mqtt object makes the firmware pick MQTT with no fallback");
        let ws = value.get("websocket").expect("websocket is mandatory");
        assert_eq!(ws["url"], "ws://192.168.1.20:25808/robot/v1");
        assert_eq!(ws["version"], 1);
        assert_eq!(ws["token"].as_str().unwrap().len(), 64);
        assert!(value["server_time"]["timestamp"].is_i64());
        // Unbound device gets an activation code.
        assert_eq!(value["activation"]["code"].as_str().unwrap().len(), 6);
        assert!(value["activation"]["message"].is_string());
        assert_eq!(value["activation"]["timeout_ms"], 30000);
    }

    #[tokio::test]
    async fn bound_device_gets_no_activation_section() {
        let (state, _dir) = state(true).await;
        let (record, _) = state
            .registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:ff".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        state
            .registry
            .claim(record.activation_code.as_deref().unwrap(), "0190f5fe-7c00-7a00-8000-0000000000aa")
            .await
            .unwrap();

        let app = device_router(state);
        let response = app
            .oneshot(
                Request::post("/ota")
                    .header("Device-Id", "aa:bb:cc:dd:ee:ff")
                    .header("Client-Id", "cid")
                    .header("content-type", "application/json")
                    .body(Body::from(REPORT_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("activation").is_none());
    }

    #[tokio::test]
    async fn activate_returns_202_until_bound_then_200() {
        let (state, _dir) = state(true).await;
        let registry = state.registry.clone();
        let (record, _) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:01".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();

        let pending = device_router(state.clone())
            .oneshot(
                Request::post("/ota/activate")
                    .header("Device-Id", "aa:bb:cc:dd:ee:01")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pending.status(), StatusCode::ACCEPTED, "202 = still waiting for the user");

        registry
            .claim(record.activation_code.as_deref().unwrap(), "0190f5fe-7c00-7a00-8000-0000000000aa")
            .await
            .unwrap();

        let done = device_router(state)
            .oneshot(
                Request::post("/ota/activate")
                    .header("Device-Id", "aa:bb:cc:dd:ee:01")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(done.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn ota_still_answers_when_lan_is_off_but_omits_the_url() {
        let (state, _dir) = state(false).await;
        let app = device_router(state);
        let response = app
            .oneshot(
                Request::post("/ota")
                    .header("Device-Id", "aa:bb:cc:dd:ee:02")
                    .header("Client-Id", "cid")
                    .header("content-type", "application/json")
                    .body(Body::from(REPORT_BODY))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("mqtt").is_none(), "still never mqtt");
        assert_eq!(value["websocket"]["url"], "", "empty url keeps the websocket object present");
    }

    #[test]
    fn build_ota_response_shape_is_stable() {
        let record = crate::registry::RobotRecord {
            robot_id: "aa:bb:cc:dd:ee:ff".into(),
            client_id: "cid".into(),
            name: "表情机器人".into(),
            companion_id: None,
            token_hash: "hash".into(),
            activation_code: Some("483920".into()),
            board: "esp32-s3n16r8-emoji".into(),
            firmware_version: "1.9.0".into(),
            last_seen: Some(1),
            created_at: 1,
        };
        let value = build_ota_response(&record, "tok", Some("ws://x/robot/v1"), 1_700_000_000_000, 480);
        assert_eq!(value["websocket"]["token"], "tok");
        assert_eq!(value["server_time"]["timezone_offset"], 480);
        assert_eq!(value["firmware"]["version"], "1.9.0", "echo the device's own version: no upgrade");
        assert_eq!(value["firmware"]["url"], "");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot device`
Expected: FAIL — `cannot find function device_router in this scope`

- [ ] **Step 3: 写最小实现**

`Cargo.toml` 的 `[dependencies]` 追加：

```toml
axum = { workspace = true }
chrono = { workspace = true }
```

`[dev-dependencies]` 追加：

```toml
tower = { workspace = true }
```

创建 `crates/backend/nomifun-robot/src/dto.rs`：

```rust
//! Wire shapes shared by the device face and the management REST face.

use serde::{Deserialize, Serialize};

use crate::registry::RobotRecord;

/// A robot as shown in the UI. Never carries the token or its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotDto {
    pub robot_id: String,
    pub name: String,
    pub companion_id: Option<String>,
    pub board: String,
    pub firmware_version: String,
    /// RFC 3339, or `null` if never seen.
    pub last_seen: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp_millis(0).expect("epoch is valid"))
        .to_rfc3339()
}

impl From<&RobotRecord> for RobotDto {
    fn from(record: &RobotRecord) -> Self {
        Self {
            robot_id: record.robot_id.clone(),
            name: record.name.clone(),
            companion_id: record.companion_id.clone(),
            board: record.board.clone(),
            firmware_version: record.firmware_version.clone(),
            last_seen: record.last_seen.map(ms_to_rfc3339),
            created_at: ms_to_rfc3339(record.created_at),
        }
    }
}

/// The fields we read out of the firmware's device report body. Everything else
/// in the report (partition table, chip info, heap) is ignored on purpose.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceReportBody {
    #[serde(default)]
    pub mac_address: Option<String>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub application: DeviceReportApplication,
    #[serde(default)]
    pub board: DeviceReportBoard,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceReportApplication {
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceReportBoard {
    #[serde(default, rename = "type")]
    pub board_type: Option<String>,
}
```

创建 `crates/backend/nomifun-robot/src/routes/mod.rs`：

```rust
//! HTTP faces: the unauthenticated device face (`/robot/*`) and the
//! owner-authenticated management face (`/api/robots*`).

pub mod device;

pub use device::{RobotDeviceState, build_ota_response, device_router};
```

`routes/device.rs` 顶部写入（放在测试模块之前）：

```rust
//! The device face. Mounted with `nest("/robot", ...)` **outside** the session
//! auth layers — a robot has no cookie and no session.
//!
//! The OTA response is the only channel that configures the firmware's server
//! address, and it must obey two firmware rules absolutely: always include
//! `websocket`, never include `mqtt` (any `mqtt` object makes the firmware pick
//! MQTT with no fallback path).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::dto::DeviceReportBody;
use crate::endpoint::EndpointAdvertiser;
use crate::registry::{RobotRecord, RobotRegistry, RobotReport};

/// Message shown/spoken by the device while waiting to be claimed.
const ACTIVATION_MESSAGE: &str = "请在 nomifun 中输入此码绑定伙伴";
/// How long the firmware waits between activation polls, in milliseconds.
const ACTIVATION_TIMEOUT_MS: i64 = 30_000;

/// Shared state of the device face.
#[derive(Clone)]
pub struct RobotDeviceState {
    pub registry: Arc<RobotRegistry>,
    pub advertiser: Arc<dyn EndpointAdvertiser>,
}

/// Router for the device face, to be nested under `/robot`.
pub fn device_router(state: RobotDeviceState) -> Router {
    Router::new()
        .route("/ota", post(ota_report).get(ota_report_get))
        .route("/ota/activate", post(activate))
        .with_state(state)
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn local_timezone_offset_minutes() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

/// Build the OTA response body.
///
/// `firmware.version` deliberately echoes the device's own version so the
/// firmware concludes it is up to date — hosting firmware images is a
/// non-goal (spec §1), and `url` stays an empty string so the firmware's
/// parse of that field is still safe.
pub fn build_ota_response(
    record: &RobotRecord,
    token: &str,
    ws_url: Option<&str>,
    now_ms: i64,
    tz_offset_minutes: i32,
) -> serde_json::Value {
    let mut body = json!({
        "websocket": {
            "url": ws_url.unwrap_or_default(),
            "token": token,
            "version": 1,
        },
        "server_time": {
            "timestamp": now_ms,
            "timezone_offset": tz_offset_minutes,
        },
        "firmware": {
            "version": record.firmware_version,
            "url": "",
        },
    });
    if let Some(code) = &record.activation_code {
        body["activation"] = json!({
            "code": code,
            "message": ACTIVATION_MESSAGE,
            "timeout_ms": ACTIVATION_TIMEOUT_MS,
        });
    }
    body
}

fn report_from(headers: &HeaderMap, body: &DeviceReportBody) -> Option<RobotReport> {
    let robot_id = header(headers, "device-id").or_else(|| body.mac_address.clone())?;
    let client_id = header(headers, "client-id")
        .or_else(|| body.uuid.clone())
        .unwrap_or_default();
    Some(RobotReport {
        robot_id,
        client_id,
        board: body.board.board_type.clone().unwrap_or_else(|| "unknown".to_owned()),
        firmware_version: body.application.version.clone().unwrap_or_else(|| "0.0.0".to_owned()),
    })
}

async fn ota_report(
    State(state): State<RobotDeviceState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<DeviceReportBody>>,
) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let Some(report) = report_from(&headers, &body) else {
        return (StatusCode::BAD_REQUEST, "missing Device-Id").into_response();
    };
    let robot_id = report.robot_id.clone();
    let (record, token) = match state.registry.upsert_on_report(report, now_ms()).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::error!(%robot_id, %error, "robot: OTA upsert failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "registry write failed").into_response();
        }
    };
    let ws_url = state.advertiser.websocket_url(peer.ip());
    tracing::info!(
        robot_id = %record.robot_id,
        board = %record.board,
        bound = record.companion_id.is_some(),
        reachable = ws_url.is_some(),
        "robot: OTA report"
    );
    Json(build_ota_response(
        &record,
        &token,
        ws_url.as_deref(),
        now_ms(),
        local_timezone_offset_minutes(),
    ))
    .into_response()
}

/// The firmware uses GET when its report body is empty.
async fn ota_report_get(
    state: State<RobotDeviceState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    ota_report(state, peer, headers, None).await
}

async fn activate(State(state): State<RobotDeviceState>, headers: HeaderMap) -> Response {
    let Some(robot_id) = header(&headers, "device-id") else {
        return (StatusCode::BAD_REQUEST, "missing Device-Id").into_response();
    };
    let bound = state
        .registry
        .list()
        .await
        .into_iter()
        .find(|r| r.robot_id == robot_id)
        .and_then(|r| r.companion_id);
    if bound.is_some() {
        StatusCode::OK.into_response()
    } else {
        // 202 tells the firmware "still waiting for the user"; it keeps polling.
        StatusCode::ACCEPTED.into_response()
    }
}

/// `IpAddr` is only used through `ConnectInfo`; keep the import meaningful for
/// readers of `report_from`'s neighbours.
const _: fn() -> Option<IpAddr> = || None;
```

`lib.rs` 加 `pub mod dto;` 与 `pub mod routes;`。

**注意**：`device_router` 的 handler 用了 `ConnectInfo<SocketAddr>`，所以宿主挂载时该 router 必须经 `.into_make_service_with_connect_info::<SocketAddr>()` 的 listener 服务。桌面 LAN listener 已经这么做了（`desktop.rs:1152`）；loopback listener 没有。Task 18 装配时对此显式处理：若 `ConnectInfo` 缺失则回退到「取第一个候选接口」。为此把 `ConnectInfo<SocketAddr>` 换成 `Option<ConnectInfo<SocketAddr>>` 并在 `peer` 缺失时用 `IpAddr::V4(Ipv4Addr::UNSPECIFIED)`——本任务先按 `ConnectInfo` 写，Task 18 再改为 `Option<...>` 并补一个「无 ConnectInfo 时仍回 200」的测试。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot device`
Expected: PASS — 5 个测试全过

若 `Request::post(...)` 不存在（axum 0.8 用 `http::Request::builder()`），改为：

```rust
Request::builder()
    .method("POST")
    .uri("/ota")
    .header("Device-Id", "aa:bb:cc:dd:ee:ff")
    .header("content-type", "application/json")
    .body(Body::from(REPORT_BODY))
    .unwrap()
```

`oneshot` 需要 router 带 `ConnectInfo`——`Router::oneshot` 不提供 `ConnectInfo`，所以本任务测试改用 `device_router(state).into_service()` 无法注入。改法：把 handler 的 `ConnectInfo<SocketAddr>` 改为 `Option<ConnectInfo<SocketAddr>>`（本任务就改，不等 Task 18），缺失时 peer 用 `IpAddr::V4(Ipv4Addr::new(0,0,0,0))`；`best_host` 对 0.0.0.0 匹配不到前缀会回落第一个接口，测试断言的 `192.168.1.20` 依然成立。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add OTA report and activation endpoints"
```

---

### Task 7: 状态注册表与 robot.status 事件

**Files:**
- Create: `crates/backend/nomifun-robot/src/status.rs`
- Create: `crates/backend/nomifun-robot/src/events.rs`
- Modify: `crates/backend/nomifun-robot/src/dto.rs`（加 `RobotStatusDto`）
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod status;`、`pub mod events;`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `nomifun-api-types`、`nomifun-realtime`）

**Interfaces:**
- Consumes: `nomifun_realtime::UserEventSink`、`nomifun_api_types::WebSocketMessage`
- Produces:
  - `pub enum RobotPhase { Offline, Idle, Listening, Speaking }`，`pub fn as_wire(&self) -> &'static str`
  - `pub struct RobotStatusDto { pub robot_id: String, pub companion_id: Option<String>, pub phase: String, pub changed_at: i64 }`
  - `pub struct RobotEventEmitter`，`pub fn new(user_events: std::sync::Arc<dyn UserEventSink>) -> Self`，`pub fn emit_status(&self, owner_id: &str, payload: &RobotStatusDto)`
  - `pub struct RobotStatusRegistry`，`pub fn new(emitter: RobotEventEmitter, owner_id: String) -> Self`，`pub async fn publish(&self, robot_id: &str, companion_id: Option<&str>, phase: RobotPhase, now_ms: i64)`，`pub async fn snapshot(&self) -> Vec<RobotStatusDto>`，`pub async fn mark_offline(&self, robot_id: &str, now_ms: i64)`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/status.rs`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RobotEventEmitter;
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::UserEventSink;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recording {
        sent: Mutex<Vec<(String, WebSocketMessage<serde_json::Value>)>>,
    }

    impl UserEventSink for Recording {
        fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
            self.sent.lock().unwrap().push((user_id.to_owned(), event));
        }
    }

    fn registry(sink: Arc<Recording>) -> RobotStatusRegistry {
        RobotStatusRegistry::new(RobotEventEmitter::new(sink), "owner-1".to_owned())
    }

    #[test]
    fn phase_wire_names_match_the_shared_contract() {
        assert_eq!(RobotPhase::Offline.as_wire(), "offline");
        assert_eq!(RobotPhase::Idle.as_wire(), "idle");
        assert_eq!(RobotPhase::Listening.as_wire(), "listening");
        assert_eq!(RobotPhase::Speaking.as_wire(), "speaking");
    }

    #[tokio::test]
    async fn publish_emits_to_the_owner_and_updates_the_snapshot() {
        let sink = Arc::new(Recording::default());
        let reg = registry(sink.clone());

        reg.publish("aa:bb", Some("companion-1"), RobotPhase::Listening, 1_700_000_000_000).await;

        let sent = sink.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "owner-1");
        assert_eq!(sent[0].1.name, "robot.status");
        let payload: RobotStatusDto = serde_json::from_value(sent[0].1.data.clone()).unwrap();
        assert_eq!(payload.robot_id, "aa:bb");
        assert_eq!(payload.companion_id.as_deref(), Some("companion-1"));
        assert_eq!(payload.phase, "listening");
        assert_eq!(payload.changed_at, 1_700_000_000_000);
        drop(sent);

        let snap = reg.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].phase, "listening");
    }

    #[tokio::test]
    async fn repeated_identical_phase_does_not_re_emit() {
        let sink = Arc::new(Recording::default());
        let reg = registry(sink.clone());
        reg.publish("aa:bb", None, RobotPhase::Idle, 1).await;
        reg.publish("aa:bb", None, RobotPhase::Idle, 2).await;
        assert_eq!(sink.sent.lock().unwrap().len(), 1, "same phase is not news");
        assert_eq!(reg.snapshot().await[0].changed_at, 1, "changed_at keeps the first transition");
    }

    #[tokio::test]
    async fn mark_offline_transitions_and_emits() {
        let sink = Arc::new(Recording::default());
        let reg = registry(sink.clone());
        reg.publish("aa:bb", Some("c1"), RobotPhase::Speaking, 1).await;
        reg.mark_offline("aa:bb", 5).await;

        let snap = reg.snapshot().await;
        assert_eq!(snap[0].phase, "offline");
        assert_eq!(snap[0].changed_at, 5);
        assert_eq!(snap[0].companion_id.as_deref(), Some("c1"), "binding survives going offline");
        assert_eq!(sink.sent.lock().unwrap().len(), 2);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot status`
Expected: FAIL — `cannot find enum RobotPhase in this scope`

- [ ] **Step 3: 写最小实现**

`Cargo.toml` 的 `[dependencies]` 追加：

```toml
nomifun-api-types = { workspace = true }
nomifun-realtime = { workspace = true }
```

`dto.rs` 追加：

```rust
/// Live phase of one robot. Shares the wire shape between the REST snapshot
/// (`GET /api/robots/statuses`) and the `robot.status` WebSocket event, so the
/// UI's three-stage consumer can merge them by `changed_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotStatusDto {
    pub robot_id: String,
    pub companion_id: Option<String>,
    /// One of `offline` | `idle` | `listening` | `speaking`.
    pub phase: String,
    /// Milliseconds since the epoch.
    pub changed_at: i64,
}
```

创建 `crates/backend/nomifun-robot/src/events.rs`：

```rust
//! Realtime emission for robot status.
//!
//! Robot presence is not turn-scoped — an idle desktop must still learn that the
//! robot on the desk went away — so it travels on the owner's realtime channel
//! (`UserEventSink`), exactly like `ssh.status`.

use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::UserEventSink;
use tracing::error;

use crate::dto::RobotStatusDto;

/// Emits robot transitions to the installation owner only.
#[derive(Clone)]
pub struct RobotEventEmitter {
    user_events: Arc<dyn UserEventSink>,
}

impl RobotEventEmitter {
    pub fn new(user_events: Arc<dyn UserEventSink>) -> Self {
        Self { user_events }
    }

    /// `robot.status` — one robot changed phase.
    pub fn emit_status(&self, owner_id: &str, payload: &RobotStatusDto) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "robot: status event serialize failed");
                return;
            }
        };
        self.user_events
            .send_to_user(owner_id, WebSocketMessage::new("robot.status", value));
    }
}
```

创建 `crates/backend/nomifun-robot/src/status.rs`（测试模块之前）：

```rust
//! Live phase tracking. One writer (`publish`) both updates the snapshot and
//! emits the event, so the REST snapshot and the WebSocket stream cannot drift.

use std::collections::BTreeMap;

use tokio::sync::RwLock;

use crate::dto::RobotStatusDto;
use crate::events::RobotEventEmitter;

/// What a robot is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotPhase {
    Offline,
    Idle,
    Listening,
    Speaking,
}

impl RobotPhase {
    /// Wire name (shared contract with the UI).
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Speaking => "speaking",
        }
    }
}

/// Owns current phases and publishes transitions.
pub struct RobotStatusRegistry {
    emitter: RobotEventEmitter,
    owner_id: String,
    inner: RwLock<BTreeMap<String, RobotStatusDto>>,
}

impl RobotStatusRegistry {
    pub fn new(emitter: RobotEventEmitter, owner_id: String) -> Self {
        Self { emitter, owner_id, inner: RwLock::new(BTreeMap::new()) }
    }

    /// Record a phase and emit it. A repeat of the current phase is dropped
    /// (identical phases are not news and would spam the socket).
    pub async fn publish(
        &self,
        robot_id: &str,
        companion_id: Option<&str>,
        phase: RobotPhase,
        now_ms: i64,
    ) {
        let payload = {
            let mut map = self.inner.write().await;
            if let Some(existing) = map.get(robot_id)
                && existing.phase == phase.as_wire()
            {
                return;
            }
            let payload = RobotStatusDto {
                robot_id: robot_id.to_owned(),
                companion_id: companion_id
                    .map(str::to_owned)
                    .or_else(|| map.get(robot_id).and_then(|e| e.companion_id.clone())),
                phase: phase.as_wire().to_owned(),
                changed_at: now_ms,
            };
            map.insert(robot_id.to_owned(), payload.clone());
            payload
        };
        self.emitter.emit_status(&self.owner_id, &payload);
    }

    /// Transition a robot to offline, preserving its known binding.
    pub async fn mark_offline(&self, robot_id: &str, now_ms: i64) {
        self.publish(robot_id, None, RobotPhase::Offline, now_ms).await;
    }

    /// All known phases, ordered by `robot_id`.
    pub async fn snapshot(&self) -> Vec<RobotStatusDto> {
        self.inner.read().await.values().cloned().collect()
    }
}
```

`lib.rs` 加 `pub mod events;` 与 `pub mod status;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot status`
Expected: PASS — 4 个测试全过

若 `if let ... && ...`（let-chains）报错，改成嵌套 `if let Some(existing) = map.get(robot_id) { if existing.phase == phase.as_wire() { return; } }`。仓库是 edition 2024，let-chains 应可用；以实际编译结果为准。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add status registry and robot.status realtime event"
```

---

### Task 8: WS 端点、LanWsSource 与会话骨架

**Files:**
- Create: `crates/backend/nomifun-robot/src/lan_source.rs`
- Create: `crates/backend/nomifun-robot/src/session.rs`
- Modify: `crates/backend/nomifun-robot/src/routes/device.rs`（加 `/v1` 路由与 WS 升级 handler）
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod lan_source;`、`pub mod session;`、`RobotGateway`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `futures-util`、`uuid`）

**Interfaces:**
- Consumes: `link::{AcceptedLink, Frame, LinkError, RobotIdentity, RobotLinkSink, RobotLinkSource, RobotLinkStream}`、`protocol::{parse_device_message, serialize_server_message, DeviceMessage, ServerMessage}`、`registry::RobotRegistry`、`status::{RobotPhase, RobotStatusRegistry}`
- Produces:
  - `pub struct LanWsSource`，`pub fn new() -> (std::sync::Arc<Self>, LanLinkAcceptor)`
  - `pub struct LanLinkAcceptor`，`pub async fn offer(&self, link: AcceptedLink) -> Result<(), LinkError>`
  - `pub struct SessionDeps { pub registry: std::sync::Arc<RobotRegistry>, pub status: std::sync::Arc<RobotStatusRegistry> }`
  - `pub async fn run_session(link: AcceptedLink, deps: SessionDeps)`
  - `pub const PING_INTERVAL_SECS: u64 = 60;`
  - `pub struct RobotGateway`，`pub fn new(deps: SessionDeps) -> Self`，`pub async fn serve(self: std::sync::Arc<Self>, sources: Vec<std::sync::Arc<dyn RobotLinkSource>>)`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/session.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RobotEventEmitter;
    use crate::link::{AcceptedLink, Frame, LinkError, RobotIdentity, RobotLinkSink, RobotLinkStream};
    use crate::registry::{RobotRegistry, RobotReport};
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::UserEventSink;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    struct NullSink;
    impl UserEventSink for NullSink {
        fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
    }

    /// A sink that records everything written, and a stream driven by a channel.
    struct RecordingSink(Arc<Mutex<Vec<Frame>>>);
    #[async_trait::async_trait]
    impl RobotLinkSink for RecordingSink {
        async fn send(&mut self, frame: Frame) -> Result<(), LinkError> {
            self.0.lock().unwrap().push(frame);
            Ok(())
        }
        async fn close(&mut self) {}
    }

    struct ChannelStream(mpsc::Receiver<Frame>);
    #[async_trait::async_trait]
    impl RobotLinkStream for ChannelStream {
        async fn next(&mut self) -> Option<Result<Frame, LinkError>> {
            self.0.recv().await.map(Ok)
        }
    }

    async fn harness(
        bound: bool,
    ) -> (SessionDeps, AcceptedLink, mpsc::Sender<Frame>, Arc<Mutex<Vec<Frame>>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (record, token) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:ff".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        if bound {
            registry
                .claim(record.activation_code.as_deref().unwrap(), "0190f5fe-7c00-7a00-8000-0000000000aa")
                .await
                .unwrap();
        }
        let _ = token;
        let status = Arc::new(crate::status::RobotStatusRegistry::new(
            RobotEventEmitter::new(Arc::new(NullSink)),
            "owner-1".to_owned(),
        ));
        let written = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel(16);
        let link = AcceptedLink {
            identity: RobotIdentity {
                robot_id: "aa:bb:cc:dd:ee:ff".into(),
                client_id: "cid".into(),
                peer: "192.168.1.9".into(),
            },
            sink: Box::new(RecordingSink(written.clone())),
            stream: Box::new(ChannelStream(rx)),
        };
        (SessionDeps { registry, status }, link, tx, written, dir)
    }

    fn texts(frames: &Arc<Mutex<Vec<Frame>>>) -> Vec<serde_json::Value> {
        frames
            .lock()
            .unwrap()
            .iter()
            .filter_map(|f| match f {
                Frame::Text(t) => serde_json::from_str(t).ok(),
                Frame::Binary(_) => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn bound_device_gets_a_server_hello_after_its_hello() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true}}"#.into(),
        ))
        .await
        .unwrap();
        // Closing the stream ends the session loop.
        drop(tx);
        task.await.unwrap();

        let sent = texts(&written);
        assert_eq!(sent.len(), 1, "exactly one server hello");
        assert_eq!(sent[0]["type"], "hello");
        assert_eq!(sent[0]["audio_params"]["sample_rate"], 24000);
        assert!(sent[0]["session_id"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[tokio::test]
    async fn unbound_device_is_refused_after_hello_with_no_server_hello() {
        let (deps, link, tx, written, _dir) = harness(false).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        task.await.unwrap();

        let sent = texts(&written);
        assert!(
            sent.iter().all(|m| m["type"] != "hello"),
            "an unbound robot must never get a session"
        );
    }

    #[tokio::test]
    async fn audio_before_hello_is_ignored_not_fatal() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Binary(bytes::Bytes::from_static(&[0xfc, 0x01]))).await.unwrap();
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        drop(tx);
        task.await.unwrap();

        assert_eq!(texts(&written).len(), 1, "session still established after stray audio");
    }

    #[tokio::test]
    async fn unknown_message_type_does_not_end_the_session() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps));

        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(r#"{"type":"brand_new_thing","x":1}"#.into())).await.unwrap();
        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"stop"}"#.into()))
            .await
            .unwrap();
        drop(tx);
        task.await.unwrap();

        assert_eq!(texts(&written)[0]["type"], "hello");
    }

    #[tokio::test]
    async fn session_marks_offline_when_the_link_drops() {
        let (deps, link, tx, _written, _dir) = harness(true).await;
        let status = deps.status.clone();
        let task = tokio::spawn(run_session(link, deps));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        drop(tx);
        task.await.unwrap();

        let snap = status.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].phase, "offline");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot session`
Expected: FAIL — `cannot find function run_session in this scope`

- [ ] **Step 3: 写最小实现**

`Cargo.toml` 的 `[dependencies]` 追加：

```toml
futures-util = { workspace = true }
uuid = { workspace = true }
```

创建 `crates/backend/nomifun-robot/src/session.rs`（测试模块之前）：

```rust
//! One actor per connected robot.
//!
//! This task owns the read loop, the handshake, the keepalive ping, and (from
//! later tasks) the audio pipelines. It never touches a socket directly — only
//! [`AcceptedLink`] halves — so the same actor serves a LAN WebSocket today and
//! a relay tunnel later.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

use crate::link::{AcceptedLink, Frame, RobotLinkSink};
use crate::protocol::{DeviceMessage, ServerMessage, parse_device_message, serialize_server_message};
use crate::registry::RobotRegistry;
use crate::status::{RobotPhase, RobotStatusRegistry};

/// The firmware declares a link dead after 120 s of silence; ping at half that.
pub const PING_INTERVAL_SECS: u64 = 60;

/// Everything a session actor needs from the host.
#[derive(Clone)]
pub struct SessionDeps {
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Outbound frames are funnelled through one writer task so the ping timer and
/// the pipelines never contend for the sink.
struct Writer {
    tx: mpsc::Sender<Frame>,
}

impl Writer {
    fn spawn(mut sink: Box<dyn RobotLinkSink>) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<Frame>(64);
        let handle = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if sink.send(frame).await.is_err() {
                    break;
                }
            }
            sink.close().await;
        });
        (Self { tx }, handle)
    }

    async fn send_json(&self, msg: &ServerMessage) {
        let _ = self.tx.send(Frame::Text(serialize_server_message(msg))).await;
    }
}

/// Run one robot session to completion. Returns when the inbound stream ends.
pub async fn run_session(link: AcceptedLink, deps: SessionDeps) {
    let AcceptedLink { identity, sink, mut stream } = link;
    let robot_id = identity.robot_id.clone();
    let (writer, writer_task) = Writer::spawn(sink);

    let mut session_id: Option<String> = None;
    let mut companion_id: Option<String> = None;
    let mut ping = interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping.tick().await; // the first tick is immediate; skip it

    loop {
        tokio::select! {
            _ = ping.tick() => {
                if let Some(sid) = &session_id {
                    writer.send_json(&ServerMessage::Ping { session_id: sid.clone() }).await;
                }
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break };
                let Ok(frame) = frame else { break };
                match frame {
                    Frame::Binary(_) if session_id.is_none() => {
                        // Wake-word audio can arrive before `listen start`; before
                        // the handshake it is simply noise.
                        continue;
                    }
                    Frame::Binary(_) => {
                        // Uplink audio handling lands in the uplink pipeline task.
                        continue;
                    }
                    Frame::Text(raw) => {
                        let message = match parse_device_message(&raw) {
                            Ok(m) => m,
                            Err(error) => {
                                tracing::warn!(%robot_id, %error, "robot: unparseable text frame");
                                continue;
                            }
                        };
                        match message {
                            DeviceMessage::Hello(hello) => {
                                let record = deps.registry.list().await.into_iter().find(|r| r.robot_id == robot_id);
                                let Some(bound) = record.as_ref().and_then(|r| r.companion_id.clone()) else {
                                    tracing::warn!(%robot_id, "robot: refusing session, not bound to a companion");
                                    break;
                                };
                                let sid = uuid::Uuid::new_v4().to_string();
                                tracing::info!(
                                    %robot_id,
                                    companion_id = %bound,
                                    session_id = %sid,
                                    protocol_version = hello.version,
                                    mcp = hello.mcp,
                                    "robot: session established"
                                );
                                writer.send_json(&ServerMessage::Hello { session_id: sid.clone() }).await;
                                deps.status
                                    .publish(&robot_id, Some(&bound), RobotPhase::Idle, now_ms())
                                    .await;
                                session_id = Some(sid);
                                companion_id = Some(bound);
                            }
                            DeviceMessage::Unknown { raw_type } => {
                                tracing::debug!(%robot_id, %raw_type, "robot: unknown message type");
                            }
                            // Listen / Abort / Mcp handling is wired by the
                            // pipeline and bridge tasks.
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let _ = companion_id;
    deps.status.mark_offline(&robot_id, now_ms()).await;
    drop(writer);
    let _ = writer_task.await;
    tracing::info!(%robot_id, "robot: session ended");
}
```

创建 `crates/backend/nomifun-robot/src/lan_source.rs`：

```rust
//! LAN WebSocket source.
//!
//! LAN links are push-driven: axum hands us an already-upgraded socket from a
//! request handler. To fit the pull-shaped [`RobotLinkSource`] contract (which a
//! future outbound relay source needs), the handler `offer`s links into a queue
//! and `run` drains it.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use crate::link::{AcceptedLink, LinkError, RobotLinkSource};

/// Handle given to HTTP handlers so they can hand off upgraded sockets.
#[derive(Clone)]
pub struct LanLinkAcceptor {
    tx: mpsc::Sender<AcceptedLink>,
}

impl LanLinkAcceptor {
    /// Hand an authenticated link to the gateway.
    pub async fn offer(&self, link: AcceptedLink) -> Result<(), LinkError> {
        self.tx.send(link).await.map_err(|_| LinkError::Closed)
    }
}

/// The LAN source: drains what handlers offered.
pub struct LanWsSource {
    rx: Mutex<mpsc::Receiver<AcceptedLink>>,
}

impl LanWsSource {
    /// Build the source and the handle its HTTP handlers use.
    pub fn new() -> (Arc<Self>, LanLinkAcceptor) {
        let (tx, rx) = mpsc::channel(8);
        (Arc::new(Self { rx: Mutex::new(rx) }), LanLinkAcceptor { tx })
    }
}

#[async_trait::async_trait]
impl RobotLinkSource for LanWsSource {
    fn name(&self) -> &'static str {
        "lan-ws"
    }

    async fn run(self: Arc<Self>, accept: mpsc::Sender<AcceptedLink>) -> anyhow::Result<()> {
        let mut rx = self.rx.lock().await;
        while let Some(link) = rx.recv().await {
            if accept.send(link).await.is_err() {
                break;
            }
        }
        Ok(())
    }
}
```

`lib.rs` 追加模块与 gateway：

```rust
pub mod lan_source;
pub mod session;

use std::sync::Arc;

/// Owns the accept loop: every [`link::RobotLinkSource`] feeds one channel, and
/// each accepted link becomes a detached [`session::run_session`] task.
pub struct RobotGateway {
    deps: session::SessionDeps,
}

impl RobotGateway {
    pub fn new(deps: session::SessionDeps) -> Self {
        Self { deps }
    }

    /// Run until every source has finished.
    pub async fn serve(self: Arc<Self>, sources: Vec<Arc<dyn link::RobotLinkSource>>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<link::AcceptedLink>(8);
        for source in sources {
            let tx = tx.clone();
            let name = source.name();
            tokio::spawn(async move {
                if let Err(error) = source.run(tx).await {
                    tracing::error!(source = name, %error, "robot: link source stopped");
                }
            });
        }
        drop(tx);
        while let Some(link) = rx.recv().await {
            let deps = self.deps.clone();
            tokio::spawn(session::run_session(link, deps));
        }
    }
}
```

`routes/device.rs` 追加 WS 升级 handler（并把 `device_router` 的 `Router::new()` 链上 `.route("/v1", get(ws_upgrade))`；`RobotDeviceState` 加 `pub acceptor: crate::lan_source::LanLinkAcceptor` 字段，测试里用 `LanWsSource::new().1` 填充）：

```rust
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};

use crate::link::{AcceptedLink, Frame, LinkError, RobotIdentity, RobotLinkSink, RobotLinkStream};

async fn ws_upgrade(
    State(state): State<RobotDeviceState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let token = header(&headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer ").map(str::to_owned).or(Some(v)))
        .unwrap_or_default();
    let Some(record) = state.registry.resolve_token(&token).await else {
        tracing::warn!("robot: websocket rejected, unknown token");
        return (StatusCode::UNAUTHORIZED, "unknown device token").into_response();
    };
    let identity = RobotIdentity {
        robot_id: record.robot_id.clone(),
        client_id: record.client_id.clone(),
        peer: peer.map(|ConnectInfo(p)| p.ip().to_string()).unwrap_or_else(|| "unknown".to_owned()),
    };
    let acceptor = state.acceptor.clone();
    upgrade.on_upgrade(move |socket| async move {
        let (sink, stream) = split_ws(socket);
        let link = AcceptedLink { identity, sink: Box::new(sink), stream: Box::new(stream) };
        if acceptor.offer(link).await.is_err() {
            tracing::error!("robot: gateway not accepting links");
        }
    })
}

struct WsSink(futures_util::stream::SplitSink<WebSocket, Message>);
struct WsStream(futures_util::stream::SplitStream<WebSocket>);

fn split_ws(socket: WebSocket) -> (WsSink, WsStream) {
    use futures_util::StreamExt;
    let (tx, rx) = socket.split();
    (WsSink(tx), WsStream(rx))
}

#[async_trait::async_trait]
impl RobotLinkSink for WsSink {
    async fn send(&mut self, frame: Frame) -> Result<(), LinkError> {
        use futures_util::SinkExt;
        let message = match frame {
            Frame::Text(t) => Message::Text(t.into()),
            Frame::Binary(b) => Message::Binary(b),
        };
        self.0.send(message).await.map_err(|e| LinkError::Transport(e.to_string()))
    }

    async fn close(&mut self) {
        use futures_util::SinkExt;
        let _ = self.0.close().await;
    }
}

#[async_trait::async_trait]
impl RobotLinkStream for WsStream {
    async fn next(&mut self) -> Option<Result<Frame, LinkError>> {
        use futures_util::StreamExt;
        loop {
            match self.0.next().await? {
                Ok(Message::Text(t)) => return Some(Ok(Frame::Text(t.to_string()))),
                Ok(Message::Binary(b)) => return Some(Ok(Frame::Binary(b))),
                Ok(Message::Close(_)) => return None,
                // Ping/Pong are handled by axum; keep reading.
                Ok(_) => continue,
                Err(e) => return Some(Err(LinkError::Transport(e.to_string()))),
            }
        }
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot session`
Expected: PASS — 5 个测试全过

`Message::Text` 在 axum 0.8 收的是 `Utf8Bytes`；若类型不匹配，用 `Message::Text(t.into())` 或 `Message::Text(t.as_str().into())`，以编译器提示为准。`Message::Binary(b)` 收 `Bytes`，与 `Frame::Binary` 同类型。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add websocket endpoint, LAN link source and session actor"
```

---

### Task 9: 音频基建（Opus / WAV / 重采样 / 容器解码）

**Files:**
- Create: `crates/backend/nomifun-robot/src/audio/mod.rs`
- Create: `crates/backend/nomifun-robot/src/audio/opus.rs`
- Create: `crates/backend/nomifun-robot/src/audio/wav.rs`
- Create: `crates/backend/nomifun-robot/src/audio/resample.rs`
- Create: `crates/backend/nomifun-robot/src/audio/container.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod audio;`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `audiopus`、`symphonia`）

**Interfaces:**
- Consumes: `protocol::{UPLINK_SAMPLE_RATE, DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS}`
- Produces:
  - `pub struct AudioBuffer { pub pcm: Vec<i16>, pub sample_rate: u32 }`
  - `pub struct OpusStreamDecoder`，`pub fn new_uplink() -> anyhow::Result<Self>`，`pub fn decode(&mut self, packet: &[u8]) -> anyhow::Result<Vec<i16>>`
  - `pub struct OpusStreamEncoder`，`pub fn new_downlink() -> anyhow::Result<Self>`，`pub fn encode_frames(&mut self, pcm: &[i16]) -> anyhow::Result<Vec<Vec<u8>>>`
  - `pub fn pcm_to_wav(pcm: &[i16], sample_rate: u32) -> Vec<u8>`
  - `pub fn resample_linear(pcm: &[i16], from: u32, to: u32) -> Vec<i16>`
  - `pub fn decode_container(bytes: &[u8], mime_hint: Option<&str>) -> anyhow::Result<AudioBuffer>`
  - `pub const UPLINK_FRAME_SAMPLES: usize = 960;` `pub const DOWNLINK_FRAME_SAMPLES: usize = 1440;`

**依赖决策**：Opus 用 **`audiopus`**（其 `audiopus_sys` 在系统无 libopus 时从源码编译，三平台打包不必预装 libopus；`opus` crate 走 pkg-config，Windows/macOS 打包负担大）。**spec §13 列了 `rubato`，本计划刻意不引入**：下行 TTS 走 `format:"pcm"` 时 provider 返回的就是 24 kHz，与设备声明完全一致，无需重采样；只有容器格式解码出非 24 kHz 时才需要，单声道语音用线性插值足够。`resample_linear` 的签名即抽象边界，将来若音质不足可原地换成 rubato 实现。

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/audio/mod.rs`：

```rust
//! Audio primitives: Opus codec wrappers, WAV packing, resampling, container
//! decode. Everything here is synchronous and allocation-explicit so the
//! pipelines can be tested without a device or a provider.

pub mod container;
pub mod opus;
pub mod resample;
pub mod wav;

pub use container::decode_container;
pub use opus::{DOWNLINK_FRAME_SAMPLES, OpusStreamDecoder, OpusStreamEncoder, UPLINK_FRAME_SAMPLES};
pub use resample::resample_linear;
pub use wav::pcm_to_wav;

/// Mono PCM with its sample rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBuffer {
    pub pcm: Vec<i16>,
    pub sample_rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 440 Hz mono tone, `ms` milliseconds at `rate`.
    pub(crate) fn tone(rate: u32, ms: u32) -> Vec<i16> {
        let n = (rate as u64 * ms as u64 / 1000) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16
            })
            .collect()
    }

    #[test]
    fn opus_round_trip_preserves_length_and_energy() {
        let pcm = tone(16_000, 180); // three 60 ms frames
        let mut encoder = OpusStreamEncoder::new_uplink_for_test().unwrap();
        let frames = encoder.encode_frames(&pcm).unwrap();
        assert_eq!(frames.len(), 3, "180 ms of 16 kHz audio is three 60 ms frames");
        assert!(frames.iter().all(|f| !f.is_empty()));

        let mut decoder = OpusStreamDecoder::new_uplink().unwrap();
        let mut decoded = Vec::new();
        for frame in &frames {
            decoded.extend(decoder.decode(frame).unwrap());
        }
        assert_eq!(decoded.len(), pcm.len(), "sample count survives the round trip");

        let rms = |s: &[i16]| (s.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
        let (before, after) = (rms(&pcm), rms(&decoded));
        assert!(
            (after / before - 1.0).abs() < 0.35,
            "lossy but recognisable: before={before:.0} after={after:.0}"
        );
    }

    #[test]
    fn downlink_encoder_emits_1440_sample_frames() {
        let pcm = tone(24_000, 120);
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let frames = encoder.encode_frames(&pcm).unwrap();
        assert_eq!(frames.len(), 2, "120 ms of 24 kHz audio is two 60 ms frames");
        assert_eq!(DOWNLINK_FRAME_SAMPLES, 1440);
        assert_eq!(UPLINK_FRAME_SAMPLES, 960);
    }

    #[test]
    fn trailing_partial_frame_is_zero_padded_not_dropped() {
        let pcm = tone(24_000, 90); // one full frame + half a frame
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let frames = encoder.encode_frames(&pcm).unwrap();
        assert_eq!(frames.len(), 2, "the tail is padded so the last words are not cut off");
    }

    #[test]
    fn wav_header_is_44_bytes_and_declares_the_rate() {
        let wav = pcm_to_wav(&[0, 1, -1, 32767], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 4 * 2);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1, "mono");
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16, "16-bit");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
    }

    #[test]
    fn resample_scales_length_and_is_identity_at_equal_rates() {
        let pcm = tone(16_000, 100);
        let up = resample_linear(&pcm, 16_000, 24_000);
        assert!((up.len() as i64 - 2400).abs() <= 1, "got {}", up.len());
        assert_eq!(resample_linear(&pcm, 16_000, 16_000), pcm, "same rate copies through");
        assert!(resample_linear(&[], 16_000, 24_000).is_empty());
    }

    #[test]
    fn decode_container_reads_our_own_wav() {
        let pcm = tone(24_000, 60);
        let wav = pcm_to_wav(&pcm, 24_000);
        let buffer = decode_container(&wav, Some("audio/wav")).unwrap();
        assert_eq!(buffer.sample_rate, 24_000);
        assert_eq!(buffer.pcm.len(), pcm.len());
    }

    #[test]
    fn decode_container_rejects_garbage() {
        assert!(decode_container(b"not audio at all", None).is_err());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot audio`
Expected: FAIL — `unresolved module or unlinked crate 'container'` / `cannot find struct OpusStreamEncoder`

- [ ] **Step 3: 写最小实现**

先加依赖（用 `cargo add` 让 Cargo 解析出真实可用版本，再把版本回填 workspace）：

```bash
cargo add --package nomifun-robot audiopus
cargo add --package nomifun-robot symphonia --features mp3,wav,isomp4,aac,vorbis
```

把解析出的版本写进根 `Cargo.toml` 的 `[workspace.dependencies]`，crate 内改为 `{ workspace = true }`。

**若 `audiopus_sys` 的 vendored 构建失败**（缺 cmake / 编译器）：改用 `cargo add --package nomifun-robot opus` 并在 `docs/` 的构建说明里记录「需系统 libopus（Linux `libopus-dev`、macOS `brew install opus`、Windows vcpkg）」。两条路径的 API 差异只在 `opus.rs` 一个文件内，其余代码不受影响。

创建 `crates/backend/nomifun-robot/src/audio/opus.rs`：

```rust
//! Opus wrappers pinned to the two shapes this gateway needs.
//!
//! Uplink is fixed by firmware: 16 kHz mono 60 ms. Downlink is what we declared
//! in the server hello: 24 kHz mono 60 ms. Nothing else is supported on purpose
//! — a mismatched frame size makes the device's decoder fail outright.

use audiopus::coder::{Decoder, Encoder};
use audiopus::{Application, Channels, SampleRate};

use crate::protocol::{DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS, UPLINK_SAMPLE_RATE};

/// Samples in one 60 ms uplink frame (16 kHz).
pub const UPLINK_FRAME_SAMPLES: usize = (UPLINK_SAMPLE_RATE * FRAME_DURATION_MS / 1000) as usize;
/// Samples in one 60 ms downlink frame (24 kHz).
pub const DOWNLINK_FRAME_SAMPLES: usize = (DOWNLINK_SAMPLE_RATE * FRAME_DURATION_MS / 1000) as usize;

/// Largest Opus packet we will ever produce or accept, per RFC 6716.
const MAX_PACKET_BYTES: usize = 1275;

fn rate(hz: u32) -> anyhow::Result<SampleRate> {
    Ok(match hz {
        16_000 => SampleRate::Hz16000,
        24_000 => SampleRate::Hz24000,
        other => anyhow::bail!("unsupported opus sample rate {other}"),
    })
}

/// Decodes uplink packets from the device.
pub struct OpusStreamDecoder {
    inner: Decoder,
    frame_samples: usize,
}

impl OpusStreamDecoder {
    /// 16 kHz mono — the only uplink shape the firmware produces.
    pub fn new_uplink() -> anyhow::Result<Self> {
        Ok(Self {
            inner: Decoder::new(rate(UPLINK_SAMPLE_RATE)?, Channels::Mono)?,
            frame_samples: UPLINK_FRAME_SAMPLES,
        })
    }

    /// Decode one packet into PCM. The buffer is sized for a full frame and
    /// truncated to what Opus actually produced.
    pub fn decode(&mut self, packet: &[u8]) -> anyhow::Result<Vec<i16>> {
        let mut pcm = vec![0i16; self.frame_samples];
        let produced = self.inner.decode(Some(packet), &mut pcm[..], false)?;
        pcm.truncate(produced);
        Ok(pcm)
    }
}

/// Encodes PCM into 60 ms packets.
pub struct OpusStreamEncoder {
    inner: Encoder,
    frame_samples: usize,
}

impl OpusStreamEncoder {
    /// 24 kHz mono — matches the `audio_params` in our server hello.
    pub fn new_downlink() -> anyhow::Result<Self> {
        Ok(Self {
            inner: Encoder::new(rate(DOWNLINK_SAMPLE_RATE)?, Channels::Mono, Application::Voip)?,
            frame_samples: DOWNLINK_FRAME_SAMPLES,
        })
    }

    /// 16 kHz mono. Only used by tests that mimic the device's uplink.
    pub fn new_uplink_for_test() -> anyhow::Result<Self> {
        Ok(Self {
            inner: Encoder::new(rate(UPLINK_SAMPLE_RATE)?, Channels::Mono, Application::Voip)?,
            frame_samples: UPLINK_FRAME_SAMPLES,
        })
    }

    /// Split `pcm` into whole frames and encode each. A trailing partial frame
    /// is zero-padded rather than dropped, otherwise the last syllable of every
    /// sentence disappears.
    pub fn encode_frames(&mut self, pcm: &[i16]) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        for chunk in pcm.chunks(self.frame_samples) {
            let mut frame = chunk.to_vec();
            frame.resize(self.frame_samples, 0);
            let mut packet = vec![0u8; MAX_PACKET_BYTES];
            let written = self.inner.encode(&frame, &mut packet[..])?;
            packet.truncate(written);
            out.push(packet);
        }
        Ok(out)
    }
}
```

创建 `crates/backend/nomifun-robot/src/audio/wav.rs`：

```rust
//! 44-byte canonical WAV header + PCM. The ASR upload path wants a container,
//! and this is the one every provider accepts without negotiation.

/// Wrap mono 16-bit PCM in a WAV container.
pub fn pcm_to_wav(pcm: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}
```

创建 `crates/backend/nomifun-robot/src/audio/resample.rs`:

```rust
//! Linear resampling for mono speech.
//!
//! The happy path never calls this: TTS is requested as 24 kHz PCM, exactly the
//! rate we declared to the device. It only runs when a provider returns a
//! container at some other rate. Linear interpolation is adequate for mono
//! speech; this signature is the seam where a polyphase resampler (rubato) would
//! drop in if quality ever proves insufficient.

/// Resample mono PCM from `from` Hz to `to` Hz.
pub fn resample_linear(pcm: &[i16], from: u32, to: u32) -> Vec<i16> {
    if pcm.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to {
        return pcm.to_vec();
    }
    let out_len = ((pcm.len() as u64 * to as u64) / from as u64).max(1) as usize;
    let ratio = from as f64 / to as f64;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let left = src.floor() as usize;
            let frac = src - left as f64;
            let a = pcm[left.min(pcm.len() - 1)] as f64;
            let b = pcm[(left + 1).min(pcm.len() - 1)] as f64;
            (a + (b - a) * frac).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect()
}
```

创建 `crates/backend/nomifun-robot/src/audio/container.rs`：

```rust
//! Container decode for TTS audio that did not come back as raw PCM.
//!
//! Providers differ: OpenAI-compatible endpoints honour `format: "pcm"`, others
//! return mp3 or an Ogg container regardless. symphonia probes the bytes, so the
//! mime hint is advisory only.

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::AudioBuffer;

/// Decode any container symphonia recognises into mono PCM.
/// Multi-channel input is downmixed by averaging.
pub fn decode_container(bytes: &[u8], mime_hint: Option<&str>) -> anyhow::Result<AudioBuffer> {
    let source = std::io::Cursor::new(bytes.to_vec());
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(mime) = mime_hint {
        hint.mime_type(mime);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("audio has no default track"))?;
    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut pcm: Vec<i16> = Vec::new();
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        if sample_rate == 0 {
            sample_rate = spec.rate;
        }
        let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let channels = spec.channels.count().max(1);
        if channels == 1 {
            pcm.extend_from_slice(buffer.samples());
        } else {
            pcm.extend(buffer.samples().chunks(channels).map(|frame| {
                (frame.iter().map(|s| *s as i32).sum::<i32>() / channels as i32) as i16
            }));
        }
    }
    if pcm.is_empty() || sample_rate == 0 {
        anyhow::bail!("decoded no audio samples");
    }
    Ok(AudioBuffer { pcm, sample_rate })
}
```

`lib.rs` 加 `pub mod audio;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot audio`
Expected: PASS — 7 个测试全过

`audiopus` 的 `Decoder::decode` / `Encoder::encode` 参数顺序与 `SampleRate`/`Channels`/`Application` 的路径按解析到的版本可能不同（0.2 是 `audiopus::coder::{Decoder,Encoder}`，0.3 相同）。以编译器提示与 `cargo doc -p audiopus --open` 为准修正；语义不变：解码传 `Some(packet)` 与输出缓冲，编码传输入帧与输出缓冲并返回写入字节数。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add opus codec, wav packing, resampling and container decode"
```

---

### Task 10: VAD 抽象与能量档

**Files:**
- Create: `crates/backend/nomifun-robot/src/vad/mod.rs`
- Create: `crates/backend/nomifun-robot/src/vad/energy.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod vad;`）

**Interfaces:**
- Consumes: `audio::UPLINK_FRAME_SAMPLES`
- Produces:
  - `pub struct VadTuning { pub sensitivity: f32, pub min_silence_ms: u32 }`，`pub fn from_profile(engine: &str, sensitivity: f32, min_silence_ms: u32) -> Self`，`Default` 为 `sensitivity: 0.5, min_silence_ms: 700`
  - `pub enum VadDecision { Silence, Speech, EndOfUtterance }`
  - `pub trait VadEngine: Send { fn name(&self) -> &'static str; fn push_frame(&mut self, pcm: &[i16]) -> VadDecision; fn reset(&mut self); }`
  - `pub struct EnergyVad`，`pub fn new(tuning: VadTuning) -> Self`
  - `pub fn frame_ms(samples: usize, sample_rate: u32) -> u32`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/vad/mod.rs`：

```rust
//! Endpointing. The device's own VAD only drives its LED and never ends a turn,
//! so `mode=auto` sessions end **only** when this decides they did.

pub mod energy;

pub use energy::EnergyVad;

/// Tunables exposed per companion (`voice.vad` in the companion profile).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadTuning {
    /// 0.0 (permissive: almost everything is speech) … 1.0 (strict).
    pub sensitivity: f32,
    /// Trailing silence, in milliseconds, that ends an utterance.
    pub min_silence_ms: u32,
}

impl Default for VadTuning {
    fn default() -> Self {
        Self { sensitivity: 0.5, min_silence_ms: 700 }
    }
}

impl VadTuning {
    /// Build from companion profile values, clamping to sane ranges.
    /// `engine` is accepted (and ignored) here so callers can pass the profile
    /// field verbatim; engine selection happens in the pipeline.
    pub fn from_profile(engine: &str, sensitivity: f32, min_silence_ms: u32) -> Self {
        let _ = engine;
        Self {
            sensitivity: sensitivity.clamp(0.0, 1.0),
            min_silence_ms: min_silence_ms.clamp(200, 3000),
        }
    }
}

/// What one frame told us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// No speech yet (or still trailing silence, but not long enough).
    Silence,
    /// Speech in progress.
    Speech,
    /// Speech had started and trailing silence has now passed the threshold.
    EndOfUtterance,
}

/// A frame-at-a-time endpointer.
pub trait VadEngine: Send {
    /// Stable name for logs and the UI.
    fn name(&self) -> &'static str;
    /// Feed one frame of 16 kHz mono PCM.
    fn push_frame(&mut self, pcm: &[i16]) -> VadDecision;
    /// Forget all state (called between turns).
    fn reset(&mut self);
}

/// Duration of a frame, in whole milliseconds.
pub fn frame_ms(samples: usize, sample_rate: u32) -> u32 {
    if sample_rate == 0 {
        return 0;
    }
    (samples as u64 * 1000 / sample_rate as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::UPLINK_FRAME_SAMPLES;

    fn loud() -> Vec<i16> {
        (0..UPLINK_FRAME_SAMPLES)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                ((t * 300.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
            })
            .collect()
    }

    fn quiet() -> Vec<i16> {
        vec![0i16; UPLINK_FRAME_SAMPLES]
    }

    #[test]
    fn frame_ms_matches_the_60ms_contract() {
        assert_eq!(frame_ms(UPLINK_FRAME_SAMPLES, 16_000), 60);
        assert_eq!(frame_ms(0, 16_000), 0);
        assert_eq!(frame_ms(960, 0), 0);
    }

    #[test]
    fn tuning_clamps_out_of_range_profile_values() {
        let t = VadTuning::from_profile("silero", 5.0, 10);
        assert_eq!(t.sensitivity, 1.0);
        assert_eq!(t.min_silence_ms, 200);
        let t = VadTuning::from_profile("silero", -1.0, 99_999);
        assert_eq!(t.sensitivity, 0.0);
        assert_eq!(t.min_silence_ms, 3000);
        assert_eq!(VadTuning::default().min_silence_ms, 700);
    }

    #[test]
    fn energy_vad_ends_an_utterance_after_the_silence_window() {
        // 700 ms of silence at 60 ms per frame is 12 frames.
        let mut vad = EnergyVad::new(VadTuning { sensitivity: 0.5, min_silence_ms: 700 });
        assert_eq!(vad.name(), "energy");

        // Leading silence never ends a turn that never started.
        for _ in 0..20 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence);
        }
        // Speech.
        for _ in 0..5 {
            assert_eq!(vad.push_frame(&loud()), VadDecision::Speech);
        }
        // Trailing silence: eleven frames is not yet 700 ms.
        for i in 0..11 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence, "frame {i}");
        }
        assert_eq!(vad.push_frame(&quiet()), VadDecision::EndOfUtterance);
    }

    #[test]
    fn energy_vad_resets_between_turns() {
        let mut vad = EnergyVad::new(VadTuning::default());
        for _ in 0..3 {
            vad.push_frame(&loud());
        }
        vad.reset();
        for _ in 0..30 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence, "reset forgets the speech");
        }
    }

    #[test]
    fn brief_gaps_inside_speech_do_not_end_the_turn() {
        let mut vad = EnergyVad::new(VadTuning { sensitivity: 0.5, min_silence_ms: 700 });
        for _ in 0..4 {
            vad.push_frame(&loud());
        }
        for _ in 0..6 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence);
        }
        assert_eq!(vad.push_frame(&loud()), VadDecision::Speech, "speech resumes");
        for _ in 0..11 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence);
        }
        assert_eq!(vad.push_frame(&quiet()), VadDecision::EndOfUtterance, "the counter restarted");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot vad`
Expected: FAIL — `unresolved module or unlinked crate 'energy'`

- [ ] **Step 3: 写最小实现**

创建 `crates/backend/nomifun-robot/src/vad/energy.rs`：

```rust
//! RMS energy endpointer. No model, no weights, no warm-up — the dependable
//! floor under the Silero engine and its fallback when ONNX is unavailable.

use crate::audio::UPLINK_SAMPLE_RATE_HINT;

use super::{VadDecision, VadEngine, VadTuning, frame_ms};

/// RMS below which a frame counts as silence, at `sensitivity = 0.0` and `1.0`.
/// Speech at a normal distance from an INMP441 lands well above 900; room noise
/// sits under 250.
const RMS_AT_MIN_SENSITIVITY: f32 = 200.0;
const RMS_AT_MAX_SENSITIVITY: f32 = 1200.0;

/// Energy-threshold VAD.
pub struct EnergyVad {
    tuning: VadTuning,
    threshold: f32,
    speech_started: bool,
    trailing_silence_ms: u32,
}

impl EnergyVad {
    pub fn new(tuning: VadTuning) -> Self {
        let threshold = RMS_AT_MIN_SENSITIVITY
            + (RMS_AT_MAX_SENSITIVITY - RMS_AT_MIN_SENSITIVITY) * tuning.sensitivity;
        Self { tuning, threshold, speech_started: false, trailing_silence_ms: 0 }
    }

    fn rms(pcm: &[i16]) -> f32 {
        if pcm.is_empty() {
            return 0.0;
        }
        let sum: f64 = pcm.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum / pcm.len() as f64).sqrt() as f32
    }
}

impl VadEngine for EnergyVad {
    fn name(&self) -> &'static str {
        "energy"
    }

    fn push_frame(&mut self, pcm: &[i16]) -> VadDecision {
        let is_speech = Self::rms(pcm) >= self.threshold;
        if is_speech {
            self.speech_started = true;
            self.trailing_silence_ms = 0;
            return VadDecision::Speech;
        }
        if !self.speech_started {
            // Leading silence: nothing to end.
            return VadDecision::Silence;
        }
        self.trailing_silence_ms =
            self.trailing_silence_ms.saturating_add(frame_ms(pcm.len(), UPLINK_SAMPLE_RATE_HINT));
        if self.trailing_silence_ms > self.tuning.min_silence_ms {
            self.speech_started = false;
            self.trailing_silence_ms = 0;
            VadDecision::EndOfUtterance
        } else {
            VadDecision::Silence
        }
    }

    fn reset(&mut self) {
        self.speech_started = false;
        self.trailing_silence_ms = 0;
    }
}
```

`audio/mod.rs` 追加一行常量再导出（VAD 只需要采样率数值，不该反向依赖 protocol）：

```rust
/// Uplink sample rate, re-exported so the VAD engines need not import `protocol`.
pub const UPLINK_SAMPLE_RATE_HINT: u32 = crate::protocol::UPLINK_SAMPLE_RATE;
```

`lib.rs` 加 `pub mod vad;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot vad`
Expected: PASS — 5 个测试全过

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/
git commit -m "feat(robot): add VAD abstraction with energy engine"
```

---

### Task 11: Silero VAD（ONNX）

**Files:**
- Create: `crates/backend/nomifun-robot/src/vad/silero.rs`
- Create: `crates/backend/nomifun-robot/assets/silero_vad.onnx`（下载，非手写）
- Modify: `crates/backend/nomifun-robot/src/vad/mod.rs`（加 `pub mod silero;`、`build_engine`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `ort`、`ndarray`）

**Interfaces:**
- Consumes: `VadEngine`、`VadTuning`、`VadDecision`、`audio::UPLINK_SAMPLE_RATE_HINT`
- Produces:
  - `pub struct SileroVad`，`pub fn new(tuning: VadTuning) -> anyhow::Result<Self>`
  - `pub fn build_engine(engine: &str, tuning: VadTuning) -> Box<dyn VadEngine>` —— `"silero"` 优先 Silero，失败记 warning 并回落 `EnergyVad`；其他值直接用 `EnergyVad`

- [ ] **Step 1: 写失败测试**

`vad/mod.rs` 的测试模块追加：

```rust
    #[test]
    fn build_engine_prefers_silero_and_never_fails() {
        let engine = build_engine("silero", VadTuning::default());
        assert!(
            engine.name() == "silero" || engine.name() == "energy",
            "silero is preferred but a load failure must degrade, not panic"
        );
    }

    #[test]
    fn build_engine_honours_an_explicit_energy_choice() {
        assert_eq!(build_engine("energy", VadTuning::default()).name(), "energy");
        assert_eq!(build_engine("anything-else", VadTuning::default()).name(), "energy");
    }

    #[test]
    fn silero_ends_an_utterance_on_real_speech_then_silence() {
        let Ok(mut vad) = crate::vad::silero::SileroVad::new(VadTuning {
            sensitivity: 0.5,
            min_silence_ms: 700,
        }) else {
            eprintln!("skipping: ONNX runtime unavailable in this environment");
            return;
        };
        assert_eq!(vad.name(), "silero");

        // Silero wants 512-sample chunks at 16 kHz; the engine buffers whatever
        // frame size we hand it, so feed it our real 60 ms frames.
        let speech = speech_like_frame();
        let mut saw_speech = false;
        for _ in 0..10 {
            if vad.push_frame(&speech) == VadDecision::Speech {
                saw_speech = true;
            }
        }
        assert!(saw_speech, "a speech-shaped signal must register as speech");

        let quiet = vec![0i16; crate::audio::UPLINK_FRAME_SAMPLES];
        let mut ended = false;
        for _ in 0..30 {
            if vad.push_frame(&quiet) == VadDecision::EndOfUtterance {
                ended = true;
                break;
            }
        }
        assert!(ended, "trailing silence must end the utterance");
    }

    /// A frame with speech-like structure: a 150 Hz fundamental plus formant-ish
    /// harmonics and a little noise. Pure tones can read as non-speech to Silero.
    fn speech_like_frame() -> Vec<i16> {
        use crate::audio::UPLINK_FRAME_SAMPLES;
        (0..UPLINK_FRAME_SAMPLES)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                let tau = std::f32::consts::TAU;
                let v = (t * 150.0 * tau).sin() * 0.5
                    + (t * 700.0 * tau).sin() * 0.3
                    + (t * 1800.0 * tau).sin() * 0.15
                    + ((i * 2654435761usize % 1000) as f32 / 1000.0 - 0.5) * 0.1;
                (v * 9000.0) as i16
            })
            .collect()
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot vad`
Expected: FAIL — `cannot find function build_engine in this scope`

- [ ] **Step 3: 写最小实现**

先取权重（约 2.2 MB，Silero VAD v5 的 ONNX 导出）：

```bash
mkdir -p crates/backend/nomifun-robot/assets
curl -fL -o crates/backend/nomifun-robot/assets/silero_vad.onnx \
  https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
ls -l crates/backend/nomifun-robot/assets/silero_vad.onnx   # 期望 1.5-2.5 MB
```

若该 URL 404（上游改过目录），在 https://github.com/snakers4/silero-vad 仓库里找 `silero_vad.onnx` 的当前路径下载；文件必须是 ONNX（前 4 字节含 `\x08`，`file` 报 `data`），不是 LFS 指针文本（若 `head -c 20` 看到 `version https://git-lfs` 就是取错了，改用 `?download=` 或 release 附件）。

加依赖：

```bash
cargo add --package nomifun-robot ort
cargo add --package nomifun-robot ndarray
```

把解析出的版本回填根 `Cargo.toml` 的 `[workspace.dependencies]`，crate 内改 `{ workspace = true }`。

创建 `crates/backend/nomifun-robot/src/vad/silero.rs`：

```rust
//! Silero VAD via ONNX Runtime.
//!
//! The model is a streaming RNN: it takes 512-sample chunks at 16 kHz plus a
//! carried state tensor and returns P(speech) for that chunk. We buffer the
//! device's 60 ms (960-sample) frames into 512-sample chunks and take the
//! maximum probability across the chunks belonging to one frame, so a frame that
//! starts silent but ends in speech still counts as speech.
//!
//! Weights are embedded at compile time — no runtime download, no user setup.

use std::sync::Arc;

use ort::session::Session;
use ort::value::Value;

use crate::audio::UPLINK_SAMPLE_RATE_HINT;

use super::{VadDecision, VadEngine, VadTuning, frame_ms};

/// Chunk size the model expects at 16 kHz.
const CHUNK: usize = 512;
/// Hidden state shape: 2 layers × 1 batch × 128 features.
const STATE_LEN: usize = 2 * 1 * 128;
/// Probability threshold at `sensitivity = 0.0` and `1.0`.
const P_AT_MIN_SENSITIVITY: f32 = 0.20;
const P_AT_MAX_SENSITIVITY: f32 = 0.80;

static MODEL: &[u8] = include_bytes!("../../assets/silero_vad.onnx");

/// Streaming Silero endpointer.
pub struct SileroVad {
    session: Session,
    state: Vec<f32>,
    pending: Vec<i16>,
    tuning: VadTuning,
    threshold: f32,
    speech_started: bool,
    trailing_silence_ms: u32,
}

impl SileroVad {
    /// Load the embedded model. Fails if ONNX Runtime is unavailable — callers
    /// must fall back to [`super::EnergyVad`], never propagate.
    pub fn new(tuning: VadTuning) -> anyhow::Result<Self> {
        let session = Session::builder()?.commit_from_memory(MODEL)?;
        let threshold =
            P_AT_MIN_SENSITIVITY + (P_AT_MAX_SENSITIVITY - P_AT_MIN_SENSITIVITY) * tuning.sensitivity;
        Ok(Self {
            session,
            state: vec![0.0; STATE_LEN],
            pending: Vec::with_capacity(CHUNK * 2),
            tuning,
            threshold,
            speech_started: false,
            trailing_silence_ms: 0,
        })
    }

    /// Run one 512-sample chunk, updating the carried state.
    fn speech_probability(&mut self, chunk: &[i16]) -> anyhow::Result<f32> {
        let samples: Vec<f32> = chunk.iter().map(|s| *s as f32 / 32768.0).collect();
        let input = Value::from_array(([1usize, CHUNK], samples))?;
        let state = Value::from_array(([2usize, 1, 128], self.state.clone()))?;
        let rate = Value::from_array(([1usize], vec![UPLINK_SAMPLE_RATE_HINT as i64]))?;

        let outputs = self
            .session
            .run(ort::inputs!["input" => input, "state" => state, "sr" => rate])?;

        let (_, probability) = outputs["output"].try_extract_tensor::<f32>()?;
        let p = probability.first().copied().unwrap_or(0.0);
        let (_, next_state) = outputs["stateN"].try_extract_tensor::<f32>()?;
        if next_state.len() == STATE_LEN {
            self.state.copy_from_slice(next_state);
        }
        Ok(p)
    }
}

impl VadEngine for SileroVad {
    fn name(&self) -> &'static str {
        "silero"
    }

    fn push_frame(&mut self, pcm: &[i16]) -> VadDecision {
        self.pending.extend_from_slice(pcm);
        let mut max_p = 0.0f32;
        let mut ran = false;
        while self.pending.len() >= CHUNK {
            let chunk: Vec<i16> = self.pending.drain(..CHUNK).collect();
            match self.speech_probability(&chunk) {
                Ok(p) => {
                    ran = true;
                    max_p = max_p.max(p);
                }
                Err(error) => {
                    // A mid-stream inference error must not end the call; treat
                    // the frame as silence and keep going.
                    tracing::warn!(%error, "robot: silero inference failed for a chunk");
                }
            }
        }
        if !ran {
            return VadDecision::Silence;
        }
        if max_p >= self.threshold {
            self.speech_started = true;
            self.trailing_silence_ms = 0;
            return VadDecision::Speech;
        }
        if !self.speech_started {
            return VadDecision::Silence;
        }
        self.trailing_silence_ms =
            self.trailing_silence_ms.saturating_add(frame_ms(pcm.len(), UPLINK_SAMPLE_RATE_HINT));
        if self.trailing_silence_ms > self.tuning.min_silence_ms {
            self.speech_started = false;
            self.trailing_silence_ms = 0;
            VadDecision::EndOfUtterance
        } else {
            VadDecision::Silence
        }
    }

    fn reset(&mut self) {
        self.state.iter_mut().for_each(|v| *v = 0.0);
        self.pending.clear();
        self.speech_started = false;
        self.trailing_silence_ms = 0;
    }
}

/// Keep `Arc` meaningful for readers: the session is owned, not shared, because
/// the carried state is per-stream.
const _: fn() -> Option<Arc<()>> = || None;
```

`vad/mod.rs` 追加：

```rust
pub mod silero;

/// Build the engine a companion asked for. Silero is preferred; if its model or
/// the ONNX runtime is unavailable this degrades to [`EnergyVad`] with a warning
/// rather than breaking the voice link.
pub fn build_engine(engine: &str, tuning: VadTuning) -> Box<dyn VadEngine> {
    if engine == "silero" {
        match silero::SileroVad::new(tuning) {
            Ok(vad) => return Box::new(vad),
            Err(error) => {
                tracing::warn!(%error, "robot: silero VAD unavailable, falling back to energy VAD");
            }
        }
    }
    Box::new(EnergyVad::new(tuning))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot vad`
Expected: PASS — 8 个测试全过（`silero_ends_an_utterance_on_real_speech_then_silence` 在 ONNX 不可用时打印 skip 后通过）

`ort` 的 API 在 1.x → 2.x 之间变化很大（`Session::builder()`、`inputs!`、`try_extract_tensor` 的返回形状、输入名 `input`/`state`/`sr` 与输出名 `output`/`stateN`）。核实办法：`cargo doc -p ort --open` 看 `Session::run` 与 `Value::from_array`；模型的真实输入输出名用 Python 一行确认（若本机有 python 与 onnx：`python3 -c "import onnx;m=onnx.load('crates/backend/nomifun-robot/assets/silero_vad.onnx');print([i.name for i in m.graph.input],[o.name for o in m.graph.output])"`），按实际名字改。若 `ort` 需要下载 onnxruntime 二进制而环境离线，`SileroVad::new` 会失败 → `build_engine` 回落能量档，链路依然可用；此时在 commit message 里注明「Silero 未在本机验证」，留给真机验收。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add silero ONNX VAD with energy fallback"
```

---

### Task 12: 分句器与情绪标记

**Files:**
- Create: `crates/backend/nomifun-robot/src/pipeline/mod.rs`
- Create: `crates/backend/nomifun-robot/src/pipeline/sentence.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod pipeline;`）

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub struct SentenceSplitter`（`Default`），`pub fn push(&mut self, chunk: &str) -> Vec<String>`，`pub fn flush(&mut self) -> Option<String>`
  - `pub fn strip_emotion(sentence: &str) -> (Option<&'static str>, String)`
  - `pub fn normalize_emotion(name: &str) -> &'static str`
  - `pub const EMOTIONS: [&str; 21]`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/pipeline/sentence.rs`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_chinese_and_ascii_terminators() {
        let mut s = SentenceSplitter::default();
        assert_eq!(s.push("你好。今天"), vec!["你好。"]);
        assert_eq!(s.push("天气不错！"), vec!["今天天气不错！"]);
        assert_eq!(s.push("Hi there. Bye?"), vec!["Hi there.", " Bye?"]);
        assert!(s.push("no terminator yet").is_empty());
        assert_eq!(s.flush().as_deref(), Some("no terminator yet"));
        assert!(s.flush().is_none(), "flush drains");
    }

    #[test]
    fn newline_ends_a_sentence_too() {
        let mut s = SentenceSplitter::default();
        assert_eq!(s.push("第一行\n第二行"), vec!["第一行"]);
    }

    #[test]
    fn decimal_points_do_not_split_english_numbers() {
        let mut s = SentenceSplitter::default();
        assert!(s.push("It is 3.5 degrees").is_empty(), "3.5 is not a sentence end");
        assert_eq!(s.push(" outside.").len(), 1);
    }

    #[test]
    fn whitespace_only_output_is_never_emitted() {
        let mut s = SentenceSplitter::default();
        assert!(s.push("   \n  ").is_empty(), "blank lines are not sentences");
        assert!(s.flush().is_none());
    }

    #[test]
    fn strips_a_leading_emotion_marker() {
        let (emotion, text) = strip_emotion("[emotion:happy] 你好呀");
        assert_eq!(emotion, Some("happy"));
        assert_eq!(text, "你好呀");
    }

    #[test]
    fn unknown_emotion_name_falls_back_to_neutral() {
        let (emotion, text) = strip_emotion("[emotion:ecstatic]太好了");
        assert_eq!(emotion, Some("neutral"), "the firmware only knows 21 names");
        assert_eq!(text, "太好了");
    }

    #[test]
    fn sentence_without_marker_is_untouched() {
        let (emotion, text) = strip_emotion("就这样");
        assert_eq!(emotion, None);
        assert_eq!(text, "就这样");
    }

    #[test]
    fn marker_must_be_at_the_start_to_count() {
        let (emotion, text) = strip_emotion("我觉得 [emotion:sad] 不太好");
        assert_eq!(emotion, None);
        assert_eq!(text, "我觉得 [emotion:sad] 不太好");
    }

    #[test]
    fn normalize_accepts_all_21_firmware_names() {
        assert_eq!(EMOTIONS.len(), 21);
        for name in EMOTIONS {
            assert_eq!(normalize_emotion(name), name, "{name} must survive normalisation");
        }
        assert_eq!(normalize_emotion("HAPPY"), "happy", "case-insensitive");
        assert_eq!(normalize_emotion(" happy "), "happy", "trimmed");
        assert_eq!(normalize_emotion("nonsense"), "neutral");
        assert_eq!(normalize_emotion(""), "neutral");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot sentence`
Expected: FAIL — `cannot find struct SentenceSplitter in this scope`

- [ ] **Step 3: 写最小实现**

创建 `crates/backend/nomifun-robot/src/pipeline/mod.rs`：

```rust
//! The two audio pipelines and the text plumbing between them.

pub mod sentence;

pub use sentence::{EMOTIONS, SentenceSplitter, normalize_emotion, strip_emotion};
```

创建 `crates/backend/nomifun-robot/src/pipeline/sentence.rs`（测试模块之前）：

```rust
//! Incremental sentence splitting and emotion markers.
//!
//! The model streams text; the device needs whole sentences (one `sentence_start`
//! plus its audio at a time). Splitting eagerly is what keeps first-audio latency
//! low, so this runs on every stream chunk.
//!
//! Emotion travels as a leading `[emotion:name]` marker the system prompt asks
//! for. It is stripped before display and TTS, and mapped onto the 21 names the
//! firmware understands (anything else would silently become `neutral` on-device
//! anyway, so we normalise here and log nothing).

/// The exact emotion vocabulary the firmware maps to eye animations and gimbal
/// moves. Any other value degrades to `neutral` on-device.
pub const EMOTIONS: [&str; 21] = [
    "neutral",
    "happy",
    "laughing",
    "funny",
    "sad",
    "angry",
    "crying",
    "loving",
    "embarrassed",
    "surprised",
    "shocked",
    "thinking",
    "winking",
    "cool",
    "relaxed",
    "delicious",
    "kissy",
    "confident",
    "sleepy",
    "silly",
    "confused",
];

/// Map any name onto the firmware vocabulary, defaulting to `neutral`.
pub fn normalize_emotion(name: &str) -> &'static str {
    let needle = name.trim().to_ascii_lowercase();
    EMOTIONS.iter().copied().find(|known| *known == needle).unwrap_or("neutral")
}

/// Split a leading `[emotion:name]` marker off a sentence.
///
/// Returns the normalised emotion (only when a marker was present) and the
/// remaining text. A marker anywhere but the start is left alone — the model was
/// asked to lead with it, and rewriting mid-sentence text would mangle content.
pub fn strip_emotion(sentence: &str) -> (Option<&'static str>, String) {
    let trimmed = sentence.trim_start();
    let Some(rest) = trimmed.strip_prefix("[emotion:") else {
        return (None, sentence.to_owned());
    };
    let Some(end) = rest.find(']') else {
        return (None, sentence.to_owned());
    };
    let name = normalize_emotion(&rest[..end]);
    (Some(name), rest[end + 1..].trim_start().to_owned())
}

/// Terminators that end a sentence. `\n` counts: the model uses it as a beat.
const TERMINATORS: [char; 9] = ['。', '！', '？', '；', '!', '?', ';', '\n', '.'];

/// Buffers streamed text and hands back whole sentences.
#[derive(Debug, Default)]
pub struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    /// Feed a stream chunk; returns every sentence it completed.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        loop {
            let Some(cut) = self.find_terminator() else { break };
            let sentence: String = self.buf.drain(..cut).collect();
            if !sentence.trim().is_empty() {
                out.push(sentence);
            }
        }
        // A buffer holding only whitespace is not worth carrying.
        if self.buf.trim().is_empty() {
            self.buf.clear();
        }
        out
    }

    /// Byte index just past the first real terminator, if any.
    fn find_terminator(&self) -> Option<usize> {
        let bytes_len = self.buf.len();
        for (index, ch) in self.buf.char_indices() {
            if !TERMINATORS.contains(&ch) {
                continue;
            }
            let end = index + ch.len_utf8();
            // An ASCII '.' between digits is a decimal point, not a full stop.
            if ch == '.' {
                let before = self.buf[..index].chars().next_back();
                let after = if end < bytes_len { self.buf[end..].chars().next() } else { None };
                if before.is_some_and(|c| c.is_ascii_digit()) && after.is_some_and(|c| c.is_ascii_digit()) {
                    continue;
                }
                // A trailing '.' at the very end of the buffer may still be
                // mid-number ("3." + "5"); wait for more input.
                if after.is_none() {
                    return None;
                }
            }
            return Some(end);
        }
        None
    }

    /// Emit whatever is left (end of turn), if it is not blank.
    pub fn flush(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        if rest.trim().is_empty() { None } else { Some(rest) }
    }
}
```

`lib.rs` 加 `pub mod pipeline;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot sentence`
Expected: PASS — 9 个测试全过

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/
git commit -m "feat(robot): add incremental sentence splitter and emotion markers"
```

---

### Task 13: 服务接缝（SpeechServices / CompanionTurnDispatcher）

**Files:**
- Create: `crates/backend/nomifun-robot/src/services.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod services;`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `[features] test-support = []`）

**Interfaces:**
- Consumes: `audio::AudioBuffer`、`vad::VadTuning`
- Produces:
  - `pub struct SpeechContext { pub robot_id: String, pub companion_id: String }`
  - `pub trait SpeechServices: Send + Sync` — `transcribe(&self, ctx, wav: Vec<u8>) -> anyhow::Result<String>`、`synthesize(&self, ctx, text: &str) -> anyhow::Result<AudioBuffer>`、`explain_image(&self, ctx, jpeg: Vec<u8>, question: &str) -> anyhow::Result<String>`
  - `pub enum TurnEvent { Text(String), Done, Failed { message: String, provider_fault: bool } }`
  - `pub trait CompanionTurnDispatcher: Send + Sync` — `ensure_thread(&self, robot_id, companion_id) -> anyhow::Result<String>`、`dispatch(&self, conversation_id, text, use_fallback_model: bool) -> anyhow::Result<tokio::sync::mpsc::Receiver<TurnEvent>>`、`cancel(&self, conversation_id) -> anyhow::Result<()>`、`vad_tuning(&self, companion_id) -> VadTuning`、`has_fallback_model(&self, companion_id) -> bool`
  - `#[cfg(any(test, feature = "test-support"))] pub mod mock`：`MockSpeech`、`MockDispatcher`（可编程返回值，供管线与集成测试使用）

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/services.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::mock::{MockDispatcher, MockSpeech};
    use super::*;
    use std::sync::Arc;

    fn ctx() -> SpeechContext {
        SpeechContext { robot_id: "aa:bb".into(), companion_id: "c1".into() }
    }

    #[tokio::test]
    async fn mock_speech_returns_scripted_values_and_records_calls() {
        let speech = Arc::new(MockSpeech::new());
        speech.push_transcript("你好小智");
        speech.set_tts_rate(24_000);

        let text = speech.transcribe(&ctx(), vec![1, 2, 3]).await.unwrap();
        assert_eq!(text, "你好小智");
        assert_eq!(speech.transcribe_calls(), 1);

        let audio = speech.synthesize(&ctx(), "在呢").await.unwrap();
        assert_eq!(audio.sample_rate, 24_000);
        assert!(!audio.pcm.is_empty(), "mock synthesises silence of a plausible length");
        assert_eq!(speech.synthesized_text(), vec!["在呢".to_owned()]);
    }

    #[tokio::test]
    async fn mock_speech_can_be_scripted_to_fail() {
        let speech = Arc::new(MockSpeech::new());
        speech.fail_next_transcribe("network down");
        let error = speech.transcribe(&ctx(), vec![]).await.unwrap_err();
        assert!(error.to_string().contains("network down"));
        // The failure is consumed; the next call succeeds with the default.
        assert_eq!(speech.transcribe(&ctx(), vec![]).await.unwrap(), "");
    }

    #[tokio::test]
    async fn mock_dispatcher_streams_scripted_turn_events() {
        let dispatcher = Arc::new(MockDispatcher::new());
        dispatcher.script_turn(vec![
            TurnEvent::Text("你好".into()),
            TurnEvent::Text("呀。".into()),
            TurnEvent::Done,
        ]);

        let conversation = dispatcher.ensure_thread("aa:bb", "c1").await.unwrap();
        assert!(!conversation.is_empty());
        assert_eq!(dispatcher.ensure_thread("aa:bb", "c1").await.unwrap(), conversation, "same thread reused");

        let mut rx = dispatcher.dispatch(&conversation, "在吗", false).await.unwrap();
        let mut chunks = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                TurnEvent::Text(t) => chunks.push(t),
                TurnEvent::Done => break,
                TurnEvent::Failed { message, .. } => panic!("unexpected failure: {message}"),
            }
        }
        assert_eq!(chunks, vec!["你好".to_owned(), "呀。".to_owned()]);
        assert_eq!(dispatcher.dispatched_text(), vec!["在吗".to_owned()]);
    }

    #[tokio::test]
    async fn mock_dispatcher_records_fallback_usage_and_cancels() {
        let dispatcher = Arc::new(MockDispatcher::new());
        dispatcher.script_turn(vec![TurnEvent::Done]);
        dispatcher.set_has_fallback(true);
        assert!(dispatcher.has_fallback_model("c1").await);

        let _ = dispatcher.dispatch("conv-1", "hi", true).await.unwrap();
        assert_eq!(dispatcher.fallback_dispatches(), 1);

        dispatcher.cancel("conv-1").await.unwrap();
        assert_eq!(dispatcher.cancelled(), vec!["conv-1".to_owned()]);
    }

    #[tokio::test]
    async fn mock_dispatcher_serves_vad_tuning() {
        let dispatcher = Arc::new(MockDispatcher::new());
        assert_eq!(dispatcher.vad_tuning("c1").await, crate::vad::VadTuning::default());
        dispatcher.set_vad_tuning(crate::vad::VadTuning { sensitivity: 0.9, min_silence_ms: 400 });
        let tuning = dispatcher.vad_tuning("c1").await;
        assert_eq!(tuning.sensitivity, 0.9);
        assert_eq!(tuning.min_silence_ms, 400);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot services`
Expected: FAIL — `could not find 'mock' in the crate root` / `cannot find struct SpeechContext`

- [ ] **Step 3: 写最小实现**

`Cargo.toml` 追加（放在 `[dependencies]` 之前或之后均可）：

```toml
[features]
default = []
# Exposes the mock service implementations to integration tests.
test-support = []
```

创建 `crates/backend/nomifun-robot/src/services.rs`（测试模块之前）：

```rust
//! The seam between the robot pipelines and the rest of nomifun.
//!
//! The pipelines only ever see these traits, so every audio/text path is
//! testable without a model provider, a conversation service, or a device. The
//! real implementations live in [`crate::wiring`] — same crate, separate module,
//! so the dependency direction stays obvious.

use crate::audio::AudioBuffer;
use crate::vad::VadTuning;

/// Who a speech call is for. Both ids are logged and used to pick per-companion
/// model slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechContext {
    pub robot_id: String,
    pub companion_id: String,
}

/// ASR, TTS and one-shot vision.
#[async_trait::async_trait]
pub trait SpeechServices: Send + Sync {
    /// WAV bytes in, transcript out. An empty transcript is a valid answer
    /// (silence or noise) and must not be an error.
    async fn transcribe(&self, ctx: &SpeechContext, wav: Vec<u8>) -> anyhow::Result<String>;
    /// One sentence in, mono PCM out (any sample rate; the caller resamples).
    async fn synthesize(&self, ctx: &SpeechContext, text: &str) -> anyhow::Result<AudioBuffer>;
    /// A JPEG plus a question in, a natural-language answer out.
    async fn explain_image(
        &self,
        ctx: &SpeechContext,
        jpeg: Vec<u8>,
        question: &str,
    ) -> anyhow::Result<String>;
}

/// What the agent stream told us, reduced to what the downlink needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    /// An incremental slice of assistant text.
    Text(String),
    /// The turn finished normally.
    Done,
    /// The turn failed. `provider_fault` means the error looked like a model or
    /// provider problem, which is what makes a fallback-model retry sensible.
    Failed { message: String, provider_fault: bool },
}

/// Companion conversation access.
#[async_trait::async_trait]
pub trait CompanionTurnDispatcher: Send + Sync {
    /// Find or create the long-lived thread for this `(robot, companion)` pair.
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String>;
    /// Start a turn and stream its events.
    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<TurnEvent>>;
    /// Stop the in-flight turn (the public `cancel`, never a runtime kill).
    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()>;
    /// The companion's endpointing tunables.
    async fn vad_tuning(&self, companion_id: &str) -> VadTuning;
    /// Whether a fallback chat model is configured for this companion.
    async fn has_fallback_model(&self, companion_id: &str) -> bool;
}

#[cfg(any(test, feature = "test-support"))]
pub mod mock {
    //! Programmable doubles for the two seams above.

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    /// Scriptable [`SpeechServices`].
    #[derive(Default)]
    pub struct MockSpeech {
        transcripts: Mutex<std::collections::VecDeque<String>>,
        transcribe_failure: Mutex<Option<String>>,
        transcribe_calls: AtomicUsize,
        synthesized: Mutex<Vec<String>>,
        tts_rate: Mutex<u32>,
        vision_answer: Mutex<String>,
    }

    impl MockSpeech {
        pub fn new() -> Self {
            Self { tts_rate: Mutex::new(24_000), ..Default::default() }
        }

        /// Queue one transcript to return.
        pub fn push_transcript(&self, text: &str) {
            self.transcripts.lock().unwrap().push_back(text.to_owned());
        }

        /// Make the next `transcribe` fail.
        pub fn fail_next_transcribe(&self, message: &str) {
            *self.transcribe_failure.lock().unwrap() = Some(message.to_owned());
        }

        pub fn set_tts_rate(&self, rate: u32) {
            *self.tts_rate.lock().unwrap() = rate;
        }

        pub fn set_vision_answer(&self, text: &str) {
            *self.vision_answer.lock().unwrap() = text.to_owned();
        }

        pub fn transcribe_calls(&self) -> usize {
            self.transcribe_calls.load(Ordering::SeqCst)
        }

        pub fn synthesized_text(&self) -> Vec<String> {
            self.synthesized.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl SpeechServices for MockSpeech {
        async fn transcribe(&self, _ctx: &SpeechContext, _wav: Vec<u8>) -> anyhow::Result<String> {
            self.transcribe_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(message) = self.transcribe_failure.lock().unwrap().take() {
                anyhow::bail!(message);
            }
            Ok(self.transcripts.lock().unwrap().pop_front().unwrap_or_default())
        }

        async fn synthesize(&self, _ctx: &SpeechContext, text: &str) -> anyhow::Result<AudioBuffer> {
            self.synthesized.lock().unwrap().push(text.to_owned());
            let rate = *self.tts_rate.lock().unwrap();
            // ~80 ms of silence per character keeps frame counts realistic.
            let samples = (rate as usize / 1000) * 80 * text.chars().count().max(1);
            Ok(AudioBuffer { pcm: vec![0i16; samples], sample_rate: rate })
        }

        async fn explain_image(
            &self,
            _ctx: &SpeechContext,
            _jpeg: Vec<u8>,
            _question: &str,
        ) -> anyhow::Result<String> {
            Ok(self.vision_answer.lock().unwrap().clone())
        }
    }

    /// Scriptable [`CompanionTurnDispatcher`].
    #[derive(Default)]
    pub struct MockDispatcher {
        threads: Mutex<std::collections::BTreeMap<String, String>>,
        scripted: Mutex<std::collections::VecDeque<Vec<TurnEvent>>>,
        dispatched: Mutex<Vec<String>>,
        cancelled: Mutex<Vec<String>>,
        fallback_dispatches: AtomicUsize,
        has_fallback: AtomicBool,
        tuning: Mutex<Option<VadTuning>>,
    }

    impl MockDispatcher {
        pub fn new() -> Self {
            Self::default()
        }

        /// Queue the events one `dispatch` call will emit.
        pub fn script_turn(&self, events: Vec<TurnEvent>) {
            self.scripted.lock().unwrap().push_back(events);
        }

        pub fn set_has_fallback(&self, value: bool) {
            self.has_fallback.store(value, Ordering::SeqCst);
        }

        pub fn set_vad_tuning(&self, tuning: VadTuning) {
            *self.tuning.lock().unwrap() = Some(tuning);
        }

        pub fn dispatched_text(&self) -> Vec<String> {
            self.dispatched.lock().unwrap().clone()
        }

        pub fn cancelled(&self) -> Vec<String> {
            self.cancelled.lock().unwrap().clone()
        }

        pub fn fallback_dispatches(&self) -> usize {
            self.fallback_dispatches.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl CompanionTurnDispatcher for MockDispatcher {
        async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String> {
            let key = format!("{robot_id}|{companion_id}");
            let mut threads = self.threads.lock().unwrap();
            let id = threads
                .entry(key)
                .or_insert_with(|| format!("conv-{}", threads.len() + 1))
                .clone();
            Ok(id)
        }

        async fn dispatch(
            &self,
            _conversation_id: &str,
            text: &str,
            use_fallback_model: bool,
        ) -> anyhow::Result<tokio::sync::mpsc::Receiver<TurnEvent>> {
            self.dispatched.lock().unwrap().push(text.to_owned());
            if use_fallback_model {
                self.fallback_dispatches.fetch_add(1, Ordering::SeqCst);
            }
            let events = self.scripted.lock().unwrap().pop_front().unwrap_or_else(|| vec![TurnEvent::Done]);
            let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
            tokio::spawn(async move {
                for event in events {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
            Ok(rx)
        }

        async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()> {
            self.cancelled.lock().unwrap().push(conversation_id.to_owned());
            Ok(())
        }

        async fn vad_tuning(&self, _companion_id: &str) -> VadTuning {
            self.tuning.lock().unwrap().unwrap_or_default()
        }

        async fn has_fallback_model(&self, _companion_id: &str) -> bool {
            self.has_fallback.load(Ordering::SeqCst)
        }
    }
}
```

`lib.rs` 加 `pub mod services;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot services`
Expected: PASS — 5 个测试全过

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/
git commit -m "feat(robot): add speech and dispatcher trait seams with mocks"
```

---

### Task 14: 上行管线

**Files:**
- Create: `crates/backend/nomifun-robot/src/pipeline/uplink.rs`
- Modify: `crates/backend/nomifun-robot/src/pipeline/mod.rs`（加 `pub mod uplink;`）

**Interfaces:**
- Consumes: `audio::{OpusStreamDecoder, pcm_to_wav, UPLINK_FRAME_SAMPLES}`、`protocol::{ListeningMode, UPLINK_SAMPLE_RATE}`、`vad::{VadDecision, VadEngine}`
- Produces:
  - `pub enum UplinkOutcome { Continue, Utterance(Vec<u8>) }`（`Utterance` 内是 WAV 字节）
  - `pub struct UplinkPipeline`
  - `pub fn new(vad: Box<dyn VadEngine>) -> anyhow::Result<Self>`
  - `pub fn begin(&mut self, mode: ListeningMode)`
  - `pub fn push_packet(&mut self, packet: &[u8]) -> UplinkOutcome`
  - `pub fn finish(&mut self) -> Option<Vec<u8>>`
  - `pub fn abort(&mut self)`
  - `pub fn is_active(&self) -> bool`
  - `pub const MAX_UTTERANCE_MS: u32 = 60_000;`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/pipeline/uplink.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{OpusStreamEncoder, UPLINK_FRAME_SAMPLES};
    use crate::vad::{EnergyVad, VadTuning};

    /// Encode `ms` of loud audio into 60 ms Opus packets, as the device would.
    fn loud_packets(ms: u32) -> Vec<Vec<u8>> {
        let n = (16_000u64 * ms as u64 / 1000) as usize;
        let pcm: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                ((t * 300.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
            })
            .collect();
        OpusStreamEncoder::new_uplink_for_test().unwrap().encode_frames(&pcm).unwrap()
    }

    fn quiet_packets(ms: u32) -> Vec<Vec<u8>> {
        let n = (16_000u64 * ms as u64 / 1000) as usize;
        OpusStreamEncoder::new_uplink_for_test().unwrap().encode_frames(&vec![0i16; n]).unwrap()
    }

    fn pipeline() -> UplinkPipeline {
        UplinkPipeline::new(Box::new(EnergyVad::new(VadTuning {
            sensitivity: 0.5,
            min_silence_ms: 700,
        })))
        .unwrap()
    }

    fn wav_sample_count(wav: &[u8]) -> usize {
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        data_len / 2
    }

    #[test]
    fn auto_mode_ends_the_utterance_on_trailing_silence() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Auto);
        assert!(p.is_active());

        for packet in loud_packets(300) {
            assert!(matches!(p.push_packet(&packet), UplinkOutcome::Continue));
        }
        let mut wav = None;
        for packet in quiet_packets(900) {
            if let UplinkOutcome::Utterance(bytes) = p.push_packet(&packet) {
                wav = Some(bytes);
                break;
            }
        }
        let wav = wav.expect("700 ms of silence must end the turn");
        assert_eq!(&wav[0..4], b"RIFF");
        assert!(wav_sample_count(&wav) >= 16_000 * 300 / 1000, "the speech is all in there");
        assert!(!p.is_active(), "the pipeline closes itself after emitting");
    }

    #[test]
    fn manual_mode_never_ends_on_its_own_and_finish_emits() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Manual);
        for packet in loud_packets(180) {
            assert!(matches!(p.push_packet(&packet), UplinkOutcome::Continue));
        }
        for packet in quiet_packets(2000) {
            assert!(
                matches!(p.push_packet(&packet), UplinkOutcome::Continue),
                "manual mode waits for `listen stop`, however long the pause"
            );
        }
        let wav = p.finish().expect("finish emits what was buffered");
        assert!(wav_sample_count(&wav) > 0);
        assert!(p.finish().is_none(), "finish drains");
    }

    #[test]
    fn packets_before_begin_are_dropped() {
        let mut p = pipeline();
        for packet in loud_packets(120) {
            assert!(matches!(p.push_packet(&packet), UplinkOutcome::Continue));
        }
        assert!(p.finish().is_none(), "audio outside a listen window is not an utterance");
    }

    #[test]
    fn abort_discards_the_buffer_and_closes() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Auto);
        for packet in loud_packets(180) {
            p.push_packet(&packet);
        }
        p.abort();
        assert!(!p.is_active());
        assert!(p.finish().is_none());
    }

    #[test]
    fn hitting_the_ceiling_force_emits_instead_of_growing_forever() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Manual);
        let mut emitted = None;
        // Feed well past the 60 s ceiling; manual mode would otherwise never end.
        'outer: for _ in 0..70 {
            for packet in loud_packets(1000) {
                if let UplinkOutcome::Utterance(wav) = p.push_packet(&packet) {
                    emitted = Some(wav);
                    break 'outer;
                }
            }
        }
        let wav = emitted.expect("the ceiling must force an emit");
        let seconds = wav_sample_count(&wav) as f64 / 16_000.0;
        assert!((59.0..=62.0).contains(&seconds), "capped near the ceiling, got {seconds:.1}s");
    }

    #[test]
    fn a_second_utterance_works_after_the_first() {
        let mut p = pipeline();
        for round in 0..2 {
            p.begin(crate::protocol::ListeningMode::Auto);
            for packet in loud_packets(200) {
                p.push_packet(&packet);
            }
            let mut got = false;
            for packet in quiet_packets(900) {
                if matches!(p.push_packet(&packet), UplinkOutcome::Utterance(_)) {
                    got = true;
                    break;
                }
            }
            assert!(got, "round {round} must produce an utterance (VAD state was reset)");
        }
    }

    #[test]
    fn a_corrupt_packet_is_skipped_without_killing_the_turn() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Manual);
        assert!(matches!(p.push_packet(&[0xff, 0xff, 0xff, 0xff]), UplinkOutcome::Continue));
        for packet in loud_packets(120) {
            p.push_packet(&packet);
        }
        assert!(p.finish().is_some(), "the good audio still made it through");
        assert_eq!(UPLINK_FRAME_SAMPLES, 960);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot uplink`
Expected: FAIL — `cannot find struct UplinkPipeline in this scope`

- [ ] **Step 3: 写最小实现**

创建 `crates/backend/nomifun-robot/src/pipeline/uplink.rs`（测试模块之前）：

```rust
//! Microphone → transcript-ready WAV.
//!
//! The firmware never tells us a turn is over in `auto`/`realtime` mode — its own
//! VAD only drives an LED — so endpointing happens here. In `manual` mode the
//! device sends `listen stop` and we just hand over what we buffered.
//!
//! A hard ceiling guards against a stuck-open microphone: without it a `manual`
//! session whose `listen stop` never arrives would grow this buffer forever.

use crate::audio::{OpusStreamDecoder, pcm_to_wav};
use crate::protocol::{ListeningMode, UPLINK_SAMPLE_RATE};
use crate::vad::{VadDecision, VadEngine};

/// Longest single utterance we will buffer, in milliseconds.
pub const MAX_UTTERANCE_MS: u32 = 60_000;

fn max_samples() -> usize {
    (UPLINK_SAMPLE_RATE as u64 * MAX_UTTERANCE_MS as u64 / 1000) as usize
}

/// What a packet did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UplinkOutcome {
    /// Keep listening.
    Continue,
    /// The utterance ended; here is the WAV to transcribe.
    Utterance(Vec<u8>),
}

/// Decodes uplink Opus, buffers PCM, and decides when the user stopped talking.
pub struct UplinkPipeline {
    decoder: OpusStreamDecoder,
    vad: Box<dyn VadEngine>,
    pcm: Vec<i16>,
    mode: ListeningMode,
    active: bool,
}

impl UplinkPipeline {
    /// Build with the companion's chosen endpointer.
    pub fn new(vad: Box<dyn VadEngine>) -> anyhow::Result<Self> {
        Ok(Self {
            decoder: OpusStreamDecoder::new_uplink()?,
            vad,
            pcm: Vec::new(),
            mode: ListeningMode::Auto,
            active: false,
        })
    }

    /// Open a listening window (device sent `listen start`).
    pub fn begin(&mut self, mode: ListeningMode) {
        self.mode = mode;
        self.active = true;
        self.pcm.clear();
        self.vad.reset();
    }

    /// Whether a listening window is open.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Feed one uplink Opus packet.
    pub fn push_packet(&mut self, packet: &[u8]) -> UplinkOutcome {
        if !self.active {
            // Wake-word audio arrives before `listen start`; it is not part of a
            // turn and must not be transcribed as one.
            return UplinkOutcome::Continue;
        }
        let frame = match self.decoder.decode(packet) {
            Ok(pcm) => pcm,
            Err(error) => {
                tracing::warn!(%error, "robot: dropping undecodable uplink packet");
                return UplinkOutcome::Continue;
            }
        };
        self.pcm.extend_from_slice(&frame);

        if self.pcm.len() >= max_samples() {
            tracing::warn!(
                seconds = MAX_UTTERANCE_MS / 1000,
                "robot: utterance hit the ceiling, forcing transcription"
            );
            return UplinkOutcome::Utterance(self.take_wav());
        }

        // Manual mode is ended by the device, never by us.
        if matches!(self.mode, ListeningMode::Manual) {
            return UplinkOutcome::Continue;
        }

        match self.vad.push_frame(&frame) {
            VadDecision::EndOfUtterance => UplinkOutcome::Utterance(self.take_wav()),
            VadDecision::Speech | VadDecision::Silence => UplinkOutcome::Continue,
        }
    }

    /// Close the window and emit whatever was buffered (device sent `listen
    /// stop`). Returns `None` if nothing was captured.
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        self.active = false;
        if self.pcm.is_empty() {
            self.pcm.clear();
            return None;
        }
        Some(self.take_wav())
    }

    /// Throw the buffer away (device sent `abort`).
    pub fn abort(&mut self) {
        self.active = false;
        self.pcm.clear();
        self.vad.reset();
    }

    fn take_wav(&mut self) -> Vec<u8> {
        let pcm = std::mem::take(&mut self.pcm);
        self.active = false;
        self.vad.reset();
        pcm_to_wav(&pcm, UPLINK_SAMPLE_RATE)
    }
}
```

`pipeline/mod.rs` 追加 `pub mod uplink;` 与 `pub use uplink::{MAX_UTTERANCE_MS, UplinkOutcome, UplinkPipeline};`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot uplink`
Expected: PASS — 7 个测试全过

若 `a_corrupt_packet_is_skipped_without_killing_the_turn` 失败（某些 libopus 版本对 `[0xff;4]` 不报错而返回噪声帧），把断言改成「不 panic 且后续音频仍能 finish」——即删掉对 `push_packet` 返回值的 `matches!` 断言，只保留最后的 `finish().is_some()`。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/
git commit -m "feat(robot): add uplink pipeline with VAD endpointing"
```

---

### Task 15: 下行管线（节奏控制与打断冲刷）

**Files:**
- Create: `crates/backend/nomifun-robot/src/pipeline/downlink.rs`
- Modify: `crates/backend/nomifun-robot/src/pipeline/mod.rs`（加 `pub mod downlink;`）

**Interfaces:**
- Consumes: `link::Frame`、`audio::{AudioBuffer, OpusStreamEncoder, resample_linear}`、`protocol::{DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS}`、`protocol::binary::encode_binary_v1`
- Produces:
  - `pub struct DownlinkPacer`
  - `pub fn spawn(out: tokio::sync::mpsc::Sender<Frame>) -> (Self, tokio::task::JoinHandle<()>)`
  - `pub fn generation(&self) -> u64`
  - `pub async fn enqueue(&self, generation: u64, packets: Vec<Vec<u8>>)`
  - `pub fn flush(&self) -> u64`
  - `pub fn encode_for_downlink(encoder: &mut OpusStreamEncoder, audio: &AudioBuffer) -> anyhow::Result<Vec<Vec<u8>>>`
  - `pub const PRIME_FRAMES: u64 = 3;`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/pipeline/downlink.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioBuffer, OpusStreamEncoder};
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn packets(n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|i| vec![0xfc, i as u8]).collect()
    }

    async fn drain(rx: &mut mpsc::Receiver<Frame>, want: usize) -> usize {
        let mut got = 0;
        while got < want {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(_)) => got += 1,
                _ => break,
            }
        }
        got
    }

    #[tokio::test(start_paused = true)]
    async fn primes_a_burst_then_holds_a_60ms_cadence() {
        let (out_tx, mut out_rx) = mpsc::channel(256);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);

        let started = tokio::time::Instant::now();
        pacer.enqueue(pacer.generation(), packets(10)).await;
        assert_eq!(drain(&mut out_rx, 10).await, 10, "every frame is delivered");

        let elapsed = started.elapsed();
        // 3 frames prime immediately; the remaining 7 are paced 60 ms apart.
        let expected = Duration::from_millis(((10 - PRIME_FRAMES) * FRAME_DURATION_MS as u64) as u64);
        assert!(
            elapsed >= expected && elapsed < expected + Duration::from_millis(180),
            "expected ~{expected:?} of pacing, got {elapsed:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn frames_go_out_as_bare_binary_with_no_header() {
        let (out_tx, mut out_rx) = mpsc::channel(8);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        pacer.enqueue(pacer.generation(), vec![vec![0xfc, 0x01, 0x02]]).await;

        let frame = out_rx.recv().await.unwrap();
        match frame {
            Frame::Binary(bytes) => assert_eq!(
                bytes.as_ref(),
                &[0xfc, 0x01, 0x02],
                "v1 framing means the Opus packet travels unwrapped"
            ),
            Frame::Text(t) => panic!("audio must be a binary frame, got text: {t}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn flush_drops_everything_still_queued() {
        let (out_tx, mut out_rx) = mpsc::channel(512);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let generation = pacer.generation();

        pacer.enqueue(generation, packets(200)).await;
        let bumped = pacer.flush();
        assert_eq!(bumped, generation + 1, "flush advances the generation");

        // Let the pacer work through its queue; stale frames must be discarded.
        tokio::time::advance(Duration::from_secs(30)).await;
        let mut delivered = 0;
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await {
            delivered += 1;
        }
        assert!(
            delivered <= PRIME_FRAMES as usize + 2,
            "only the already-primed frames may escape a flush, got {delivered}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stale_enqueues_are_ignored_outright() {
        let (out_tx, mut out_rx) = mpsc::channel(64);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let stale = pacer.generation();
        pacer.flush();

        pacer.enqueue(stale, packets(5)).await;
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await.is_err(),
            "frames from a cancelled turn must never reach the device"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cadence_restarts_after_a_flush_so_the_next_turn_primes_again() {
        let (out_tx, mut out_rx) = mpsc::channel(256);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        pacer.enqueue(pacer.generation(), packets(4)).await;
        drain(&mut out_rx, 4).await;

        let generation = pacer.flush();
        let started = tokio::time::Instant::now();
        pacer.enqueue(generation, packets(3)).await;
        assert_eq!(drain(&mut out_rx, 3).await, 3);
        assert!(
            started.elapsed() < Duration::from_millis(30),
            "a fresh generation primes immediately, it does not inherit the old deadline"
        );
    }

    #[test]
    fn encode_for_downlink_resamples_to_24k_and_frames_at_60ms() {
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        // 120 ms at 16 kHz becomes 120 ms at 24 kHz = two 60 ms frames.
        let audio = AudioBuffer { pcm: vec![0i16; 16_000 * 120 / 1000], sample_rate: 16_000 };
        let frames = encode_for_downlink(&mut encoder, &audio).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| !f.is_empty()));
    }

    #[test]
    fn encode_for_downlink_passes_24k_through_untouched() {
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let audio = AudioBuffer { pcm: vec![0i16; 24_000 * 180 / 1000], sample_rate: 24_000 };
        assert_eq!(encode_for_downlink(&mut encoder, &audio).unwrap().len(), 3);
    }

    #[test]
    fn encode_for_downlink_tolerates_empty_audio() {
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let audio = AudioBuffer { pcm: Vec::new(), sample_rate: 24_000 };
        assert!(encode_for_downlink(&mut encoder, &audio).unwrap().is_empty());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot downlink`
Expected: FAIL — `cannot find struct DownlinkPacer in this scope`

- [ ] **Step 3: 写最小实现**

创建 `crates/backend/nomifun-robot/src/pipeline/downlink.rs`（测试模块之前）：

```rust
//! Reply audio → device, at the speed the device can swallow.
//!
//! Two firmware facts shape this whole file:
//!
//! 1. The decode queue holds ~40 packets (≈2.4 s) and **silently drops** what
//!    does not fit. Bursting a whole sentence tears the audio, so frames leave
//!    here on a 60 ms cadence with a small priming burst to cover jitter.
//! 2. `abort` does **not** flush the device's own queue. Cancelling a reply
//!    therefore means dropping our queued frames *immediately* — anything we
//!    still hand over will be played. That is what generations are for: `flush`
//!    bumps the counter and every frame tagged with an older one is discarded.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep_until};

use crate::audio::{AudioBuffer, OpusStreamEncoder, resample_linear};
use crate::link::Frame;
use crate::protocol::binary::encode_binary_v1;
use crate::protocol::{DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS};

/// Frames allowed out back-to-back before pacing engages. Fills the device's
/// jitter buffer so the first syllable is not choppy, while staying far below
/// its ~40-packet ceiling.
pub const PRIME_FRAMES: u64 = 3;

struct PacedFrame {
    generation: u64,
    packet: Vec<u8>,
}

/// Hands Opus packets to the device on a real-time cadence.
pub struct DownlinkPacer {
    tx: mpsc::Sender<PacedFrame>,
    generation: Arc<AtomicU64>,
}

impl DownlinkPacer {
    /// Start the pacing task. `out` is the session's writer channel.
    pub fn spawn(out: mpsc::Sender<Frame>) -> (Self, JoinHandle<()>) {
        // Deep enough to hold a long sentence without blocking the encoder.
        let (tx, mut rx) = mpsc::channel::<PacedFrame>(512);
        let generation = Arc::new(AtomicU64::new(0));
        let task_generation = generation.clone();

        let handle = tokio::spawn(async move {
            let frame_gap = Duration::from_millis(FRAME_DURATION_MS as u64);
            let mut current: Option<(u64, Instant, u64)> = None; // (generation, start, index)

            while let Some(item) = rx.recv().await {
                let live = task_generation.load(Ordering::SeqCst);
                if item.generation != live {
                    // A cancelled turn's audio must never be played.
                    continue;
                }
                let (start, index) = match current {
                    Some((generation, start, index)) if generation == item.generation => (start, index),
                    // New (or resumed) generation: restart the cadence so the
                    // next reply primes instead of inheriting an old deadline.
                    _ => (Instant::now(), 0),
                };
                let deadline = start + frame_gap * (index.saturating_sub(PRIME_FRAMES) as u32);
                sleep_until(deadline).await;

                // Re-check: the turn may have been cancelled while we slept.
                if task_generation.load(Ordering::SeqCst) != item.generation {
                    current = None;
                    continue;
                }
                if out.send(Frame::Binary(encode_binary_v1(&item.packet))).await.is_err() {
                    break;
                }
                current = Some((item.generation, start, index + 1));
            }
        });

        (Self { tx, generation }, handle)
    }

    /// The generation a new reply should be tagged with.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Queue one sentence's packets. Stale generations are dropped here so a
    /// cancelled turn cannot even occupy queue space.
    pub async fn enqueue(&self, generation: u64, packets: Vec<Vec<u8>>) {
        if generation != self.generation() {
            return;
        }
        for packet in packets {
            if generation != self.generation() {
                return;
            }
            if self.tx.send(PacedFrame { generation, packet }).await.is_err() {
                return;
            }
        }
    }

    /// Cancel everything queued and return the new generation.
    pub fn flush(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// Resample to the rate we declared to the device and cut 60 ms Opus frames.
pub fn encode_for_downlink(
    encoder: &mut OpusStreamEncoder,
    audio: &AudioBuffer,
) -> anyhow::Result<Vec<Vec<u8>>> {
    if audio.pcm.is_empty() {
        return Ok(Vec::new());
    }
    let pcm = if audio.sample_rate == DOWNLINK_SAMPLE_RATE {
        std::borrow::Cow::Borrowed(&audio.pcm)
    } else {
        std::borrow::Cow::Owned(resample_linear(&audio.pcm, audio.sample_rate, DOWNLINK_SAMPLE_RATE))
    };
    encoder.encode_frames(&pcm)
}
```

`pipeline/mod.rs` 追加 `pub mod downlink;` 与 `pub use downlink::{DownlinkPacer, PRIME_FRAMES, encode_for_downlink};`。

`protocol/mod.rs` 确认 `pub mod binary;` 已导出（Task 4 已加）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot downlink`
Expected: PASS — 8 个测试全过

`frame_gap * (index...) as u32` 要求 `Duration * u32`；若类型不匹配改用 `frame_gap.checked_mul(n).unwrap_or(frame_gap)`。`#[tokio::test(start_paused = true)]` 需要 tokio 的 `test-util` feature——若报 `start_paused` 未知，在 `[dev-dependencies]` 加 `tokio = { workspace = true, features = ["test-util", "macros", "rt-multi-thread"] }`。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml
git commit -m "feat(robot): add downlink pacer with generation-based flush"
```

---

### Task 16: 会话对话循环（上下行接线 + 备用模型降级）

**Files:**
- Modify: `crates/backend/nomifun-robot/src/session.rs`（`SessionDeps` 加两个字段；补全 `run_session` 的 Listen/Binary/Abort 分支；新增 `drive_turn`）

**Interfaces:**
- Consumes: `services::{CompanionTurnDispatcher, SpeechContext, SpeechServices, TurnEvent}`、`pipeline::{DownlinkPacer, SentenceSplitter, UplinkOutcome, UplinkPipeline, encode_for_downlink, strip_emotion}`、`audio::OpusStreamEncoder`、`vad::build_engine`、`protocol::ListenState`
- Produces:
  - `SessionDeps` 新增 `pub speech: std::sync::Arc<dyn SpeechServices>`、`pub dispatcher: std::sync::Arc<dyn CompanionTurnDispatcher>`
  - `pub struct TurnOutcome { pub failed: Option<TurnFailure>, pub used_fallback: bool }`
  - `pub struct TurnFailure { pub message: String, pub provider_fault: bool }`
  - `pub const MAX_CONSECUTIVE_TTS_FAILURES: usize = 2;`

- [ ] **Step 1: 写失败测试**

`session.rs` 的测试模块内，把 `harness` 改为同时构造 mock 服务，并追加对话循环测试。先替换 `harness` 的返回签名与构造（把 `SessionDeps { registry, status }` 改为带四个字段），再追加：

```rust
    use crate::services::mock::{MockDispatcher, MockSpeech};
    use crate::services::TurnEvent;

    /// Encode `ms` of loud 16 kHz audio into 60 ms uplink packets.
    fn uplink_packets(ms: u32, loud: bool) -> Vec<Vec<u8>> {
        let n = (16_000u64 * ms as u64 / 1000) as usize;
        let pcm: Vec<i16> = (0..n)
            .map(|i| {
                if !loud {
                    return 0;
                }
                let t = i as f32 / 16_000.0;
                ((t * 300.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
            })
            .collect();
        crate::audio::OpusStreamEncoder::new_uplink_for_test()
            .unwrap()
            .encode_frames(&pcm)
            .unwrap()
    }

    async fn send_audio(tx: &mpsc::Sender<Frame>, ms: u32, loud: bool) {
        for packet in uplink_packets(ms, loud) {
            tx.send(Frame::Binary(bytes::Bytes::from(packet))).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_full_turn_produces_stt_emotion_sentence_audio_and_stop() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("今天天气怎么样");
        dispatcher.script_turn(vec![
            TurnEvent::Text("[emotion:happy] 晴朗得很。".into()),
            TurnEvent::Done,
        ]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into()))
            .await
            .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await; // trailing silence ends the utterance

        // Give the turn time to run, then close the link.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        drop(tx);
        task.await.unwrap();

        let sent = texts(&written);
        let types: Vec<String> = sent
            .iter()
            .map(|m| {
                let t = m["type"].as_str().unwrap_or_default();
                match (t, m["state"].as_str()) {
                    ("tts", Some(state)) => format!("tts:{state}"),
                    _ => t.to_owned(),
                }
            })
            .collect();

        assert!(types.contains(&"stt".to_owned()), "the transcript is shown on screen: {types:?}");
        assert!(types.contains(&"llm".to_owned()), "the emotion marker drives the face: {types:?}");
        assert!(types.contains(&"tts:start".to_owned()), "{types:?}");
        assert!(types.contains(&"tts:sentence_start".to_owned()), "{types:?}");
        assert!(types.contains(&"tts:stop".to_owned()), "{types:?}");

        let stt = sent.iter().find(|m| m["type"] == "stt").unwrap();
        assert_eq!(stt["text"], "今天天气怎么样");
        let llm = sent.iter().find(|m| m["type"] == "llm").unwrap();
        assert_eq!(llm["emotion"], "happy");
        let sentence = sent
            .iter()
            .find(|m| m["type"] == "tts" && m["state"] == "sentence_start")
            .unwrap();
        assert_eq!(sentence["text"], "晴朗得很。", "the emotion marker is stripped before display");

        let audio_frames = written.lock().unwrap().iter().filter(|f| matches!(f, Frame::Binary(_))).count();
        assert!(audio_frames > 0, "synthesised audio reached the device");
        assert_eq!(dispatcher.dispatched_text(), vec!["今天天气怎么样".to_owned()]);
    }

    #[tokio::test]
    async fn empty_transcript_idles_the_device_without_bothering_the_model() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let dispatcher = deps.dispatcher_mock();
        // MockSpeech returns "" by default.

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"start","mode":"manual"}"#.into()))
            .await
            .unwrap();
        send_audio(&tx, 200, true).await;
        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"stop"}"#.into()))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        drop(tx);
        task.await.unwrap();

        assert!(dispatcher.dispatched_text().is_empty(), "no model turn for silence");
        let sent = texts(&written);
        let states: Vec<&str> = sent
            .iter()
            .filter(|m| m["type"] == "tts")
            .filter_map(|m| m["state"].as_str())
            .collect();
        assert_eq!(states, vec!["start", "stop"], "an empty round returns the device to listening");
    }

    #[tokio::test]
    async fn abort_cancels_the_turn_and_sends_tts_stop() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("讲个很长的故事");
        // A long reply so there is something in flight to cancel.
        dispatcher.script_turn(
            (0..40)
                .map(|i| TurnEvent::Text(format!("第{i}句话。")))
                .chain(std::iter::once(TurnEvent::Done))
                .collect(),
        );

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into()))
            .await
            .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        tx.send(Frame::Text(r#"{"session_id":"s","type":"abort","reason":"wake_word_detected"}"#.into()))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let frames_at_abort = written.lock().unwrap().len();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let frames_later = written.lock().unwrap().len();
        drop(tx);
        task.await.unwrap();

        assert_eq!(
            frames_at_abort, frames_later,
            "not one frame may be written after abort — the device does not flush its own queue"
        );
        assert_eq!(dispatcher.cancelled().len(), 1, "the platform turn is cancelled too");
        let sent = texts(&written);
        assert!(
            sent.iter().any(|m| m["type"] == "tts" && m["state"] == "stop"),
            "abort must be acknowledged with tts stop"
        );
    }

    #[tokio::test]
    async fn a_provider_failure_retries_once_on_the_fallback_model() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("你好");
        dispatcher.set_has_fallback(true);
        dispatcher.script_turn(vec![TurnEvent::Failed {
            message: "upstream 503".into(),
            provider_fault: true,
        }]);
        dispatcher.script_turn(vec![TurnEvent::Text("我在。".into()), TurnEvent::Done]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into()))
            .await
            .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        drop(tx);
        task.await.unwrap();

        assert_eq!(dispatcher.fallback_dispatches(), 1, "exactly one fallback retry");
        assert_eq!(dispatcher.dispatched_text().len(), 2, "the same text, twice");
        let sent = texts(&written);
        assert!(
            sent.iter().any(|m| m["type"] == "tts" && m["state"] == "sentence_start" && m["text"] == "我在。"),
            "the fallback reply reaches the device"
        );
    }

    #[tokio::test]
    async fn a_failure_with_no_fallback_reports_sadly_and_stops() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("你好");
        dispatcher.set_has_fallback(false);
        dispatcher.script_turn(vec![TurnEvent::Failed {
            message: "upstream 503".into(),
            provider_fault: true,
        }]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into()))
            .await
            .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drop(tx);
        task.await.unwrap();

        assert_eq!(dispatcher.fallback_dispatches(), 0);
        let sent = texts(&written);
        assert!(
            sent.iter().any(|m| m["type"] == "llm" && m["emotion"] == "sad"),
            "the robot looks sad about it"
        );
        assert!(
            sent.iter().any(|m| m["type"] == "tts" && m["state"] == "stop"),
            "the device must not be left stuck in speaking"
        );
    }

    #[tokio::test]
    async fn status_walks_idle_listening_speaking_then_offline() {
        let (deps, link, tx, _written, _dir) = harness(true).await;
        let status = deps.deps.status.clone();
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("嗨");
        dispatcher.script_turn(vec![TurnEvent::Text("嗨。".into()), TurnEvent::Done]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(status.snapshot()[0].phase, "idle");

        tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into()))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(status.snapshot().await[0].phase, "listening");

        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drop(tx);
        task.await.unwrap();
        assert_eq!(status.snapshot().await[0].phase, "offline");
    }
```

`harness` 改造为返回一个持有 mock 句柄的小结构（替换 Task 8 版本）：

```rust
    struct Harness {
        deps: SessionDeps,
        speech: Arc<MockSpeech>,
        dispatcher: Arc<MockDispatcher>,
    }

    impl Harness {
        fn speech_mock(&self) -> Arc<MockSpeech> {
            self.speech.clone()
        }
        fn dispatcher_mock(&self) -> Arc<MockDispatcher> {
            self.dispatcher.clone()
        }
    }
```

并在 `harness()` 里构造 `let speech = Arc::new(MockSpeech::new()); let dispatcher = Arc::new(MockDispatcher::new());`，`SessionDeps { registry, status, speech: speech.clone(), dispatcher: dispatcher.clone() }`，返回 `(Harness { deps, speech, dispatcher }, link, tx, written, dir)`。Task 8 的五个测试相应把 `deps` 改为 `deps.deps.clone()`（`status` 那条改为 `deps.deps.status.clone()`）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot session`
Expected: FAIL — `SessionDeps has no field named 'speech'`（以及 `no method named speech_mock`）

- [ ] **Step 3: 写最小实现**

`session.rs` 顶部的 `use` 追加：

```rust
use crate::audio::OpusStreamEncoder;
use crate::pipeline::{
    DownlinkPacer, SentenceSplitter, UplinkOutcome, UplinkPipeline, encode_for_downlink,
    strip_emotion,
};
use crate::protocol::ListenState;
use crate::services::{CompanionTurnDispatcher, SpeechContext, SpeechServices, TurnEvent};
use crate::vad::build_engine;
```

`SessionDeps` 改为：

```rust
/// Everything a session actor needs from the host.
#[derive(Clone)]
pub struct SessionDeps {
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub speech: Arc<dyn SpeechServices>,
    pub dispatcher: Arc<dyn CompanionTurnDispatcher>,
}
```

`Writer` 加 `Clone`（turn 驱动任务需要一份）：给 `struct Writer` 标 `#[derive(Clone)]`。

追加 turn 相关类型与驱动：

```rust
/// Consecutive TTS failures tolerated before the reply is abandoned. One bad
/// sentence is survivable (it is still on screen); a run of them means the
/// provider is down and the device should be released.
pub const MAX_CONSECUTIVE_TTS_FAILURES: usize = 2;

/// Why a turn ended badly.
#[derive(Debug, Clone)]
pub struct TurnFailure {
    pub message: String,
    pub provider_fault: bool,
}

/// How a turn ended.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub failed: Option<TurnFailure>,
    pub used_fallback: bool,
}

/// Stream a reply to the device: split into sentences, drive the face, speak.
async fn drive_turn(
    mut events: mpsc::Receiver<TurnEvent>,
    ctx: SpeechContext,
    speech: Arc<dyn SpeechServices>,
    pacer: Arc<DownlinkPacer>,
    writer: Writer,
    session_id: String,
    generation: u64,
    used_fallback: bool,
) -> TurnOutcome {
    let mut splitter = SentenceSplitter::default();
    let mut encoder = match OpusStreamEncoder::new_downlink() {
        Ok(e) => e,
        Err(error) => {
            return TurnOutcome {
                failed: Some(TurnFailure { message: error.to_string(), provider_fault: false }),
                used_fallback,
            };
        }
    };
    let mut speaking = false;
    let mut tts_failures = 0usize;
    let mut failure: Option<TurnFailure> = None;

    // One sentence: face, screen, then audio.
    macro_rules! speak {
        ($sentence:expr) => {{
            let (emotion, text) = strip_emotion(&$sentence);
            if text.trim().is_empty() {
                // A marker-only chunk still sets the face; there is nothing to say.
                if let Some(emotion) = emotion {
                    writer
                        .send_json(&ServerMessage::Llm {
                            session_id: session_id.clone(),
                            emotion: emotion.to_owned(),
                        })
                        .await;
                }
            } else {
                if !speaking {
                    writer.send_json(&ServerMessage::TtsStart { session_id: session_id.clone() }).await;
                    speaking = true;
                }
                if let Some(emotion) = emotion {
                    writer
                        .send_json(&ServerMessage::Llm {
                            session_id: session_id.clone(),
                            emotion: emotion.to_owned(),
                        })
                        .await;
                }
                writer
                    .send_json(&ServerMessage::TtsSentence {
                        session_id: session_id.clone(),
                        text: text.clone(),
                    })
                    .await;
                match speech.synthesize(&ctx, &text).await {
                    Ok(audio) => {
                        tts_failures = 0;
                        match encode_for_downlink(&mut encoder, &audio) {
                            Ok(frames) => pacer.enqueue(generation, frames).await,
                            Err(error) => {
                                tracing::warn!(%error, "robot: opus encode failed for a sentence");
                            }
                        }
                    }
                    Err(error) => {
                        tts_failures += 1;
                        tracing::warn!(%error, tts_failures, "robot: TTS failed, sentence stays on screen only");
                        if tts_failures >= MAX_CONSECUTIVE_TTS_FAILURES {
                            failure = Some(TurnFailure {
                                message: format!("TTS failed {tts_failures} times: {error}"),
                                provider_fault: true,
                            });
                        }
                    }
                }
            }
        }};
    }

    while let Some(event) = events.recv().await {
        match event {
            TurnEvent::Text(chunk) => {
                for sentence in splitter.push(&chunk) {
                    speak!(sentence);
                    if failure.is_some() {
                        break;
                    }
                }
            }
            TurnEvent::Done => {
                if failure.is_none()
                    && let Some(tail) = splitter.flush()
                {
                    speak!(tail);
                }
                break;
            }
            TurnEvent::Failed { message, provider_fault } => {
                failure = Some(TurnFailure { message, provider_fault });
                break;
            }
        }
        if failure.is_some() {
            break;
        }
    }

    if speaking {
        writer.send_json(&ServerMessage::TtsStop { session_id: session_id.clone() }).await;
    }
    TurnOutcome { failed: failure, used_fallback }
}

/// Tell the user something went wrong without stranding the device in `speaking`.
async fn report_turn_failure(writer: &Writer, session_id: &str) {
    writer
        .send_json(&ServerMessage::Llm {
            session_id: session_id.to_owned(),
            emotion: "sad".to_owned(),
        })
        .await;
    writer.send_json(&ServerMessage::TtsStart { session_id: session_id.to_owned() }).await;
    writer
        .send_json(&ServerMessage::TtsSentence {
            session_id: session_id.to_owned(),
            text: "我这边出了点问题，稍后再试试。".to_owned(),
        })
        .await;
    writer.send_json(&ServerMessage::TtsStop { session_id: session_id.to_owned() }).await;
}
```

`run_session` 主体改造：在 hello 分支成功后补建管线与 turn 状态，并补全其余分支。hello 分支末尾追加：

```rust
                                let tuning = deps.dispatcher.vad_tuning(&bound).await;
                                let engine = build_engine("silero", tuning);
                                tracing::info!(%robot_id, vad = engine.name(), "robot: endpointer ready");
                                uplink = UplinkPipeline::new(engine).ok();
                                conversation_id = deps
                                    .dispatcher
                                    .ensure_thread(&robot_id, &bound)
                                    .await
                                    .inspect_err(|error| {
                                        tracing::error!(%robot_id, %error, "robot: could not open a companion thread");
                                    })
                                    .ok();
```

`run_session` 的局部状态（在 `let mut session_id` 附近）追加：

```rust
    let mut uplink: Option<UplinkPipeline> = None;
    let mut conversation_id: Option<String> = None;
    let (pacer, pacer_task) = DownlinkPacer::spawn(writer.tx.clone());
    let pacer = Arc::new(pacer);
    let (turn_tx, mut turn_rx) = mpsc::channel::<TurnOutcome>(4);
    let mut turn_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut pending_text: Option<String> = None;
```

`Listen` 分支：

```rust
                            DeviceMessage::Listen { state, mode, .. } => {
                                let Some(pipeline) = uplink.as_mut() else { continue };
                                match state {
                                    ListenState::Start => {
                                        pipeline.begin(mode.unwrap_or(crate::protocol::ListeningMode::Auto));
                                        if let Some(bound) = &companion_id {
                                            deps.status
                                                .publish(&robot_id, Some(bound), RobotPhase::Listening, now_ms())
                                                .await;
                                        }
                                    }
                                    ListenState::Stop => {
                                        if let Some(wav) = pipeline.finish() {
                                            utterance = Some(wav);
                                        }
                                    }
                                    // The wake word itself is not part of the turn.
                                    ListenState::Detect => {}
                                }
                            }
```

`Frame::Binary` 分支（已握手那条）替换为：

```rust
                    Frame::Binary(bytes) => {
                        let Some(pipeline) = uplink.as_mut() else { continue };
                        if let UplinkOutcome::Utterance(wav) = pipeline.push_packet(&bytes) {
                            utterance = Some(wav);
                        }
                    }
```

`Abort` 分支：

```rust
                            DeviceMessage::Abort { reason } => {
                                tracing::info!(%robot_id, ?reason, "robot: abort");
                                // Order matters: stop our own queue first, because
                                // the device will play anything we hand over.
                                pacer.flush();
                                if let Some(task) = turn_task.take() {
                                    task.abort();
                                }
                                if let Some(pipeline) = uplink.as_mut() {
                                    pipeline.abort();
                                }
                                if let Some(conversation) = &conversation_id
                                    && let Err(error) = deps.dispatcher.cancel(conversation).await
                                {
                                    tracing::warn!(%robot_id, %error, "robot: turn cancel failed");
                                }
                                if let Some(sid) = &session_id {
                                    writer.send_json(&ServerMessage::TtsStop { session_id: sid.clone() }).await;
                                }
                                if let Some(bound) = &companion_id {
                                    deps.status.publish(&robot_id, Some(bound), RobotPhase::Idle, now_ms()).await;
                                }
                            }
```

在 `select!` 里增加 turn 完成分支：

```rust
            outcome = turn_rx.recv() => {
                let Some(outcome) = outcome else { continue };
                turn_task = None;
                let Some(failure) = outcome.failed else {
                    if let Some(bound) = &companion_id {
                        deps.status.publish(&robot_id, Some(bound), RobotPhase::Idle, now_ms()).await;
                    }
                    pending_text = None;
                    continue;
                };
                tracing::warn!(%robot_id, message = %failure.message, "robot: turn failed");
                let retryable = failure.provider_fault
                    && !outcome.used_fallback
                    && companion_id
                        .as_deref()
                        .is_some_and(|_| true);
                let has_fallback = match &companion_id {
                    Some(bound) => deps.dispatcher.has_fallback_model(bound).await,
                    None => false,
                };
                if retryable && has_fallback {
                    if let (Some(text), Some(conversation), Some(sid), Some(bound)) = (
                        pending_text.clone(),
                        conversation_id.clone(),
                        session_id.clone(),
                        companion_id.clone(),
                    ) {
                        tracing::info!(%robot_id, "robot: retrying the turn on the fallback model");
                        start_turn(
                            &deps, &pacer, &writer, &turn_tx, &mut turn_task,
                            &robot_id, &bound, &conversation, &sid, &text, true,
                        )
                        .await;
                        continue;
                    }
                }
                if let Some(sid) = &session_id {
                    report_turn_failure(&writer, sid).await;
                }
                pending_text = None;
                if let Some(bound) = &companion_id {
                    deps.status.publish(&robot_id, Some(bound), RobotPhase::Idle, now_ms()).await;
                }
            }
```

在 `select!` 之后（同一 loop 迭代末尾）处理攒好的 utterance。为此在 loop 顶部声明 `let mut utterance: Option<Vec<u8>> = None;`，并在 loop 末尾加：

```rust
        if let Some(wav) = utterance.take() {
            let (Some(sid), Some(bound), Some(conversation)) =
                (session_id.clone(), companion_id.clone(), conversation_id.clone())
            else {
                continue;
            };
            let ctx = SpeechContext { robot_id: robot_id.clone(), companion_id: bound.clone() };
            let transcript = match deps.speech.transcribe(&ctx, wav).await {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(%robot_id, %error, "robot: ASR failed");
                    String::new()
                }
            };
            if transcript.trim().is_empty() {
                // An empty round: hand the device straight back to listening
                // without spending a model turn on noise.
                writer.send_json(&ServerMessage::TtsStart { session_id: sid.clone() }).await;
                writer.send_json(&ServerMessage::TtsStop { session_id: sid.clone() }).await;
                deps.status.publish(&robot_id, Some(&bound), RobotPhase::Idle, now_ms()).await;
                continue;
            }
            writer
                .send_json(&ServerMessage::Stt { session_id: sid.clone(), text: transcript.clone() })
                .await;
            deps.status.publish(&robot_id, Some(&bound), RobotPhase::Speaking, now_ms()).await;
            pending_text = Some(transcript.clone());
            start_turn(
                &deps, &pacer, &writer, &turn_tx, &mut turn_task,
                &robot_id, &bound, &conversation, &sid, &transcript, false,
            )
            .await;
        }
```

清理段（`mark_offline` 之前）追加：

```rust
    if let Some(task) = turn_task.take() {
        task.abort();
    }
    pacer.flush();
```

并在函数末尾 `let _ = writer_task.await;` 之后加 `pacer_task.abort();`。

新增 `start_turn` 辅助函数：

```rust
/// Kick off one turn and remember its task so `abort` can kill it.
#[allow(clippy::too_many_arguments)]
async fn start_turn(
    deps: &SessionDeps,
    pacer: &Arc<DownlinkPacer>,
    writer: &Writer,
    turn_tx: &mpsc::Sender<TurnOutcome>,
    turn_task: &mut Option<tokio::task::JoinHandle<()>>,
    robot_id: &str,
    companion_id: &str,
    conversation_id: &str,
    session_id: &str,
    text: &str,
    use_fallback: bool,
) {
    let events = match deps.dispatcher.dispatch(conversation_id, text, use_fallback).await {
        Ok(rx) => rx,
        Err(error) => {
            tracing::error!(%robot_id, %error, "robot: dispatch failed");
            let _ = turn_tx
                .send(TurnOutcome {
                    failed: Some(TurnFailure { message: error.to_string(), provider_fault: true }),
                    used_fallback: use_fallback,
                })
                .await;
            return;
        }
    };
    let ctx = SpeechContext { robot_id: robot_id.to_owned(), companion_id: companion_id.to_owned() };
    let generation = pacer.generation();
    let (speech, pacer, writer, session_id, turn_tx) = (
        deps.speech.clone(),
        pacer.clone(),
        writer.clone(),
        session_id.to_owned(),
        turn_tx.clone(),
    );
    *turn_task = Some(tokio::spawn(async move {
        let outcome =
            drive_turn(events, ctx, speech, pacer, writer, session_id, generation, use_fallback).await;
        let _ = turn_tx.send(outcome).await;
    }));
}
```

`Writer` 的 `tx` 字段改为 `pub(crate) tx`（pacer 需要克隆它）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot session`
Expected: PASS — 11 个测试全过（Task 8 的 5 个 + 本任务 6 个）

`macro_rules! speak!` 捕获了外层可变量，若借用检查报错，把它改写为一个接收 `&mut` 参数的内部 `async fn speak_sentence(...)`，参数为 `(&Writer, &Arc<dyn SpeechServices>, &SpeechContext, &Arc<DownlinkPacer>, &mut OpusStreamEncoder, &mut bool /*speaking*/, &mut usize /*failures*/, &str /*session_id*/, u64 /*generation*/, String /*sentence*/) -> Option<TurnFailure>`，语义完全一致。`status.snapshot()[0]` 处漏了 `.await` 的那一行按编译器提示补上。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/session.rs
git commit -m "feat(robot): wire uplink, dispatch and downlink into the session loop"
```

---

### Task 17: 设备 MCP 客户端（信封传输 + 分页 + 双错误路径）

**Files:**
- Create: `crates/backend/nomifun-robot/src/mcp_bridge.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod mcp_bridge;`）

**Interfaces:**
- Consumes: `link::Frame`、`protocol::{ServerMessage, serialize_server_message}`
- Produces:
  - `pub struct RobotMcpClient`，`pub fn new(out: tokio::sync::mpsc::Sender<Frame>, session_id: String) -> Self`
  - `pub async fn handle_incoming(&self, payload: serde_json::Value)`（会话收到 `type:"mcp"` 时调用）
  - `pub async fn initialize(&self, vision_url: Option<&str>, vision_token: &str) -> anyhow::Result<()>`
  - `pub async fn list_tools(&self) -> anyhow::Result<Vec<RobotToolDescriptor>>`（跨 `nextCursor` 合并）
  - `pub async fn call_tool(&self, device_name: &str, args: serde_json::Value) -> Result<String, ToolCallError>`
  - `pub struct RobotToolDescriptor { pub device_name: String, pub exposed_name: String, pub description: String, pub input_schema: serde_json::Value }`
  - `pub enum ToolCallError { Rejected(String), Failed(String), Timeout, Offline }`
  - `pub fn exposed_tool_name(device_name: &str) -> String`
  - `pub const TOOL_CALL_TIMEOUT_SECS: u64 = 30;`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/mcp_bridge.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Spawn a client plus a fake device that answers requests with `responder`.
    fn device<F>(responder: F) -> Arc<RobotMcpClient>
    where
        F: Fn(&str, u64, &serde_json::Value) -> Option<serde_json::Value> + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::channel::<Frame>(32);
        let client = Arc::new(RobotMcpClient::new(out_tx, "sess-1".to_owned()));
        let echo = client.clone();
        tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                let Frame::Text(raw) = frame else { continue };
                let envelope: serde_json::Value = serde_json::from_str(&raw).unwrap();
                assert_eq!(envelope["type"], "mcp", "device MCP travels in an mcp envelope");
                assert_eq!(envelope["session_id"], "sess-1");
                let payload = &envelope["payload"];
                let method = payload["method"].as_str().unwrap_or_default();
                let id = payload["id"].as_u64().expect("the firmware drops non-numeric ids");
                if let Some(reply) = responder(method, id, &payload["params"]) {
                    echo.handle_incoming(reply).await;
                }
            }
        });
        client
    }

    #[test]
    fn exposed_names_are_model_friendly() {
        assert_eq!(exposed_tool_name("self.gimbal.look"), "robot_gimbal_look");
        assert_eq!(exposed_tool_name("self.audio_speaker.set_volume"), "robot_audio_speaker_set_volume");
        assert_eq!(exposed_tool_name("self.camera.take_photo"), "robot_camera_take_photo");
        assert_eq!(exposed_tool_name("weird name"), "robot_weird_name");
    }

    #[tokio::test]
    async fn initialize_delivers_the_vision_url_and_token() {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let recorder = seen.clone();
        let client = device(move |method, id, params| {
            if method == "initialize" {
                *recorder.lock().unwrap() = Some(params.clone());
                return Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "ESP32-S3N16R8-EMOJI", "version": "1.9.0" }
                    }
                }));
            }
            None
        });

        client.initialize(Some("http://192.168.1.20:25808/robot/vision/explain"), "tok-1").await.unwrap();
        let params = seen.lock().unwrap().clone().expect("initialize was sent");
        assert_eq!(
            params["capabilities"]["vision"]["url"],
            "http://192.168.1.20:25808/robot/vision/explain",
            "MCP initialize is the ONLY channel that configures the vision URL"
        );
        assert_eq!(params["capabilities"]["vision"]["token"], "tok-1");
    }

    #[tokio::test]
    async fn initialize_omits_vision_when_there_is_no_reachable_url() {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let recorder = seen.clone();
        let client = device(move |method, id, params| {
            if method == "initialize" {
                *recorder.lock().unwrap() = Some(params.clone());
                return Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} }));
            }
            None
        });
        client.initialize(None, "tok-1").await.unwrap();
        let params = seen.lock().unwrap().clone().unwrap();
        assert!(params["capabilities"].get("vision").is_none());
    }

    #[tokio::test]
    async fn list_tools_follows_next_cursor_to_the_end() {
        let client = device(|method, id, params| {
            if method != "tools/list" {
                return None;
            }
            let cursor = params.get("cursor").and_then(|c| c.as_str()).unwrap_or_default();
            let page = if cursor.is_empty() {
                json!({
                    "tools": [
                        { "name": "self.get_device_status", "description": "status", "inputSchema": { "type": "object", "properties": {} } },
                        { "name": "self.audio_speaker.set_volume", "description": "volume", "inputSchema": { "type": "object", "properties": { "volume": { "type": "integer" } } } }
                    ],
                    "nextCursor": "self.gimbal.look"
                })
            } else {
                json!({
                    "tools": [
                        { "name": "self.gimbal.look", "description": "turn the head", "inputSchema": { "type": "object", "properties": { "direction": { "type": "string" } } } }
                    ]
                })
            };
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": page }))
        });

        let tools = client.list_tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.exposed_name.as_str()).collect();
        assert_eq!(
            names,
            vec!["robot_get_device_status", "robot_audio_speaker_set_volume", "robot_gimbal_look"],
            "the 8000-byte page limit must not truncate the toolset"
        );
        let gimbal = tools.iter().find(|t| t.exposed_name == "robot_gimbal_look").unwrap();
        assert_eq!(gimbal.device_name, "self.gimbal.look");
        assert_eq!(gimbal.input_schema["properties"]["direction"]["type"], "string");
    }

    #[tokio::test]
    async fn call_tool_returns_the_text_content() {
        let client = device(|method, id, params| {
            if method != "tools/call" {
                return None;
            }
            assert_eq!(params["name"], "self.gimbal.set");
            assert_eq!(params["arguments"]["pan"], 100);
            assert!(params["stackSize"].as_u64().unwrap() >= 6144, "give the tool thread room");
            Some(json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "content": [{ "type": "text", "text": "{\"pan\":100,\"tilt\":90}" }], "isError": false }
            }))
        });

        let text = client.call_tool("self.gimbal.set", json!({ "pan": 100, "tilt": 90 })).await.unwrap();
        assert_eq!(text, "{\"pan\":100,\"tilt\":90}");
    }

    #[tokio::test]
    async fn a_firmware_error_without_a_code_field_is_still_understood() {
        // The firmware's error objects have NO `code` — a strict JSON-RPC
        // deserializer would fail here, so ours must tolerate it.
        let client = device(|method, id, _params| {
            if method != "tools/call" {
                return None;
            }
            Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "message": "Value exceeds maximum allowed: 130" }
            }))
        });

        let error = client.call_tool("self.gimbal.set", json!({ "pan": 999 })).await.unwrap_err();
        match error {
            ToolCallError::Rejected(message) => assert!(message.contains("exceeds maximum")),
            other => panic!("out-of-range parameters are a rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_is_error_result_is_a_failure_not_a_rejection() {
        let client = device(|method, id, _params| {
            if method != "tools/call" {
                return None;
            }
            Some(json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "content": [{ "type": "text", "text": "Failed to capture photo" }], "isError": true }
            }))
        });

        let error = client.call_tool("self.camera.take_photo", json!({ "question": "?" })).await.unwrap_err();
        match error {
            ToolCallError::Failed(message) => assert!(message.contains("Failed to capture")),
            other => panic!("a runtime tool failure is Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn camera_tools_get_a_much_larger_stack() {
        let client = device(|method, id, params| {
            if method != "tools/call" {
                return None;
            }
            assert!(
                params["stackSize"].as_u64().unwrap() >= 32_768,
                "JPEG encode plus TLS does not fit the 6144-byte default"
            );
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [], "isError": false } }))
        });
        let _ = client.call_tool("self.camera.take_photo", json!({ "question": "?" })).await;
    }

    #[tokio::test]
    async fn a_silent_device_times_out_rather_than_hanging_forever() {
        let client = device(|_method, _id, _params| None);
        tokio::time::pause();
        let call = tokio::spawn({
            let client = client.clone();
            async move { client.call_tool("self.gimbal.center", json!({})).await }
        });
        tokio::time::advance(std::time::Duration::from_secs(TOOL_CALL_TIMEOUT_SECS + 1)).await;
        assert!(matches!(call.await.unwrap(), Err(ToolCallError::Timeout)));
    }

    #[tokio::test]
    async fn a_closed_link_reports_offline() {
        let (out_tx, out_rx) = mpsc::channel::<Frame>(1);
        drop(out_rx);
        let client = RobotMcpClient::new(out_tx, "sess-1".to_owned());
        assert!(matches!(
            client.call_tool("self.gimbal.center", json!({})).await,
            Err(ToolCallError::Offline)
        ));
    }

    #[tokio::test]
    async fn a_notification_from_the_device_is_ignored_safely() {
        let client = device(|_method, _id, _params| None);
        // No id, and a notifications/* method: must not panic or poison state.
        client.handle_incoming(json!({ "jsonrpc": "2.0", "method": "notifications/ready" })).await;
        client.handle_incoming(json!({ "jsonrpc": "2.0", "id": 9999, "result": {} })).await;
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot mcp_bridge`
Expected: FAIL — `cannot find struct RobotMcpClient in this scope`

- [ ] **Step 3: 写最小实现**

创建 `crates/backend/nomifun-robot/src/mcp_bridge.rs`（测试模块之前）：

```rust
//! The device is an MCP **server**; we are its client.
//!
//! Three firmware quirks shape this file, and none of them are optional:
//!
//! 1. Request `id` **must be a number**. A string id makes the firmware drop the
//!    message with no reply at all, so every call would hang.
//! 2. Methods starting with `notifications` are ignored outright — nothing may
//!    depend on a notification arriving.
//! 3. `tools/list` truncates at 8000 bytes and pages via `nextCursor = <tool
//!    name>`, so one request is not enough to see the whole toolset.
//!
//! Its error objects also omit JSON-RPC's mandatory `code`, which is why this
//! module carries its own tolerant response type instead of reusing
//! `nomi_mcp::protocol::JsonRpcResponse` (whose `code` is required and would
//! fail to deserialize).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::{Duration, timeout};

use crate::link::Frame;
use crate::protocol::{ServerMessage, serialize_server_message};

/// How long to wait for a tool result. Matches the firmware's own 30 s HTTP
/// ceiling for `take_photo`, the slowest tool it has.
pub const TOOL_CALL_TIMEOUT_SECS: u64 = 30;
/// Default tool-thread stack the firmware allocates when we say nothing.
const DEFAULT_STACK: u64 = 6_144;
/// Stack for tools that encode JPEG and open TLS.
const CAMERA_STACK: u64 = 32_768;

/// A device tool, with both its on-device name and the name models see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotToolDescriptor {
    pub device_name: String,
    pub exposed_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Why a tool call did not produce a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallError {
    /// The device refused the request (bad or out-of-range arguments). Arrives
    /// as a JSON-RPC `error`, before the tool ever runs.
    Rejected(String),
    /// The tool ran and failed (`isError: true`).
    Failed(String),
    /// No reply within [`TOOL_CALL_TIMEOUT_SECS`].
    Timeout,
    /// The link is gone.
    Offline,
}

impl std::fmt::Display for ToolCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(m) => write!(f, "device rejected the call: {m}"),
            Self::Failed(m) => write!(f, "tool failed: {m}"),
            Self::Timeout => write!(f, "device did not answer in {TOOL_CALL_TIMEOUT_SECS}s"),
            Self::Offline => write!(f, "robot is offline"),
        }
    }
}

/// Turn `self.gimbal.look` into `robot_gimbal_look`: dots and spaces are not
/// valid in tool names for most providers, and the `self.` prefix carries no
/// meaning once the tool is namespaced to this robot.
pub fn exposed_tool_name(device_name: &str) -> String {
    let trimmed = device_name.strip_prefix("self.").unwrap_or(device_name);
    let sanitized: String = trimmed
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    format!("robot_{sanitized}")
}

/// Tolerant JSON-RPC response: `code` is optional because the firmware omits it.
#[derive(Debug, Deserialize)]
struct DeviceResponse {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<DeviceError>,
}

#[derive(Debug, Deserialize)]
struct DeviceError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct ToolsListPage {
    #[serde(default)]
    tools: Vec<ToolsListEntry>,
    #[serde(default, rename = "nextCursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolsListEntry {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "inputSchema")]
    input_schema: Value,
}

/// Speaks JSON-RPC to the device over the `type:"mcp"` envelope.
pub struct RobotMcpClient {
    out: mpsc::Sender<Frame>,
    session_id: String,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<DeviceResponse>>>,
}

impl RobotMcpClient {
    pub fn new(out: mpsc::Sender<Frame>, session_id: String) -> Self {
        Self { out, session_id, next_id: AtomicU64::new(1), pending: Mutex::new(HashMap::new()) }
    }

    /// Feed a `type:"mcp"` payload received from the device.
    pub async fn handle_incoming(&self, payload: Value) {
        let Ok(response) = serde_json::from_value::<DeviceResponse>(payload) else {
            return;
        };
        let Some(id) = response.id else {
            // A notification or a malformed frame: nothing is waiting on it.
            return;
        };
        if let Some(waiter) = self.pending.lock().await.remove(&id) {
            let _ = waiter.send(response);
        }
    }

    /// Send one request and await its reply.
    async fn request(&self, method: &str, params: Value) -> Result<Value, ToolCallError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        // `id` is a number on purpose: the firmware silently drops string ids.
        let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let frame = Frame::Text(serialize_server_message(&ServerMessage::Mcp {
            session_id: self.session_id.clone(),
            payload,
        }));
        if self.out.send(frame).await.is_err() {
            self.pending.lock().await.remove(&id);
            return Err(ToolCallError::Offline);
        }

        let response = match timeout(Duration::from_secs(TOOL_CALL_TIMEOUT_SECS), rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => return Err(ToolCallError::Offline),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(ToolCallError::Timeout);
            }
        };
        if let Some(error) = response.error {
            return Err(ToolCallError::Rejected(error.message));
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// MCP handshake. `vision_url` is the **only** way to configure the
    /// firmware's photo-explain endpoint; when the transport has no reachable
    /// HTTP base we simply omit the capability rather than send a dead URL.
    pub async fn initialize(&self, vision_url: Option<&str>, vision_token: &str) -> anyhow::Result<()> {
        let mut capabilities = json!({});
        if let Some(url) = vision_url {
            capabilities["vision"] = json!({ "url": url, "token": vision_token });
        }
        self.request("initialize", json!({ "capabilities": capabilities }))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }

    /// Full toolset, following `nextCursor` until the device stops paging.
    pub async fn list_tools(&self) -> anyhow::Result<Vec<RobotToolDescriptor>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        // A cursor loop needs a bound: a firmware bug repeating one cursor
        // forever must not spin this task.
        for _ in 0..16 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self
                .request("tools/list", params)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let page: ToolsListPage = serde_json::from_value(result)?;
            for entry in page.tools {
                out.push(RobotToolDescriptor {
                    exposed_name: exposed_tool_name(&entry.name),
                    device_name: entry.name,
                    description: entry.description,
                    input_schema: entry.input_schema,
                });
            }
            match page.next_cursor {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => return Ok(out),
            }
        }
        tracing::warn!("robot: tools/list paging did not terminate; using what we have");
        Ok(out)
    }

    /// Invoke a device tool by its on-device name.
    pub async fn call_tool(&self, device_name: &str, args: Value) -> Result<String, ToolCallError> {
        let stack = if device_name.contains("camera") || device_name.contains("photo") {
            CAMERA_STACK
        } else {
            DEFAULT_STACK
        };
        let result = self
            .request(
                "tools/call",
                json!({ "name": device_name, "arguments": args, "stackSize": stack }),
            )
            .await?;
        let text = result
            .get("content")
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err(ToolCallError::Failed(text));
        }
        Ok(text)
    }
}
```

`lib.rs` 加 `pub mod mcp_bridge;`。

会话侧接线（`session.rs`）：hello 成功后建客户端并发 `initialize`+`tools/list`，`DeviceMessage::Mcp` 分支转发。在 hello 分支尾部追加：

```rust
                                let client = Arc::new(crate::mcp_bridge::RobotMcpClient::new(
                                    writer.tx.clone(),
                                    sid.clone(),
                                ));
                                mcp = Some(client.clone());
                                let vision_base = deps.vision_base.clone();
                                let device_token = deps.device_token.clone();
                                tokio::spawn(async move {
                                    let url = vision_base.map(|base| format!("{base}{}", crate::endpoint::VISION_PATH));
                                    if let Err(error) = client.initialize(url.as_deref(), &device_token).await {
                                        tracing::warn!(%error, "robot: MCP initialize failed");
                                        return;
                                    }
                                    match client.list_tools().await {
                                        Ok(tools) => tracing::info!(
                                            count = tools.len(),
                                            names = ?tools.iter().map(|t| &t.exposed_name).collect::<Vec<_>>(),
                                            "robot: device tools discovered"
                                        ),
                                        Err(error) => tracing::warn!(%error, "robot: tools/list failed"),
                                    }
                                });
```

`Mcp` 分支：

```rust
                            DeviceMessage::Mcp { payload } => {
                                if let Some(client) = &mcp {
                                    client.handle_incoming(payload).await;
                                }
                            }
```

局部状态加 `let mut mcp: Option<Arc<crate::mcp_bridge::RobotMcpClient>> = None;`。`SessionDeps` 加两个字段：`pub vision_base: Option<String>`（本次连接的 HTTP 基址，由 WS 升级时从 advertiser 取得并随 `AcceptedLink` 一起传入更准确——本任务先放在 `SessionDeps` 上，Task 21 装配时改为按连接注入）与 `pub device_token: String`。测试 `harness` 相应补 `vision_base: None, device_token: "tok".into()`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot mcp_bridge session`
Expected: PASS — mcp_bridge 11 个 + session 11 个测试全过

`a_silent_device_times_out_rather_than_hanging_forever` 用了 `tokio::time::pause()`，需要该测试标 `#[tokio::test(start_paused = true)]` 而不是在函数体里调 `pause()`（在多线程 runtime 上 `pause` 会 panic）。按此调整。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/
git commit -m "feat(robot): add device MCP client with paging and tolerant error handling"
```

---

### Task 18: 工具注册表与 loopback MCP 代理服务

**为什么是代理**：`SessionMcpTransport::Http` 映射到 `TransportType::StreamableHttp`（`factory/nomi.rs:1336-1348`），而 nomi-mcp 的 streamable-HTTP 客户端 `Accept: application/json, text/event-stream`（`transport/streamable_http.rs:45-46`），纯 JSON 响应即可。于是把机器人工具做成一个 loopback MCP 服务注册进会话的 `extra.session_mcp_servers`，**agent 引擎零改动**就拿到了 `robot_*` 工具。代理同时把设备的三处非标准行为规范化：数字 id、通知不可依赖、`tools/list` 分页，以及缺 `code` 的错误对象（agent 侧客户端的 `code` 是必填，直接转发会反序列化失败）。

**Files:**
- Create: `crates/backend/nomifun-robot/src/tool_registry.rs`
- Create: `crates/backend/nomifun-robot/src/mcp_proxy.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加两个 `pub mod`）

**Interfaces:**
- Consumes: `mcp_bridge::{RobotMcpClient, RobotToolDescriptor, ToolCallError}`、`bootstrap` 风格的端口协商（本 crate 内自持 `TcpListener::bind`）
- Produces:
  - `pub struct RobotToolRegistry`（`Default`），`pub async fn attach(&self, robot_id: &str, client: std::sync::Arc<RobotMcpClient>, tools: Vec<RobotToolDescriptor>)`、`pub async fn detach(&self, robot_id: &str)`、`pub async fn tools(&self, robot_id: &str) -> Vec<RobotToolDescriptor>`、`pub async fn call(&self, robot_id: &str, exposed_name: &str, args: serde_json::Value) -> Result<String, ToolCallError>`
  - `pub struct RobotMcpProxyServer { pub port: u16, pub token: String }`，`pub async fn spawn(registry: std::sync::Arc<RobotToolRegistry>) -> anyhow::Result<Self>`、`pub fn url_for(&self, robot_id: &str) -> String`、`pub fn headers(&self) -> std::collections::HashMap<String, String>`、`pub fn stop(&self)`
  - `pub const MCP_PROXY_SERVER_NAME: &str = "robot";`

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/mcp_proxy.rs`，先写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Frame;
    use crate::mcp_bridge::{RobotMcpClient, RobotToolDescriptor};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn descriptor(device_name: &str, exposed: &str) -> RobotToolDescriptor {
        RobotToolDescriptor {
            device_name: device_name.to_owned(),
            exposed_name: exposed.to_owned(),
            description: "turn the head".to_owned(),
            input_schema: json!({ "type": "object", "properties": { "direction": { "type": "string" } } }),
        }
    }

    /// A registry with one attached robot whose device answers `tools/call`.
    async fn fixture() -> (Arc<RobotToolRegistry>, Arc<RobotMcpClient>) {
        let (out_tx, mut out_rx) = mpsc::channel::<Frame>(16);
        let client = Arc::new(RobotMcpClient::new(out_tx, "sess-1".to_owned()));
        let echo = client.clone();
        tokio::spawn(async move {
            while let Some(Frame::Text(raw)) = out_rx.recv().await {
                let envelope: serde_json::Value = serde_json::from_str(&raw).unwrap();
                let payload = &envelope["payload"];
                let id = payload["id"].as_u64().unwrap();
                let name = payload["params"]["name"].as_str().unwrap_or_default().to_owned();
                echo.handle_incoming(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": format!("called {name}") }], "isError": false }
                }))
                .await;
            }
        });
        let registry = Arc::new(RobotToolRegistry::default());
        registry
            .attach("aa:bb", client.clone(), vec![descriptor("self.gimbal.look", "robot_gimbal_look")])
            .await;
        (registry, client)
    }

    async fn rpc(server: &RobotMcpProxyServer, robot_id: &str, body: serde_json::Value) -> serde_json::Value {
        let response = reqwest::Client::new()
            .post(server.url_for(robot_id))
            .bearer_auth(&server.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .contains("application/json"),
            "the agent's streamable-http client accepts plain JSON; keep it simple"
        );
        response.json().await.unwrap()
    }

    #[tokio::test]
    async fn initialize_answers_locally_without_touching_the_device() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(&server, "aa:bb", json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })).await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["capabilities"]["tools"], json!({}));
        assert_eq!(reply["result"]["serverInfo"]["name"], MCP_PROXY_SERVER_NAME);
        server.stop();
    }

    #[tokio::test]
    async fn tools_list_serves_cached_exposed_names_in_one_page() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(&server, "aa:bb", json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await;
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "robot_gimbal_look");
        assert_eq!(tools[0]["inputSchema"]["properties"]["direction"]["type"], "string");
        assert!(
            reply["result"].get("nextCursor").is_none(),
            "the device's 8000-byte paging is absorbed here, not passed on"
        );
        server.stop();
    }

    #[tokio::test]
    async fn tools_call_maps_the_exposed_name_back_to_the_device_name() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "aa:bb",
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "robot_gimbal_look", "arguments": { "direction": "left" } } }),
        )
        .await;
        assert_eq!(reply["result"]["content"][0]["text"], "called self.gimbal.look");
        assert_eq!(reply["result"]["isError"], false);
        server.stop();
    }

    #[tokio::test]
    async fn an_unknown_tool_is_a_method_not_found_error_with_a_code() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "aa:bb",
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "robot_nope" } }),
        )
        .await;
        assert_eq!(
            reply["error"]["code"], -32601,
            "the agent's client requires `code`; the firmware omits it, so we always supply one"
        );
        assert!(reply["error"]["message"].as_str().unwrap().contains("robot_nope"));
        server.stop();
    }

    #[tokio::test]
    async fn an_offline_robot_reports_an_error_instead_of_hanging() {
        let registry = Arc::new(RobotToolRegistry::default());
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "not-connected",
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "robot_gimbal_look" } }),
        )
        .await;
        assert!(reply["error"]["message"].as_str().unwrap().contains("offline"));
        assert!(reply["error"]["code"].is_i64());
        server.stop();
    }

    #[tokio::test]
    async fn detach_empties_the_toolset() {
        let (registry, _client) = fixture().await;
        assert_eq!(registry.tools("aa:bb").await.len(), 1);
        registry.detach("aa:bb").await;
        assert!(registry.tools("aa:bb").await.is_empty());
        assert!(matches!(
            registry.call("aa:bb", "robot_gimbal_look", json!({})).await,
            Err(crate::mcp_bridge::ToolCallError::Offline)
        ));
    }

    #[tokio::test]
    async fn a_missing_or_wrong_bearer_token_is_rejected() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let unauthenticated = reqwest::Client::new()
            .post(server.url_for("aa:bb"))
            .json(&json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/list" }))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), 401);

        let wrong = reqwest::Client::new()
            .post(server.url_for("aa:bb"))
            .bearer_auth("not-the-token")
            .json(&json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/list" }))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 401);
        server.stop();
    }

    #[tokio::test]
    async fn notifications_are_accepted_and_produce_no_body() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let response = reqwest::Client::new()
            .post(server.url_for("aa:bb"))
            .bearer_auth(&server.token)
            .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        server.stop();
    }

    #[tokio::test]
    async fn the_server_binds_loopback_only() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        assert!(server.url_for("aa:bb").starts_with("http://127.0.0.1:"));
        assert_ne!(server.port, 0);
        assert_eq!(server.token.len(), 64, "per-boot 256-bit secret");
        assert_eq!(server.headers().get("Authorization").unwrap(), &format!("Bearer {}", server.token));
        server.stop();
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot mcp_proxy`
Expected: FAIL — `cannot find struct RobotToolRegistry in this scope`

- [ ] **Step 3: 写最小实现**

`Cargo.toml` 的 `[dev-dependencies]` 追加 `reqwest = { workspace = true }`。

创建 `crates/backend/nomifun-robot/src/tool_registry.rs`：

```rust
//! Which robots are connected and what tools they offer.
//!
//! Sessions attach on handshake and detach on disconnect; the MCP proxy reads
//! from here. Tool descriptors are cached at attach time so `tools/list` never
//! has to round-trip a sleeping device.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::mcp_bridge::{RobotMcpClient, RobotToolDescriptor, ToolCallError};

struct Attached {
    client: Arc<RobotMcpClient>,
    tools: Vec<RobotToolDescriptor>,
}

/// Live robot → (MCP client, cached toolset).
#[derive(Default)]
pub struct RobotToolRegistry {
    inner: RwLock<HashMap<String, Attached>>,
}

impl RobotToolRegistry {
    /// Register a connected robot and its discovered tools.
    pub async fn attach(
        &self,
        robot_id: &str,
        client: Arc<RobotMcpClient>,
        tools: Vec<RobotToolDescriptor>,
    ) {
        self.inner.write().await.insert(robot_id.to_owned(), Attached { client, tools });
    }

    /// Forget a robot (link dropped).
    pub async fn detach(&self, robot_id: &str) {
        self.inner.write().await.remove(robot_id);
    }

    /// Cached toolset, empty when the robot is not connected.
    pub async fn tools(&self, robot_id: &str) -> Vec<RobotToolDescriptor> {
        self.inner.read().await.get(robot_id).map(|a| a.tools.clone()).unwrap_or_default()
    }

    /// Invoke a tool by the name models see.
    pub async fn call(
        &self,
        robot_id: &str,
        exposed_name: &str,
        args: Value,
    ) -> Result<String, ToolCallError> {
        let (client, device_name) = {
            let map = self.inner.read().await;
            let attached = map.get(robot_id).ok_or(ToolCallError::Offline)?;
            let tool = attached
                .tools
                .iter()
                .find(|t| t.exposed_name == exposed_name)
                .ok_or_else(|| ToolCallError::Rejected(format!("unknown tool {exposed_name}")))?;
            (attached.client.clone(), tool.device_name.clone())
        };
        client.call_tool(&device_name, args).await
    }
}
```

创建 `crates/backend/nomifun-robot/src/mcp_proxy.rs`（测试模块之前）：

```rust
//! A loopback MCP server fronting the connected robots.
//!
//! Registering this URL in a conversation's `extra.session_mcp_servers` is how
//! the companion's model gets `robot_*` tools with **zero** changes to the agent
//! engine: `SessionMcpTransport::Http` becomes `TransportType::StreamableHttp`
//! and the existing MCP client dials us like any other server.
//!
//! Follows the house pattern for loopback services (`ManagedModelServer`):
//! bind `127.0.0.1:0`, mint a per-boot bearer, keep the `JoinHandle`, abort on
//! `Drop`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use rand::RngCore;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::mcp_bridge::ToolCallError;
use crate::tool_registry::RobotToolRegistry;

/// The MCP server name the model sees this toolset under.
pub const MCP_PROXY_SERVER_NAME: &str = "robot";
/// MCP protocol revision we claim (same as the firmware's own).
const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone)]
struct ProxyState {
    registry: Arc<RobotToolRegistry>,
    token: String,
}

/// Loopback MCP front for robot tools.
pub struct RobotMcpProxyServer {
    pub port: u16,
    pub token: String,
    task: JoinHandle<()>,
}

impl RobotMcpProxyServer {
    /// Bind and start serving.
    pub async fn spawn(registry: Arc<RobotToolRegistry>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();

        let app = Router::new()
            .route("/robot-mcp/{robot_id}", post(handle_rpc))
            .with_state(ProxyState { registry, token: token.clone() });
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                tracing::error!(%error, "robot: MCP proxy server stopped");
            }
        });
        tracing::info!(port, "robot: MCP proxy listening");
        Ok(Self { port, token, task })
    }

    /// The URL to put in a conversation's `session_mcp_servers`.
    pub fn url_for(&self, robot_id: &str) -> String {
        format!("http://127.0.0.1:{}/robot-mcp/{robot_id}", self.port)
    }

    /// Headers for the same registration.
    pub fn headers(&self) -> HashMap<String, String> {
        HashMap::from([("Authorization".to_owned(), format!("Bearer {}", self.token))])
    }

    /// Stop serving.
    pub fn stop(&self) {
        self.task.abort();
    }
}

impl Drop for RobotMcpProxyServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn rpc_error(id: Option<Value>, code: i64, message: String) -> Response {
    // `code` is always present: the agent-side JSON-RPC type requires it, while
    // the firmware omits it — normalising here is the whole point of the proxy.
    Json(json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message },
    }))
    .into_response()
}

async fn handle_rpc(
    State(state): State<ProxyState>,
    Path(robot_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if presented != state.token {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }

    let id = body.get("id").cloned();
    let method = body.get("method").and_then(|m| m.as_str()).unwrap_or_default();

    // Notifications carry no id and expect no body.
    if id.is_none() || method.starts_with("notifications") {
        return StatusCode::ACCEPTED.into_response();
    }

    match method {
        "initialize" => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": MCP_PROXY_SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
            },
        }))
        .into_response(),
        "tools/list" => {
            let tools: Vec<Value> = state
                .registry
                .tools(&robot_id)
                .await
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.exposed_name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    })
                })
                .collect();
            // One page always: the device's cursor paging was absorbed at attach.
            Json(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } })).into_response()
        }
        "tools/call" => {
            let params = body.get("params").cloned().unwrap_or(Value::Null);
            let Some(name) = params.get("name").and_then(|n| n.as_str()) else {
                return rpc_error(id, -32602, "tools/call needs a name".to_owned());
            };
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match state.registry.call(&robot_id, name, args).await {
                Ok(text) => Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": text }], "isError": false },
                }))
                .into_response(),
                Err(ToolCallError::Rejected(message)) => rpc_error(id, -32601, message),
                Err(error @ ToolCallError::Offline) => rpc_error(id, -32000, error.to_string()),
                Err(error) => Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": error.to_string() }], "isError": true },
                }))
                .into_response(),
            }
        }
        other => rpc_error(id, -32601, format!("method not supported: {other}")),
    }
}
```

`lib.rs` 加 `pub mod mcp_proxy;` 与 `pub mod tool_registry;`。

会话侧接线（`session.rs`）：`SessionDeps` 加 `pub tools: Arc<crate::tool_registry::RobotToolRegistry>`；Task 17 里 `tools/list` 成功那一支改为同时 attach：

```rust
                                    match client.list_tools().await {
                                        Ok(tools) => {
                                            tracing::info!(count = tools.len(), "robot: device tools discovered");
                                            registry_for_tools.attach(&robot_id_for_tools, client.clone(), tools).await;
                                        }
                                        Err(error) => tracing::warn!(%error, "robot: tools/list failed"),
                                    }
```

（`registry_for_tools = deps.tools.clone()`、`robot_id_for_tools = robot_id.clone()` 在 spawn 之前克隆。）清理段追加 `deps.tools.detach(&robot_id).await;`。测试 `harness` 补 `tools: Arc::new(RobotToolRegistry::default())`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot mcp_proxy session`
Expected: PASS — mcp_proxy 9 个 + session 11 个测试全过

axum 0.8 的路径参数语法是 `{robot_id}`（0.7 是 `:robot_id`）；若报路由解析错误，按编译/运行期报错切换。`rand::rng()` 同 Task 2 的版本注意事项。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add tool registry and loopback MCP proxy for robot tools"
```

---

### Task 19: 视觉 explain 端点

**Files:**
- Modify: `crates/backend/nomifun-robot/src/routes/device.rs`（加 `/vision/explain` 路由与 handler）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（`axum` 需 `multipart` feature——workspace 已开）

**Interfaces:**
- Consumes: `registry::RobotRegistry::resolve_token`、`services::{SpeechContext, SpeechServices}`
- Produces:
  - `RobotDeviceState` 加 `pub speech: std::sync::Arc<dyn SpeechServices>`
  - `async fn vision_explain(...) -> Response`（挂 `POST /vision/explain`）
  - `pub const VISION_MAX_BYTES: usize = 8 * 1024 * 1024;`

- [ ] **Step 1: 写失败测试**

`routes/device.rs` 的测试模块追加：

```rust
    /// Build a chunked-style multipart body by hand. The firmware hardcodes this
    /// boundary and sends **no Content-Length**, so the parser must work off the
    /// header's boundary and a streaming body.
    fn multipart_body(question: &str, jpeg: &[u8]) -> Vec<u8> {
        let boundary = "----ESP32_CAMERA_BOUNDARY";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"question\"\r\n\r\n");
        body.extend_from_slice(question.as_bytes());
        body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"camera.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n",
        );
        body.extend_from_slice(jpeg);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }

    #[tokio::test]
    async fn vision_explain_returns_the_model_answer_as_200_json() {
        let (state, _dir) = state(true).await;
        let (record, token) = state
            .registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:10".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        state
            .registry
            .claim(record.activation_code.as_deref().unwrap(), "0190f5fe-7c00-7a00-8000-0000000000aa")
            .await
            .unwrap();
        state.speech_mock().set_vision_answer("桌上有一杯咖啡。");

        let response = device_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/vision/explain")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Device-Id", "aa:bb:cc:dd:ee:10")
                    .header(
                        "content-type",
                        "multipart/form-data; boundary=----ESP32_CAMERA_BOUNDARY",
                    )
                    .body(Body::from(multipart_body("看到什么", b"\xff\xd8\xff\xd9")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["success"], true);
        assert_eq!(
            value["result"], "桌上有一杯咖啡。",
            "the firmware hands this body straight to the model as the tool result"
        );
    }

    #[tokio::test]
    async fn vision_failures_are_still_200_so_the_model_can_read_why() {
        let (state, _dir) = state(true).await;
        let (_record, token) = state
            .registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:11".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        state.speech_mock().fail_next_vision("no vision model configured");

        let response = device_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/vision/explain")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", "multipart/form-data; boundary=----ESP32_CAMERA_BOUNDARY")
                    .body(Body::from(multipart_body("看到什么", b"\xff\xd8\xff\xd9")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a non-200 is collapsed by the firmware into 'Failed to upload photo', hiding the reason"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["success"], false);
        assert!(value["message"].as_str().unwrap().contains("vision model"));
    }

    #[tokio::test]
    async fn vision_requires_a_valid_device_token() {
        let (state, _dir) = state(true).await;
        let response = device_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/vision/explain")
                    .header("Authorization", "Bearer nope")
                    .header("content-type", "multipart/form-data; boundary=----ESP32_CAMERA_BOUNDARY")
                    .body(Body::from(multipart_body("q", b"\xff\xd8\xff\xd9")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn vision_rejects_a_body_with_no_file_part() {
        let (state, _dir) = state(true).await;
        let (_record, token) = state
            .registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:12".into(),
                    client_id: "cid".into(),
                    board: "b".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        let boundary = "----ESP32_CAMERA_BOUNDARY";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"question\"\r\n\r\nq\r\n--{boundary}--\r\n"
        );
        let response = device_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/vision/explain")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("content-type", format!("multipart/form-data; boundary={boundary}"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["success"], false);
    }
```

`MockSpeech` 需要一个新方法：在 `services.rs` 的 mock 里加 `fail_next_vision(&self, message: &str)`（与 `fail_next_transcribe` 同构，`explain_image` 先取失败再返回默认答案）。`state()` 辅助函数改为返回带 mock 句柄的结构（同 session 的 `Harness` 做法），并给 `RobotDeviceState` 填 `speech` 与 `acceptor`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot device`
Expected: FAIL — `no method named speech_mock` / `RobotDeviceState has no field named 'speech'`

- [ ] **Step 3: 写最小实现**

`routes/device.rs` 追加：

```rust
use axum::extract::Multipart;

/// Largest photo we will accept. A 640×480 quality-80 JPEG is 30–60 KB; this is
/// generous headroom without letting a bad actor stream forever.
pub const VISION_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Photo understanding.
///
/// The firmware streams `multipart/form-data` in **chunked** encoding with no
/// `Content-Length`, in many small chunks, using a boundary it hardcodes — so the
/// boundary must be read from the header, and the body must be parsed as a
/// stream. It also collapses every non-200 into "Failed to upload photo", which
/// is why failures here are reported as 200 with `success: false`: that way the
/// reason reaches the model instead of vanishing.
async fn vision_explain(
    State(state): State<RobotDeviceState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let token = header(&headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer ").map(str::to_owned))
        .unwrap_or_default();
    let Some(record) = state.registry.resolve_token(&token).await else {
        return (StatusCode::UNAUTHORIZED, "unknown device token").into_response();
    };
    let Some(companion_id) = record.companion_id.clone() else {
        return Json(json!({ "success": false, "message": "这台机器人还没有绑定伙伴" })).into_response();
    };

    let mut question = String::new();
    let mut jpeg: Option<Vec<u8>> = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                return Json(json!({ "success": false, "message": format!("上传解析失败: {error}") }))
                    .into_response();
            }
        };
        match field.name().unwrap_or_default() {
            "question" => question = field.text().await.unwrap_or_default(),
            "file" => match field.bytes().await {
                Ok(bytes) if bytes.len() <= VISION_MAX_BYTES => jpeg = Some(bytes.to_vec()),
                Ok(bytes) => {
                    return Json(json!({
                        "success": false,
                        "message": format!("图片过大: {} bytes", bytes.len())
                    }))
                    .into_response();
                }
                Err(error) => {
                    return Json(json!({ "success": false, "message": format!("读取图片失败: {error}") }))
                        .into_response();
                }
            },
            _ => {}
        }
    }

    let Some(jpeg) = jpeg else {
        return Json(json!({ "success": false, "message": "缺少 file 表单字段" })).into_response();
    };

    let ctx = crate::services::SpeechContext {
        robot_id: record.robot_id.clone(),
        companion_id,
    };
    match state.speech.explain_image(&ctx, jpeg, &question).await {
        Ok(result) => Json(json!({ "success": true, "result": result })).into_response(),
        Err(error) => {
            tracing::warn!(robot_id = %record.robot_id, %error, "robot: vision explain failed");
            Json(json!({ "success": false, "message": error.to_string() })).into_response()
        }
    }
}
```

`RobotDeviceState` 加字段 `pub speech: Arc<crate::services::SpeechServices>`（写作 `Arc<dyn crate::services::SpeechServices>`），`device_router` 加 `.route("/vision/explain", post(vision_explain))`。

**30 秒时限**：设备端 HTTP 超时是 30 秒且不可配置。视觉模型调用必须自带更短的上限，否则设备会先断开而我们还在等。在 `wiring/speech.rs`（Task 20）的 `explain_image` 里用 `tokio::time::timeout(Duration::from_secs(25), ...)`，超时返回 `success:false` 文案「视觉模型响应太慢」。本任务的 handler 不额外包超时（避免双层），仅在注释里点明约束。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot device`
Expected: PASS — 9 个测试全过（Task 6 的 5 个 + 本任务 4 个）

若 `Multipart` 提取器不可用，确认根 `Cargo.toml` 的 `axum` features 含 `multipart`（已确认包含）。

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/src/
git commit -m "feat(robot): add vision explain endpoint for device photo understanding"
```

---

### Task 20: 真实接线（SpeechServices / CompanionTurnDispatcher）

**这是全计划不确定性最高的任务**：它是唯一大量调用别的 crate 私有细节的地方。每个子步骤都先给查证命令，再给代码；若查证结果与代码不符，以查证结果为准修正签名，语义不变。

**Files:**
- Create: `crates/backend/nomifun-robot/src/wiring/mod.rs`
- Create: `crates/backend/nomifun-robot/src/wiring/speech.rs`
- Create: `crates/backend/nomifun-robot/src/wiring/dispatcher.rs`
- Modify: `crates/backend/nomifun-robot/src/lib.rs`（加 `pub mod wiring;`）
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（加 `nomifun-model-invoke`、`nomifun-companion`、`nomifun-conversation`、`nomifun-ai-agent`、`nomi-providers`、`nomi-types`、`nomifun-common`、`nomifun-db`）

**Interfaces:**
- Consumes（已查证）：
  - `nomifun_model_invoke::ModelInvokeService::invoke(&ModelRef, TaskRequest) -> Result<TaskOutcome, InvokeError>`
  - `TaskRequest::{SpeechRecognition(AsrRequest), SpeechSynthesis(TtsRequest)}`；`AsrRequest { audio: InputAsset, language, prompt, extra }`；`InputAsset { id, role, bytes, mime }`；`TtsRequest { text, voice, format, extra }`
  - `TaskOutcome::Done(TaskResult)`；`TaskResult::{Transcript { text, .. }, Assets(Vec<ProducedAsset>)}`；`ProducedAsset { data: ProducedData, mime: Option<String> }`
  - `nomi_providers::openai::OpenAiProvider::new(api_key: &str, base_url: &str, compat: ProviderCompat)`（anthropic 同签名）
  - `nomi_types::llm::LlmRequest { model, system, messages, tools, max_tokens, thinking, reasoning_effort }`；`LlmEvent::TextDelta(String)`
  - `nomifun_conversation::ConversationService::{send_message_with_idempotency_key, cancel, update_extra}`（`cancel_with_origin` 是私有的）
  - `nomifun_companion::build_companion_system_prompt(&CompanionStore, &CompanionProfileConfig, Option<&str>, bool) -> String`
  - Plan B Task 1/2：`CompanionProfileConfig.{fallback_model, vision_model, voice}`、`CompanionVadConfig::{effective_sensitivity, effective_min_silence_ms}`、`nomifun_api_types::TextToSpeechConfig::from_preferences()`
- Produces:
  - `pub struct RobotSpeech`，`pub fn new(invoke: Arc<ModelInvokeService>, companions: Arc<CompanionRegistryHandle>, prefs: Arc<dyn PreferenceReader>, providers: Arc<dyn ProviderRowReader>) -> Self`，`impl SpeechServices for RobotSpeech`
  - `pub struct RobotDispatcher`，`impl CompanionTurnDispatcher for RobotDispatcher`
  - `pub trait PreferenceReader: Send + Sync { async fn get(&self, key: &str) -> Option<serde_json::Value> }`
  - `pub trait ProviderRowReader: Send + Sync { async fn credentials(&self, provider_id: &str) -> Option<ProviderCredentials> }`，`pub struct ProviderCredentials { pub api_key: String, pub base_url: String, pub platform: String }`
  - `pub const VISION_TIMEOUT_SECS: u64 = 25;`

- [ ] **Step 1: 查证四处签名，把结果记在任务里**

Run 这四条，把输出贴进你的工作笔记（后续代码按输出对齐）：

```bash
grep -n "enum ProducedData" -A 10 crates/backend/nomifun-model-invoke/src/types.rs
grep -n "pub enum ProviderCompat" -A 12 crates/agent/nomi-config/src/config.rs
grep -n "pub async fn send_message_with_idempotency_key" -A 12 crates/backend/nomifun-conversation/src/service.rs
grep -rn "pub async fn wait_for_runtime_subscription" -A 10 crates/backend/nomifun-ai-agent/src/
grep -n "pub enum ContentBlock" -A 20 crates/agent/nomi-types/src/message.rs
grep -n "pub struct Message" -A 10 crates/agent/nomi-types/src/message.rs
```

预期得到：`ProducedData` 的变体（至少含内联字节与 URL 两种）、`ProviderCompat` 的变体名、`send_message_with_idempotency_key` 的完整参数表与返回类型、`wait_for_runtime_subscription` 的路径与签名、`ContentBlock::Image` 的字段名（`media_type` / `data`）、`Message` 的构造方式。

- [ ] **Step 2: 写失败测试（模型选择的纯逻辑，不打网络）**

创建 `crates/backend/nomifun-robot/src/wiring/mod.rs`：

```rust
//! Real implementations of the [`crate::services`] seams.
//!
//! Everything that knows about providers, the invoke layer, conversations and
//! companion profiles lives here — the pipelines above never do.

pub mod dispatcher;
pub mod speech;

pub use dispatcher::RobotDispatcher;
pub use speech::{ProviderCredentials, ProviderRowReader, PreferenceReader, RobotSpeech};
```

创建 `crates/backend/nomifun-robot/src/wiring/speech.rs`，先写测试模块（只测选型与解码分支，不测真实 provider）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn asr_prefers_the_companion_slot_over_the_global_preference() {
        let companion = Some(("p-companion".to_owned(), "whisper-1".to_owned()));
        let global = Some(("p-global".to_owned(), "gpt-4o-transcribe".to_owned()));
        assert_eq!(
            pick_model(companion.clone(), global.clone()),
            Some(("p-companion".to_owned(), "whisper-1".to_owned()))
        );
        assert_eq!(pick_model(None, global.clone()), global);
        assert_eq!(pick_model(None, None), None);
    }

    #[test]
    fn global_tts_preference_is_parsed_into_a_model_and_voice() {
        let value = json!({ "provider_id": "p-1", "model": "tts-1", "voice": "alloy" });
        let parsed = parse_global_tts(&value).unwrap();
        assert_eq!(parsed.0, "p-1");
        assert_eq!(parsed.1, "tts-1");
        assert_eq!(parsed.2.as_deref(), Some("alloy"));

        let no_voice = json!({ "provider_id": "p-1", "model": "tts-1" });
        assert_eq!(parse_global_tts(&no_voice).unwrap().2, None);
        assert!(parse_global_tts(&json!({ "model": "tts-1" })).is_none(), "provider_id is required");
        assert!(parse_global_tts(&json!("nonsense")).is_none());
    }

    #[test]
    fn pcm_assets_skip_the_container_decoder() {
        // A raw PCM asset is already at the rate the device expects.
        let pcm_bytes: Vec<u8> = vec![0x00, 0x01, 0x02, 0x03];
        let audio = audio_from_asset(&pcm_bytes, Some("audio/pcm")).unwrap();
        assert_eq!(audio.sample_rate, crate::protocol::DOWNLINK_SAMPLE_RATE);
        assert_eq!(audio.pcm.len(), 2, "four bytes are two 16-bit samples");
    }

    #[test]
    fn container_assets_go_through_symphonia() {
        let wav = crate::audio::pcm_to_wav(&vec![0i16; 2400], 24_000);
        let audio = audio_from_asset(&wav, Some("audio/wav")).unwrap();
        assert_eq!(audio.sample_rate, 24_000);
        assert_eq!(audio.pcm.len(), 2400);
    }

    #[test]
    fn an_unparseable_asset_is_an_error_not_silence() {
        assert!(audio_from_asset(b"not audio", Some("audio/mpeg")).is_err());
    }

    #[test]
    fn odd_length_pcm_does_not_panic() {
        let audio = audio_from_asset(&[0x00, 0x01, 0x02], Some("audio/pcm")).unwrap();
        assert_eq!(audio.pcm.len(), 1, "the trailing half sample is dropped");
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p nomifun-robot wiring`
Expected: FAIL — `cannot find function pick_model in this scope`

- [ ] **Step 4: 写实现**

`Cargo.toml` 的 `[dependencies]` 追加：

```toml
nomifun-common = { workspace = true }
nomifun-companion = { workspace = true }
nomifun-conversation = { workspace = true }
nomifun-ai-agent = { workspace = true }
nomifun-model-invoke = { workspace = true }
nomi-providers = { workspace = true }
nomi-types = { workspace = true }
nomi-config = { workspace = true }
base64 = { workspace = true }
```

`wiring/speech.rs` 顶部写入（测试模块之前）：

```rust
//! ASR, TTS and one-shot vision against the platform's model layer.
//!
//! ASR and TTS go through `ModelInvokeService` (the single resolve algorithm for
//! non-chat tasks). Vision does **not**: the invoke layer's `ChatTextRequest` is
//! text-only, so a one-shot VLM call is made straight through
//! `nomi_providers::LlmProvider` with an inline image block. Routing it through a
//! conversation instead would self-nest (the device is already inside a tool call
//! on that conversation) and blow the firmware's 30 s HTTP ceiling.

use std::sync::Arc;
use std::time::Duration;

use nomifun_model_invoke::types::{
    AsrRequest, InputAsset, ModelRef, ProducedData, TaskRequest, TaskResult, TtsRequest,
};
use nomifun_model_invoke::ModelInvokeService;
use serde_json::Value;

use crate::audio::{AudioBuffer, decode_container};
use crate::protocol::DOWNLINK_SAMPLE_RATE;
use crate::services::{SpeechContext, SpeechServices};

/// Vision must answer well inside the firmware's fixed 30 s HTTP timeout,
/// otherwise the device hangs up while we are still waiting.
pub const VISION_TIMEOUT_SECS: u64 = 25;
/// Cap on a single vision answer so the device's 8 KB body handling stays happy.
const VISION_MAX_TOKENS: u32 = 512;

/// Read a global client preference.
#[async_trait::async_trait]
pub trait PreferenceReader: Send + Sync {
    async fn get(&self, key: &str) -> Option<Value>;
}

/// Provider credentials for a direct (non-invoke) call.
#[derive(Debug, Clone)]
pub struct ProviderCredentials {
    pub api_key: String,
    pub base_url: String,
    pub platform: String,
}

/// Read a provider row's credentials.
#[async_trait::async_trait]
pub trait ProviderRowReader: Send + Sync {
    async fn credentials(&self, provider_id: &str) -> Option<ProviderCredentials>;
}

/// Read a companion's model slots.
#[async_trait::async_trait]
pub trait CompanionSlotReader: Send + Sync {
    /// `(provider_id, model)` of the companion's ASR slot, if set.
    async fn asr_slot(&self, companion_id: &str) -> Option<(String, String)>;
    /// `(provider_id, model, voice)` of the companion's TTS slot, if set.
    async fn tts_slot(&self, companion_id: &str) -> Option<(String, String, Option<String>)>;
    /// `(provider_id, model)` for vision: the companion's `vision_model`, else
    /// its main chat model when that model has the vision trait.
    async fn vision_slot(&self, companion_id: &str) -> Option<(String, String)>;
}

/// First non-empty of the two candidates. Companion slots always win: a
/// per-robot voice is the whole point of having the slot.
fn pick_model(
    companion: Option<(String, String)>,
    global: Option<(String, String)>,
) -> Option<(String, String)> {
    companion.or(global)
}

/// Parse the `tools.textToSpeech` preference value.
///
/// **Seam note**: Plan B Task 2 lands
/// `nomifun_api_types::TextToSpeechConfig::from_preferences()` as the single
/// source of truth for this key. Once it is on main, replace this body with a
/// delegation to it and keep both the signature and the tests — they then pin
/// that the shared parser still satisfies this contract. Two independent parsers
/// for one preference key is exactly the drift the model-provider spec already
/// records as debt.
fn parse_global_tts(value: &Value) -> Option<(String, String, Option<String>)> {
    let provider_id = value.get("provider_id")?.as_str()?.to_owned();
    let model = value.get("model")?.as_str()?.to_owned();
    let voice = value.get("voice").and_then(|v| v.as_str()).map(str::to_owned);
    Some((provider_id, model, voice))
}

/// Parse the `tools.speechToText` preference value (same first two keys).
fn parse_global_asr(value: &Value) -> Option<(String, String)> {
    let provider_id = value.get("provider_id")?.as_str()?.to_owned();
    let model = value.get("model")?.as_str()?.to_owned();
    Some((provider_id, model))
}

/// Turn a synthesised asset into PCM. `audio/pcm` is already at the rate we
/// requested, so it bypasses the decoder entirely (that is why `format: "pcm"`
/// is preferred: no container, no guessing).
fn audio_from_asset(bytes: &[u8], mime: Option<&str>) -> anyhow::Result<AudioBuffer> {
    if mime.is_some_and(|m| m.contains("pcm")) {
        let pcm = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return Ok(AudioBuffer { pcm, sample_rate: DOWNLINK_SAMPLE_RATE });
    }
    decode_container(bytes, mime)
}

/// The real [`SpeechServices`].
pub struct RobotSpeech {
    invoke: Arc<ModelInvokeService>,
    slots: Arc<dyn CompanionSlotReader>,
    prefs: Arc<dyn PreferenceReader>,
    providers: Arc<dyn ProviderRowReader>,
}

impl RobotSpeech {
    pub fn new(
        invoke: Arc<ModelInvokeService>,
        slots: Arc<dyn CompanionSlotReader>,
        prefs: Arc<dyn PreferenceReader>,
        providers: Arc<dyn ProviderRowReader>,
    ) -> Self {
        Self { invoke, slots, prefs, providers }
    }
}

#[async_trait::async_trait]
impl SpeechServices for RobotSpeech {
    async fn transcribe(&self, ctx: &SpeechContext, wav: Vec<u8>) -> anyhow::Result<String> {
        let global = self
            .prefs
            .get("tools.speechToText")
            .await
            .as_ref()
            .and_then(parse_global_asr);
        let (provider_id, model) = pick_model(self.slots.asr_slot(&ctx.companion_id).await, global)
            .ok_or_else(|| anyhow::anyhow!("no speech-recognition model configured"))?;

        let request = TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset {
                id: None,
                role: "audio".to_owned(),
                bytes: wav,
                mime: "audio/wav".to_owned(),
            },
            language: None,
            prompt: None,
            extra: Value::Object(Default::default()),
        });
        let outcome = self.invoke.invoke(&ModelRef { provider_id, model }, request).await?;
        match outcome {
            nomifun_model_invoke::types::TaskOutcome::Done(TaskResult::Transcript { text, .. }) => {
                Ok(text)
            }
            nomifun_model_invoke::types::TaskOutcome::Done(_) => {
                anyhow::bail!("speech recognition returned a non-transcript result")
            }
            nomifun_model_invoke::types::TaskOutcome::Pending(_) => {
                anyhow::bail!("speech recognition must be synchronous for a live conversation")
            }
        }
    }

    async fn synthesize(&self, ctx: &SpeechContext, text: &str) -> anyhow::Result<AudioBuffer> {
        let global = self.prefs.get("tools.textToSpeech").await.as_ref().and_then(parse_global_tts);
        let (provider_id, model, voice) = self
            .slots
            .tts_slot(&ctx.companion_id)
            .await
            .or(global)
            .ok_or_else(|| anyhow::anyhow!("no speech-synthesis model configured"))?;

        let request = TaskRequest::SpeechSynthesis(TtsRequest {
            text: text.to_owned(),
            voice,
            // `pcm` is the one format that comes back without a container, and
            // the OpenAI contract makes it 24 kHz mono — exactly what we told
            // the device to expect.
            format: Some("pcm".to_owned()),
            extra: Value::Object(Default::default()),
        });
        let outcome = self.invoke.invoke(&ModelRef { provider_id, model }, request).await?;
        let nomifun_model_invoke::types::TaskOutcome::Done(TaskResult::Assets(assets)) = outcome
        else {
            anyhow::bail!("speech synthesis returned no audio");
        };
        let asset = assets.into_iter().next().ok_or_else(|| anyhow::anyhow!("empty audio result"))?;
        let bytes = match asset.data {
            ProducedData::Bytes(bytes) => bytes,
            other => anyhow::bail!("unsupported synthesised audio payload: {other:?}"),
        };
        audio_from_asset(&bytes, asset.mime.as_deref())
    }

    async fn explain_image(
        &self,
        ctx: &SpeechContext,
        jpeg: Vec<u8>,
        question: &str,
    ) -> anyhow::Result<String> {
        let (provider_id, model) = self
            .slots
            .vision_slot(&ctx.companion_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("未配置视觉模型"))?;
        let credentials = self
            .providers
            .credentials(&provider_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("视觉模型的供应商不可用"))?;

        let answer = tokio::time::timeout(
            Duration::from_secs(VISION_TIMEOUT_SECS),
            one_shot_vision(&credentials, &model, jpeg, question),
        )
        .await
        .map_err(|_| anyhow::anyhow!("视觉模型响应太慢"))??;
        Ok(answer)
    }
}

/// One non-streaming-shaped VLM call, collected from the stream.
async fn one_shot_vision(
    credentials: &ProviderCredentials,
    model: &str,
    jpeg: Vec<u8>,
    question: &str,
) -> anyhow::Result<String> {
    use base64::Engine;
    use nomi_providers::LlmProvider;
    use nomi_types::llm::{LlmEvent, LlmRequest};
    use nomi_types::message::{ContentBlock, Message};

    let data = base64::engine::general_purpose::STANDARD.encode(&jpeg);
    let prompt = if question.trim().is_empty() { "描述这张图片。" } else { question };
    let request = LlmRequest {
        model: model.to_owned(),
        system: "你在为一台物理机器人看图。用一到两句中文口语描述你看到的内容，直接回答问题。".to_owned(),
        messages: vec![Message::user(vec![
            ContentBlock::Image { media_type: "image/jpeg".to_owned(), data },
            ContentBlock::Text { text: prompt.to_owned() },
        ])],
        tools: vec![],
        max_tokens: VISION_MAX_TOKENS,
        thinking: None,
        reasoning_effort: None,
    };

    // Anthropic-platform rows speak the anthropic wire shape; everything else in
    // this repo defaults to OpenAI-compatible (see `factory/platform_table.rs`).
    let provider: Box<dyn LlmProvider> = if credentials.platform == "anthropic" {
        Box::new(nomi_providers::anthropic::AnthropicProvider::new(
            &credentials.api_key,
            &credentials.base_url,
            Default::default(),
        ))
    } else {
        Box::new(nomi_providers::openai::OpenAiProvider::new(
            &credentials.api_key,
            &credentials.base_url,
            Default::default(),
        ))
    };

    let mut rx = provider.stream(&request).await?;
    let mut answer = String::new();
    while let Some(event) = rx.recv().await {
        if let LlmEvent::TextDelta(delta) = event {
            answer.push_str(&delta);
        }
    }
    if answer.trim().is_empty() {
        anyhow::bail!("视觉模型没有返回内容");
    }
    Ok(answer)
}
```

`Message::user(...)` 与 `ContentBlock::{Image, Text}` 的确切构造以 Step 1 的查证输出为准（若 `Message` 没有 `user` 构造器，改为字面量 `Message { role: Role::User, content: vec![...] }`）；`ProviderCompat` 若无 `Default`，用查证到的"无特殊兼容"变体替换 `Default::default()`。

`wiring/dispatcher.rs`：

```rust
//! The real [`CompanionTurnDispatcher`].
//!
//! Thread creation mirrors the channel domain's own dispatch path
//! (`nomifun-channel/src/message_service.rs:416-500`): build a
//! `SendMessageRequest`, send it with an idempotency key, then attach to the
//! runtime's stream. The device inherits the installation owner — robots do not
//! log in.
//!
//! Cancellation uses the **public** `cancel` (`CancelOrigin` is crate-private),
//! and never `runtime_registry.terminate` — that would kill the runtime rather
//! than stop one turn.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::services::{CompanionTurnDispatcher, TurnEvent};
use crate::vad::VadTuning;

/// What the host must supply for real conversation access. Each closure-shaped
/// field is filled in `nomifun-app` where the concrete services live, keeping
/// this crate free of the router's type graph.
pub struct RobotDispatcher {
    inner: Arc<dyn RobotConversationBackend>,
}

/// The narrow view of the conversation stack this crate needs.
#[async_trait::async_trait]
pub trait RobotConversationBackend: Send + Sync {
    /// Find or create the `(robot, companion)` thread, refreshing its
    /// `session_mcp_servers` entry so the robot MCP proxy URL is never stale
    /// across restarts (the port is per-boot).
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String>;
    /// Send one user turn and stream reduced events.
    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>>;
    /// Public cancel.
    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()>;
    /// `voice.vad` of the companion profile.
    async fn vad_tuning(&self, companion_id: &str) -> VadTuning;
    /// Whether `fallback_model` is set.
    async fn has_fallback_model(&self, companion_id: &str) -> bool;
}

impl RobotDispatcher {
    pub fn new(inner: Arc<dyn RobotConversationBackend>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl CompanionTurnDispatcher for RobotDispatcher {
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String> {
        self.inner.ensure_thread(robot_id, companion_id).await
    }

    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
        self.inner.dispatch(conversation_id, text, use_fallback_model).await
    }

    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()> {
        self.inner.cancel(conversation_id).await
    }

    async fn vad_tuning(&self, companion_id: &str) -> VadTuning {
        self.inner.vad_tuning(companion_id).await
    }

    async fn has_fallback_model(&self, companion_id: &str) -> bool {
        self.inner.has_fallback_model(companion_id).await
    }
}
```

**`RobotConversationBackend` 的具体实现放在 Task 21**（`nomifun-app` 内），因为只有那里能同时拿到 `ConversationService`、`AgentRuntimeRegistry`、`CompanionRegistry` 与 `authoritative_user_id`。这样 `nomifun-robot` 不需要依赖 `nomifun-app`，依赖方向保持单向。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p nomifun-robot wiring`
Expected: PASS — 6 个测试全过

- [ ] **Step 6: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "feat(robot): wire ASR, TTS and one-shot vision to the model layer"
```

---

### Task 21: 管理 REST、宿主装配与关闭清理

**Files:**
- Create: `crates/backend/nomifun-robot/src/routes/admin.rs`
- Create: `crates/backend/nomifun-app/src/robot_wiring.rs`（`RobotConversationBackend` 的真实实现 + 三个 reader）
- Modify: `crates/backend/nomifun-robot/src/routes/mod.rs`
- Modify: `crates/backend/nomifun-app/src/services.rs`（新增字段与构造）
- Modify: `crates/backend/nomifun-app/src/router/routes.rs`（`nest("/robot", ...)` + 挂 `/api/robots*`）
- Modify: `crates/backend/nomifun-app/src/desktop.rs`（把 LAN 状态投射成 `LanEndpointSnapshot`）
- Modify: `crates/backend/nomifun-app/Cargo.toml`（加 `nomifun-robot`）

**Interfaces:**
- Consumes: `RobotRegistry`、`RobotStatusRegistry`、`RobotToolRegistry`、`RobotMcpProxyServer`、`LanAdvertiser`、`RobotGateway`、`LanWsSource`
- Produces:
  - `pub fn admin_router(state: RobotAdminState) -> axum::Router`（6 条路由，见共享契约）
  - `pub struct RobotAdminState { pub registry: Arc<RobotRegistry>, pub status: Arc<RobotStatusRegistry>, pub advertiser: Arc<dyn EndpointAdvertiser> }`
  - `nomifun-app` 侧：`pub struct RobotServices { pub registry, pub status, pub tools, pub advertiser, pub acceptor, pub proxy, gateway_task }`，`pub async fn build_robot_services(...) -> anyhow::Result<RobotServices>`

**DTO 大小写约定（不要"顺手改正"）**：`RobotDto` 与 `RobotStatusDto` 全部字段是 **snake_case**（`robot_id`、`companion_id`、`firmware_version`、`last_seen`、`created_at`、`changed_at`）。这与同仓 `SshStatusEvent` 的 camelCase 惯例相反，但 Plan B 的 UI 契约测试按 snake_case 钉住了；两边必须一致，任何一侧改成 camelCase 都会造成运行时静默失效。故这两个 struct **不得**加 `#[serde(rename_all = "camelCase")]`。

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/src/routes/admin.rs`，测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{RobotRegistry, RobotReport};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn seeded() -> (RobotAdminState, String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (record, _) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:ff".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1_700_000_000_000,
            )
            .await
            .unwrap();
        let code = record.activation_code.clone().unwrap();
        let (_tx, rx) = tokio::sync::watch::channel(crate::endpoint::LanEndpointSnapshot {
            enabled: true,
            port: 25808,
            ipv4s: vec![std::net::Ipv4Addr::new(192, 168, 1, 20)],
        });
        std::mem::forget(_tx);
        let status = Arc::new(crate::status::RobotStatusRegistry::new(
            crate::events::RobotEventEmitter::new(Arc::new(NullSink)),
            "owner-1".to_owned(),
        ));
        let state = RobotAdminState {
            registry,
            status,
            advertiser: Arc::new(crate::endpoint::LanAdvertiser::new(rx)),
        };
        (state, code, dir)
    }

    struct NullSink;
    impl nomifun_realtime::UserEventSink for NullSink {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    async fn json_of(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 128 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn list_wraps_records_in_a_robots_key_with_snake_case_fields() {
        let (state, _code, _dir) = seeded().await;
        let response = admin_router(state)
            .oneshot(Request::builder().uri("/api/robots").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value = json_of(response).await;
        let robots = value["robots"].as_array().unwrap();
        assert_eq!(robots.len(), 1);
        assert_eq!(robots[0]["robot_id"], "aa:bb:cc:dd:ee:ff");
        assert_eq!(robots[0]["firmware_version"], "1.9.0");
        assert!(robots[0]["last_seen"].is_string());
        assert!(robots[0].get("token_hash").is_none(), "secrets never leave the process");
        assert!(robots[0].get("activation_code").is_none(), "codes are for the device screen only");
    }

    #[tokio::test]
    async fn claim_binds_then_404s_on_a_spent_code_and_409s_on_a_bound_robot() {
        let (state, code, _dir) = seeded().await;

        let ok = admin_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/robots/claim")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"code":"{code}","companion_id":"0190f5fe-7c00-7a00-8000-0000000000aa"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(json_of(ok).await["companion_id"], "0190f5fe-7c00-7a00-8000-0000000000aa");

        let spent = admin_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/robots/claim")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"code":"{code}","companion_id":"0190f5fe-7c00-7a00-8000-0000000000bb"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(spent.status(), StatusCode::NOT_FOUND, "a claimed code no longer exists");
    }

    #[tokio::test]
    async fn patch_renames_and_unbinds() {
        let (state, code, _dir) = seeded().await;
        state.registry.claim(&code, "0190f5fe-7c00-7a00-8000-0000000000aa").await.unwrap();

        let renamed = admin_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"书桌机器人"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(json_of(renamed).await["name"], "书桌机器人");

        let unbound = admin_router(state)
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"companion_id":null}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(json_of(unbound).await["companion_id"].is_null());
    }

    #[tokio::test]
    async fn delete_removes_and_is_idempotent_enough() {
        let (state, _code, _dir) = seeded().await;
        let gone = admin_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        let missing = admin_router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/robots/aa:bb:cc:dd:ee:ff")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn statuses_and_endpoints_expose_what_the_ui_needs() {
        let (state, _code, _dir) = seeded().await;
        state
            .status
            .publish("aa:bb:cc:dd:ee:ff", Some("c1"), crate::status::RobotPhase::Listening, 42)
            .await;

        let statuses = admin_router(state.clone())
            .oneshot(Request::builder().uri("/api/robots/statuses").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let value = json_of(statuses).await;
        assert_eq!(value["statuses"][0]["phase"], "listening");
        assert_eq!(value["statuses"][0]["changed_at"], 42);

        let endpoints = admin_router(state)
            .oneshot(Request::builder().uri("/api/robots/endpoints").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let value = json_of(endpoints).await;
        assert_eq!(value["lan_enabled"], true);
        assert_eq!(value["ota_urls"][0], "http://192.168.1.20:25808/robot/ota");
    }

    #[tokio::test]
    async fn statuses_route_is_not_shadowed_by_the_id_route() {
        // `/api/robots/statuses` must not be captured as `{robot_id}`.
        let (state, _code, _dir) = seeded().await;
        let response = admin_router(state)
            .oneshot(Request::builder().uri("/api/robots/statuses").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(json_of(response).await.get("statuses").is_some());
    }
}
```

`RobotAdminState` 需要 `Clone`（三个字段都是 `Arc`）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot admin`
Expected: FAIL — `cannot find function admin_router in this scope`

- [ ] **Step 3: 写实现（crate 侧）**

`routes/admin.rs` 顶部：

```rust
//! The management face. Mounted **inside** the instance-owner auth layer — this
//! is the desktop UI talking, not a device.
//!
//! `GET /api/robots/statuses` and `/endpoints` are declared before
//! `/{robot_id}` so the literal segments win; the SSH domain hit the same
//! shadowing trap and solved it the same way.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::dto::RobotDto;
use crate::endpoint::EndpointAdvertiser;
use crate::registry::{ClaimError, RobotRegistry};
use crate::status::RobotStatusRegistry;

/// Shared state of the management face.
#[derive(Clone)]
pub struct RobotAdminState {
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub advertiser: Arc<dyn EndpointAdvertiser>,
}

#[derive(Deserialize)]
struct ClaimBody {
    code: String,
    companion_id: String,
}

#[derive(Deserialize)]
struct PatchBody {
    #[serde(default)]
    name: Option<String>,
    /// Absent = leave the binding alone; `null` = unbind; a value = rebind.
    #[serde(default, deserialize_with = "double_option")]
    companion_id: Option<Option<String>>,
}

/// Distinguish "key absent" from "key present and null".
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Management routes.
pub fn admin_router(state: RobotAdminState) -> Router {
    Router::new()
        .route("/api/robots", get(list))
        .route("/api/robots/claim", post(claim))
        .route("/api/robots/statuses", get(statuses))
        .route("/api/robots/endpoints", get(endpoints))
        .route("/api/robots/{robot_id}", patch(patch_robot).delete(delete_robot))
        .with_state(state)
}

async fn list(State(state): State<RobotAdminState>) -> Response {
    let robots: Vec<RobotDto> = state.registry.list().await.iter().map(RobotDto::from).collect();
    Json(json!({ "robots": robots })).into_response()
}

async fn claim(State(state): State<RobotAdminState>, Json(body): Json<ClaimBody>) -> Response {
    match state.registry.claim(&body.code, &body.companion_id).await {
        Ok(record) => Json(RobotDto::from(&record)).into_response(),
        Err(ClaimError::NotFound) => {
            (StatusCode::NOT_FOUND, Json(json!({ "message": "激活码不存在或已被使用" }))).into_response()
        }
        Err(ClaimError::AlreadyBound { companion_id }) => (
            StatusCode::CONFLICT,
            Json(json!({ "message": "这台机器人已绑定其他伙伴", "companion_id": companion_id })),
        )
            .into_response(),
    }
}

async fn patch_robot(
    State(state): State<RobotAdminState>,
    Path(robot_id): Path<String>,
    Json(body): Json<PatchBody>,
) -> Response {
    match state.registry.patch(&robot_id, body.name, body.companion_id).await {
        Ok(record) => Json(RobotDto::from(&record)).into_response(),
        Err(ClaimError::NotFound) => (StatusCode::NOT_FOUND, "unknown robot").into_response(),
        Err(ClaimError::AlreadyBound { companion_id }) => (
            StatusCode::CONFLICT,
            Json(json!({ "message": "这台机器人已绑定其他伙伴", "companion_id": companion_id })),
        )
            .into_response(),
    }
}

async fn delete_robot(
    State(state): State<RobotAdminState>,
    Path(robot_id): Path<String>,
) -> Response {
    match state.registry.remove(&robot_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "unknown robot").into_response(),
        Err(error) => {
            tracing::error!(%robot_id, %error, "robot: delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "registry write failed").into_response()
        }
    }
}

async fn statuses(State(state): State<RobotAdminState>) -> Response {
    Json(json!({ "statuses": state.status.snapshot().await })).into_response()
}

async fn endpoints(State(state): State<RobotAdminState>) -> Response {
    Json(json!({
        "ota_urls": state.advertiser.ota_urls(),
        "lan_enabled": state.advertiser.is_available(),
    }))
    .into_response()
}
```

`routes/mod.rs` 加 `pub mod admin;` 与 `pub use admin::{RobotAdminState, admin_router};`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot admin`
Expected: PASS — 6 个测试全过

- [ ] **Step 5: 宿主装配（`nomifun-app`）**

`crates/backend/nomifun-app/Cargo.toml` 加 `nomifun-robot = { workspace = true }`。

创建 `crates/backend/nomifun-app/src/robot_wiring.rs`，实现 `nomifun_robot::wiring::dispatcher::RobotConversationBackend` 与三个 reader trait。关键点（其余按 Task 20 Step 1 查证到的签名照抄 `nomifun-channel/src/message_service.rs:416-500`）：

```rust
//! Concrete conversation/model access for the robot gateway. Lives here because
//! only this crate holds `ConversationService`, the runtime registry, the
//! companion registry and the installation owner id at once.

use std::sync::Arc;

use nomifun_api_types::{AgentType, CreateConversationRequest, SendMessageRequest};
use nomifun_robot::services::TurnEvent;
use nomifun_robot::vad::VadTuning;
use serde_json::json;
use tokio::sync::mpsc;

pub struct AppRobotBackend {
    pub conversations: Arc<nomifun_conversation::ConversationService>,
    pub runtime_registry: Arc<dyn nomifun_ai_agent::AgentRuntimeRegistry>,
    pub companions: Arc<nomifun_companion::CompanionService>,
    pub owner_user_id: Arc<str>,
    /// Live robot MCP proxy URL + headers, so a reused thread is refreshed
    /// instead of pointing at last boot's port.
    pub mcp_proxy: Arc<nomifun_robot::mcp_proxy::RobotMcpProxyServer>,
}

#[async_trait::async_trait]
impl nomifun_robot::wiring::dispatcher::RobotConversationBackend for AppRobotBackend {
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String> {
        let session_mcp = json!([{
            "mcp_server_id": format!("robot-{robot_id}"),
            "name": nomifun_robot::mcp_proxy::MCP_PROXY_SERVER_NAME,
            "transport": {
                "type": "http",
                "url": self.mcp_proxy.url_for(robot_id),
                "headers": self.mcp_proxy.headers(),
            },
        }]);

        // Reuse the thread recorded for this pair, refreshing the per-boot proxy
        // URL; `update_extra` is the public patch path.
        if let Some(existing) = self.lookup_thread(robot_id, companion_id).await? {
            self.conversations
                .update_extra(&existing, json!({ "session_mcp_servers": session_mcp }))
                .await?;
            return Ok(existing);
        }

        let profile = self.companions.load_profile(companion_id).await?;
        let system_prompt = self.companions.build_system_prompt(&profile, Some("robot")).await;
        let request = CreateConversationRequest {
            r#type: AgentType::Nomi,
            extra: json!({
                "robot_session": true,
                "robot_id": robot_id,
                "companion_id": companion_id,
                "companion_session": true,
                "system_prompt": format!("{system_prompt}\n\n{}", robot_body_prompt()),
                "session_mode": "yolo",
                "session_mcp_servers": session_mcp,
            }),
            ..Default::default()
        };
        let conversation = self.conversations.create(&self.owner_user_id, request).await?;
        self.record_thread(robot_id, companion_id, &conversation.id).await?;
        Ok(conversation.id)
    }

    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
        if use_fallback_model {
            // The fallback slot is applied by patching the conversation's model
            // before the retry; the agent build reads it from there.
            self.apply_fallback_model(conversation_id).await?;
        }
        let request = SendMessageRequest {
            content: text.to_owned(),
            files: vec![],
            inject_skills: vec![],
            hidden: false,
            origin: None,
            channel_platform: Some("robot".to_owned()),
        };
        let delivery = self
            .conversations
            .send_message_with_idempotency_key(
                &self.owner_user_id,
                conversation_id,
                &uuid::Uuid::now_v7().to_string(),
                request,
                &self.runtime_registry,
            )
            .await?;
        let (tx, rx) = mpsc::channel(64);
        let stream = if delivery.completed {
            None
        } else {
            nomifun_ai_agent::wait_for_runtime_subscription(&self.runtime_registry, conversation_id)
                .await
        };
        tokio::spawn(async move {
            let Some(mut stream) = stream else {
                let _ = tx.send(TurnEvent::Done).await;
                return;
            };
            loop {
                match stream.recv().await {
                    Ok(event) => match reduce_event(event) {
                        Some(TurnEvent::Done) => {
                            let _ = tx.send(TurnEvent::Done).await;
                            break;
                        }
                        Some(reduced) => {
                            let terminal = matches!(reduced, TurnEvent::Failed { .. });
                            if tx.send(reduced).await.is_err() || terminal {
                                break;
                            }
                        }
                        None => {}
                    },
                    Err(_) => {
                        let _ = tx.send(TurnEvent::Done).await;
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }

    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()> {
        // Public `cancel` only: `cancel_with_origin` is crate-private, and
        // `runtime_registry.terminate` would kill the runtime, not the turn.
        self.conversations
            .cancel(&self.owner_user_id, conversation_id, &self.runtime_registry)
            .await?;
        Ok(())
    }

    async fn vad_tuning(&self, companion_id: &str) -> VadTuning {
        match self.companions.load_profile(companion_id).await {
            Ok(profile) => VadTuning::from_profile(
                &profile.voice.vad.engine,
                profile.voice.vad.effective_sensitivity(),
                profile.voice.vad.effective_min_silence_ms(),
            ),
            Err(_) => VadTuning::default(),
        }
    }

    async fn has_fallback_model(&self, companion_id: &str) -> bool {
        self.companions
            .load_profile(companion_id)
            .await
            .map(|p| p.fallback_model.is_some())
            .unwrap_or(false)
    }
}

/// The physical-body section appended to the companion persona.
fn robot_body_prompt() -> &'static str {
    "你现在通过一台物理机器人和用户说话。它有一块 OLED 表情屏、一个可以转动的头（云台）、扬声器和麦克风。\n\
     - 回复必须简短口语化：每句不超过 40 字，整体不超过 3 句，除非用户明确要求详细内容。\n\
     - 每句话可以用 [emotion:名] 开头来驱动表情和头部动作，可用的名字只有：neutral, happy, laughing, funny, sad, angry, crying, loving, embarrassed, surprised, shocked, thinking, winking, cool, relaxed, delicious, kissy, confident, sleepy, silly, confused。\n\
     - 需要转头、看某个方向或调音量时，用 robot_ 开头的工具。"
}
```

`reduce_event`、`lookup_thread`、`record_thread`、`apply_fallback_model` 四个辅助函数（同文件，全部为真实代码）：

```rust
use nomifun_api_types::{AgentErrorOwnership, UpdateConversationRequest};
use nomifun_ai_agent::protocol::events::AgentStreamEvent;

/// Reduce the agent's rich event stream to the three things the downlink needs.
///
/// `provider_fault` decides whether a fallback-model retry makes sense, and the
/// platform already classifies that for us: `UserLlmProvider` and
/// `UnknownUpstream` are upstream problems, everything else is ours or the
/// user's and would fail identically on the fallback model.
fn reduce_event(event: AgentStreamEvent) -> Option<TurnEvent> {
    match event {
        AgentStreamEvent::Text(data) => Some(TurnEvent::Text(data.content)),
        AgentStreamEvent::Finish(_) => Some(TurnEvent::Done),
        AgentStreamEvent::Error(data) => {
            let provider_fault = matches!(
                data.ownership,
                Some(AgentErrorOwnership::UserLlmProvider) | Some(AgentErrorOwnership::UnknownUpstream)
            );
            Some(TurnEvent::Failed { message: data.message, provider_fault })
        }
        // Tool cards, thinking, plans, tips: visible in the desktop UI, not
        // something a speaker can convey.
        _ => None,
    }
}
```

线程表用一个独立小文件（与设备注册表同目录，同样的 temp+rename 原子写）：

```rust
/// `{data_dir}/robot/threads.json` — `"{robot_id}|{companion_id}" -> conversation_id`.
///
/// A separate file from `robots.json` on purpose: rebinding a robot must not be
/// able to corrupt the device registry, and a lost thread map costs one new
/// conversation, not a re-pairing.
impl AppRobotBackend {
    fn threads_path(&self) -> std::path::PathBuf {
        self.data_dir.join("robot").join("threads.json")
    }

    async fn read_threads(&self) -> std::collections::BTreeMap<String, String> {
        match tokio::fs::read(self.threads_path()).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Default::default(),
        }
    }

    async fn lookup_thread(
        &self,
        robot_id: &str,
        companion_id: &str,
    ) -> anyhow::Result<Option<String>> {
        let key = format!("{robot_id}|{companion_id}");
        let Some(conversation_id) = self.read_threads().await.get(&key).cloned() else {
            return Ok(None);
        };
        // A conversation the user deleted must not be resurrected as a ghost id.
        match self.conversations.get(&self.owner_user_id, &conversation_id).await {
            Ok(_) => Ok(Some(conversation_id)),
            Err(_) => Ok(None),
        }
    }

    async fn record_thread(
        &self,
        robot_id: &str,
        companion_id: &str,
        conversation_id: &str,
    ) -> anyhow::Result<()> {
        let mut threads = self.read_threads().await;
        threads.insert(format!("{robot_id}|{companion_id}"), conversation_id.to_owned());
        let path = self.threads_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(&threads)?).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Point the conversation at the companion's fallback chat model for the
    /// retry. Changing the model kills and rebuilds the runtime
    /// (`AgentKillReason::ConfigurationChanged`), which is exactly what a retry
    /// after a provider outage wants: a fresh client against a different
    /// provider. The model is deliberately **left** on the fallback — silently
    /// flipping back would send the next turn straight into the same outage, and
    /// the UI shows the robot's model, so the state stays visible.
    async fn apply_fallback_model(&self, conversation_id: &str) -> anyhow::Result<()> {
        let companion_id = self
            .companion_of(conversation_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("conversation is not a robot thread"))?;
        let profile = self.companions.load_profile(&companion_id).await?;
        let fallback = profile
            .fallback_model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no fallback model configured"))?;
        tracing::warn!(
            conversation_id,
            provider_id = %fallback.provider_id,
            model = %fallback.model,
            "robot: switching this thread to the fallback model after a provider fault"
        );
        let request = UpdateConversationRequest {
            model: Some(fallback),
            ..Default::default()
        };
        self.conversations
            .update(&self.owner_user_id, conversation_id, request, &self.runtime_registry)
            .await?;
        Ok(())
    }

    /// The companion this robot thread belongs to, read from the conversation's
    /// own `extra` so it survives a restart with no in-memory state.
    async fn companion_of(&self, conversation_id: &str) -> Option<String> {
        let conversation = self.conversations.get(&self.owner_user_id, conversation_id).await.ok()?;
        conversation
            .extra
            .as_ref()?
            .get("companion_id")?
            .as_str()
            .map(str::to_owned)
    }
}
```

`AppRobotBackend` 因此还需要一个 `pub data_dir: std::path::PathBuf` 字段。`UpdateConversationRequest` 的 `model` 字段名与类型、`ConversationService::get` 的确切签名以 `grep -n "pub struct UpdateConversationRequest" -A 20 crates/backend/nomifun-api-types/src/conversation.rs` 与 `grep -n "pub async fn get" crates/backend/nomifun-conversation/src/service.rs` 的输出为准（语义不变：按 owner 取会话、按 owner 更新模型）。

`services.rs` 加字段并构造（照 `_managed_model_server` / `ssh_pool` 的姿态）：

```rust
    pub robot: Arc<nomifun_robot::routes::RobotAdminState>,
    pub(crate) _robot_proxy: Arc<nomifun_robot::mcp_proxy::RobotMcpProxyServer>,
    pub(crate) _robot_gateway_task: tokio::task::JoinHandle<()>,
```

`router/routes.rs`：在 `/mcp` 那一段旁边加

```rust
    // Robot device face: no session, no cookie, token in the Authorization
    // header. `nest` keeps its (absent) auth layer and fallback scoped, exactly
    // like /mcp. It rides both listeners because the router is shared.
    let router = router.nest("/robot", nomifun_robot::routes::device_router(robot_device_state));
```

并把 `nomifun_robot::routes::admin_router(...)` 用 `.merge(...)` 挂进 instance-owner 保护的那一层（与 `ssh_routes` 同一位置，见 `routes.rs:726-727, 1065`）。

`desktop.rs`：新增一个把 `WebUiStatus` 投射成 `LanEndpointSnapshot` 的转发任务——订阅 `subscribe_status()`，每次变化写入 `watch::Sender<LanEndpointSnapshot>`（`enabled` 取 LAN 是否在跑，`port` 取实际绑定端口，`ipv4s` 取 `detect_all_lan_ipv4s()`）。

`desktop.rs` 的关闭路径（`:976`/`:1006` 附近，`ssh_pool.shutdown_all()` 旁）加 `services._robot_proxy.stop();` 与 `services._robot_gateway_task.abort();`。

- [ ] **Step 6: 跑编译与目标包测试**

Run: `cargo check -p nomifun-app && cargo test -p nomifun-robot`
Expected: 编译通过；`nomifun-robot` 全部测试通过

- [ ] **Step 7: Commit**

```bash
git add crates/backend/nomifun-robot/ crates/backend/nomifun-app/ Cargo.toml Cargo.lock
git commit -m "feat(robot): add management REST face and host assembly"
```

---

### Task 22: 模拟设备集成测试

**Files:**
- Create: `crates/backend/nomifun-robot/tests/fake_device.rs`
- Modify: `crates/backend/nomifun-robot/Cargo.toml`（`[dev-dependencies]` 加 `tokio-tungstenite`）

**Interfaces:**
- Consumes: 全部公开面（`device_router`、`admin_router`、`RobotGateway`、`LanWsSource`、mock 服务）
- Produces: 一个完整扮演固件的测试客户端（不依赖真机、不依赖模型 provider）

- [ ] **Step 1: 写失败测试**

创建 `crates/backend/nomifun-robot/tests/fake_device.rs`：

```rust
//! End-to-end: a fake device walks the whole firmware flow against the real
//! gateway, with mocked speech and dispatch. Nothing here touches hardware or a
//! model provider, so it runs in CI-less local test loops.

use nomifun_robot::endpoint::LanEndpointSnapshot;
use nomifun_robot::registry::RobotRegistry;
use nomifun_robot::services::mock::{MockDispatcher, MockSpeech};
use nomifun_robot::services::TurnEvent;
use serde_json::Value;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

/// Boot the gateway on a real loopback port and return `(base_url, handles)`.
async fn boot() -> Harness {
    // Implementation note for the engineer: assemble exactly what Task 21's
    // `build_robot_services` does, but with `MockSpeech` / `MockDispatcher`, then
    // serve `device_router` + `admin_router` on a `TcpListener::bind("127.0.0.1:0")`
    // via `axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())`.
    // `into_make_service_with_connect_info` is required: the OTA handler reads
    // the peer IP to choose which interface to advertise.
    todo_harness().await
}

#[tokio::test]
async fn a_device_reports_gets_claimed_talks_and_is_interrupted() {
    let h = boot().await;

    // 1. OTA report: fresh device, so an activation code and a token come back.
    let ota: Value = h
        .http
        .post(format!("{}/robot/ota", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:ff")
        .header("Client-Id", "3f2b9c1e-0000-4000-8000-000000000001")
        .json(&serde_json::json!({
            "version": 2,
            "application": { "version": "1.9.0" },
            "board": { "type": "esp32-s3n16r8-emoji" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(ota.get("mqtt").is_none(), "an mqtt object would make the firmware pick MQTT");
    let token = ota["websocket"]["token"].as_str().unwrap().to_owned();
    let ws_url = ota["websocket"]["url"].as_str().unwrap().to_owned();
    let code = ota["activation"]["code"].as_str().unwrap().to_owned();

    // 2. Activation polling says 202 until a human claims it.
    let pending = h
        .http
        .post(format!("{}/robot/ota/activate", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:ff")
        .send()
        .await
        .unwrap();
    assert_eq!(pending.status(), 202);

    // 3. The UI claims the code for a companion.
    let claimed = h
        .http
        .post(format!("{}/api/robots/claim", h.base))
        .json(&serde_json::json!({ "code": code, "companion_id": "0190f5fe-7c00-7a00-8000-0000000000aa" }))
        .send()
        .await
        .unwrap();
    assert_eq!(claimed.status(), 200);

    let done = h
        .http
        .post(format!("{}/robot/ota/activate", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:ff")
        .send()
        .await
        .unwrap();
    assert_eq!(done.status(), 200);

    // 4. Connect the audio channel and handshake.
    h.speech.push_transcript("讲个故事");
    h.dispatcher.script_turn(vec![
        TurnEvent::Text("[emotion:happy] 从前有座山。".into()),
        TurnEvent::Text("山上有座庙。".into()),
        TurnEvent::Done,
    ]);

    let mut request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Device-Id", "aa:bb:cc:dd:ee:ff")
        .header("Client-Id", "3f2b9c1e-0000-4000-8000-000000000001")
        .header("Protocol-Version", "1")
        .body(())
        .unwrap();
    let (mut socket, _) = tokio_tungstenite::connect_async(std::mem::take(&mut request)).await.unwrap();

    send_text(
        &mut socket,
        r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true},"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}"#,
    )
    .await;
    let hello = next_json(&mut socket).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["audio_params"]["sample_rate"], 24000);
    assert_eq!(hello["audio_params"]["frame_duration"], 60);

    // 5. Speak: listen start, 300 ms of audio, then silence to end the turn.
    send_text(&mut socket, r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#).await;
    for packet in uplink_packets(300, true) {
        socket.send(Message::Binary(packet.into())).await.unwrap();
    }
    for packet in uplink_packets(900, false) {
        socket.send(Message::Binary(packet.into())).await.unwrap();
    }

    // 6. Expect the documented downlink sequence, and audio as bare binary.
    let mut seen_stt = false;
    let mut seen_emotion = false;
    let mut seen_sentences = 0;
    let mut audio_frames = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && seen_sentences < 2 {
        match next_frame(&mut socket).await {
            Message::Text(raw) => {
                let value: Value = serde_json::from_str(&raw).unwrap();
                match value["type"].as_str() {
                    Some("stt") => {
                        assert_eq!(value["text"], "讲个故事");
                        seen_stt = true;
                    }
                    Some("llm") => {
                        assert_eq!(value["emotion"], "happy");
                        seen_emotion = true;
                    }
                    Some("tts") if value["state"] == "sentence_start" => {
                        assert!(
                            !value["text"].as_str().unwrap().contains("[emotion:"),
                            "the marker must be stripped before it reaches the screen"
                        );
                        seen_sentences += 1;
                    }
                    _ => {}
                }
            }
            Message::Binary(_) => audio_frames += 1,
            _ => {}
        }
    }
    assert!(seen_stt && seen_emotion, "stt={seen_stt} emotion={seen_emotion}");
    assert_eq!(seen_sentences, 2);
    assert!(audio_frames > 0, "synthesised audio must arrive as binary frames");

    // 7. Interrupt: after abort, not one more frame may arrive.
    send_text(&mut socket, r#"{"session_id":"s","type":"abort","reason":"wake_word_detected"}"#).await;
    let mut saw_stop = false;
    let quiet_after = tokio::time::Instant::now() + std::time::Duration::from_millis(600);
    while tokio::time::Instant::now() < quiet_after {
        match tokio::time::timeout(std::time::Duration::from_millis(120), next_frame(&mut socket)).await {
            Ok(Message::Text(raw)) => {
                let value: Value = serde_json::from_str(&raw).unwrap();
                if value["type"] == "tts" && value["state"] == "stop" {
                    saw_stop = true;
                }
            }
            Ok(Message::Binary(_)) => {
                assert!(!saw_stop, "no audio may follow the tts stop that acknowledges an abort");
            }
            _ => {}
        }
    }
    assert!(saw_stop, "abort must be acknowledged with tts stop");

    // 8. Status is visible to the UI, and going away flips it to offline.
    let statuses: Value = h
        .http
        .get(format!("{}/api/robots/statuses", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(statuses["statuses"][0]["robot_id"], "aa:bb:cc:dd:ee:ff");

    drop(socket);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let statuses: Value = h
        .http
        .get(format!("{}/api/robots/statuses", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(statuses["statuses"][0]["phase"], "offline");
}

#[tokio::test]
async fn an_unclaimed_device_is_refused_a_session() {
    let h = boot().await;
    let ota: Value = h
        .http
        .post(format!("{}/robot/ota", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:02")
        .header("Client-Id", "cid")
        .json(&serde_json::json!({ "version": 2, "application": { "version": "1.9.0" }, "board": { "type": "b" } }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = ota["websocket"]["token"].as_str().unwrap().to_owned();
    let ws_url = ota["websocket"]["url"].as_str().unwrap().to_owned();

    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&ws_url)
        .header("Authorization", format!("Bearer {token}"))
        .body(())
        .unwrap();
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    send_text(&mut socket, r#"{"type":"hello","version":1,"transport":"websocket"}"#).await;
    // The gateway hangs up instead of issuing a session.
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), next_frame(&mut socket))
            .await
            .map(|m| !matches!(m, Message::Text(ref t) if t.contains("\"hello\"")))
            .unwrap_or(true),
        "an unbound robot must never receive a server hello"
    );
}

#[tokio::test]
async fn a_bad_token_cannot_open_the_websocket() {
    let h = boot().await;
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(format!("{}/robot/v1", h.base.replace("http://", "ws://")))
        .header("Authorization", "Bearer not-a-token")
        .body(())
        .unwrap();
    assert!(
        tokio_tungstenite::connect_async(request).await.is_err(),
        "the upgrade must be rejected with 401 before any frame is exchanged"
    );
}
```

**`boot()` 与三个小工具函数由本任务实现，不是占位符**：`todo_harness()` 只是这里为了先看到编译失败而写的名字，Step 3 会用真实实现替换它，`Harness` 结构、`send_text`、`next_json`、`next_frame`、`uplink_packets` 一并在同文件实现（`uplink_packets` 与 Task 14 测试里的同名函数逐字相同，复制过来即可——集成测试是独立 crate，不能引用单元测试模块）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-robot --test fake_device`
Expected: FAIL — `cannot find function todo_harness in this scope`

- [ ] **Step 3: 写实现**

在同文件补上：

```rust
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;

struct Harness {
    base: String,
    http: reqwest::Client,
    speech: Arc<MockSpeech>,
    dispatcher: Arc<MockDispatcher>,
    _dir: tempfile::TempDir,
}

async fn boot() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
    let speech = Arc::new(MockSpeech::new());
    let dispatcher = Arc::new(MockDispatcher::new());
    dispatcher.set_has_fallback(false);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // The advertiser must hand out this very port, or the device would be told
    // to connect somewhere nothing is listening.
    let (tx, rx) = tokio::sync::watch::channel(LanEndpointSnapshot {
        enabled: true,
        port,
        ipv4s: vec![std::net::Ipv4Addr::new(127, 0, 0, 1)],
    });
    std::mem::forget(tx);
    let advertiser: Arc<dyn nomifun_robot::endpoint::EndpointAdvertiser> =
        Arc::new(nomifun_robot::endpoint::LanAdvertiser::new(rx));

    let status = Arc::new(nomifun_robot::status::RobotStatusRegistry::new(
        nomifun_robot::events::RobotEventEmitter::new(Arc::new(NullSink)),
        "owner-1".to_owned(),
    ));
    let tools = Arc::new(nomifun_robot::tool_registry::RobotToolRegistry::default());
    let (source, acceptor) = nomifun_robot::lan_source::LanWsSource::new();

    let device_state = nomifun_robot::routes::RobotDeviceState {
        registry: registry.clone(),
        advertiser: advertiser.clone(),
        acceptor,
        speech: speech.clone(),
    };
    let admin_state = nomifun_robot::routes::RobotAdminState {
        registry: registry.clone(),
        status: status.clone(),
        advertiser: advertiser.clone(),
    };

    let gateway = Arc::new(nomifun_robot::RobotGateway::new(nomifun_robot::session::SessionDeps {
        registry,
        status,
        speech: speech.clone(),
        dispatcher: dispatcher.clone(),
        tools,
        vision_base: Some(format!("http://127.0.0.1:{port}")),
        device_token: String::new(),
    }));
    tokio::spawn(gateway.serve(vec![source]));

    let app = axum::Router::new()
        .nest("/robot", nomifun_robot::routes::device_router(device_state))
        .merge(nomifun_robot::routes::admin_router(admin_state));
    tokio::spawn(async move {
        // ConnectInfo is required: the OTA handler picks an interface by peer IP.
        let _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await;
    });

    Harness {
        base: format!("http://127.0.0.1:{port}"),
        http: reqwest::Client::new(),
        speech,
        dispatcher,
        _dir: dir,
    }
}

struct NullSink;
impl nomifun_realtime::UserEventSink for NullSink {
    fn send_to_user(
        &self,
        _user_id: &str,
        _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
    ) {
    }
}

type Socket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send_text(socket: &mut Socket, raw: &str) {
    socket.send(Message::Text(raw.into())).await.unwrap();
}

async fn next_frame(socket: &mut Socket) -> Message {
    loop {
        match socket.next().await {
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(message)) => return message,
            _ => return Message::Close(None),
        }
    }
}

async fn next_json(socket: &mut Socket) -> Value {
    loop {
        if let Message::Text(raw) = next_frame(socket).await {
            return serde_json::from_str(&raw).unwrap();
        }
    }
}

/// Same generator as the uplink unit tests: integration tests are a separate
/// crate and cannot reach into `#[cfg(test)]` modules.
fn uplink_packets(ms: u32, loud: bool) -> Vec<Vec<u8>> {
    let n = (16_000u64 * ms as u64 / 1000) as usize;
    let pcm: Vec<i16> = (0..n)
        .map(|i| {
            if !loud {
                return 0;
            }
            let t = i as f32 / 16_000.0;
            ((t * 300.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
        })
        .collect();
    nomifun_robot::audio::OpusStreamEncoder::new_uplink_for_test()
        .unwrap()
        .encode_frames(&pcm)
        .unwrap()
}
```

删掉 `todo_harness()` 那一行与它的调用（`boot()` 现在是真实实现）。`Cargo.toml` 的 `[dev-dependencies]` 加 `tokio-tungstenite = { workspace = true }`、`futures-util = { workspace = true }`；`[features]` 的 `test-support` 需在测试中启用——集成测试引用 `services::mock` 时用 `cargo test -p nomifun-robot --features test-support`，或把 `test-support` 加入 `[dev-dependencies]` 自引用（`nomifun-robot = { path = ".", features = ["test-support"] }`），取实现时先跑通的那条。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-robot --features test-support --test fake_device`
Expected: PASS — 3 个测试全过

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-robot/ Cargo.toml Cargo.lock
git commit -m "test(robot): add fake-device end-to-end integration test"
```

---

## 附：任务依赖与并发建议

```
Plan B Task 1（伙伴档案契约）─┐
Plan B Task 2（TTS 偏好）  ───┤
                              ├──► Plan A Task 20（真实接线）──► Plan A Task 21（装配）──► Plan A Task 22（集成测试）
Plan A Task 1..19 ────────────┘
```

- **Task 1 → 2 → 3/4/5 可并发**：注册表、协议词汇、帧抽象、端点广告器四者互不依赖（都只依赖 Task 1 的 crate 骨架）。
- **Task 6 依赖 2+5**（OTA 需要注册表与广告器）；**Task 7 独立**；**Task 8 依赖 3+4+6+7**。
- **Task 9 独立**（纯音频）；**Task 10 依赖 9**；**Task 11 依赖 10**；**Task 12 独立**。
- **Task 13 独立**（trait + mock）；**Task 14 依赖 9+10/11+3**；**Task 15 依赖 9+4**；**Task 16 依赖 8+12+13+14+15**。
- **Task 17 依赖 3+4**；**Task 18 依赖 17**；**Task 19 依赖 6+13**。
- **Task 20 依赖 13 + Plan B Task 1/2**；**Task 21 依赖 20 与 6/8/18/19**；**Task 22 依赖 21**。

三条线的并发安排：Plan B Task 1/2 最先落 main（解锁 Plan A Task 20）；Plan A Task 1-19 与 Plan B Task 3-15 完全并行（无共享文件）；Plan C 全程独立（不同仓库）。

**文件重叠**（需按顺序而非并行）：
- `crates/backend/nomifun-companion/src/profile.rs` —— 仅 Plan B Task 1 触碰。
- `crates/backend/nomifun-app/src/{services.rs, router/routes.rs, desktop.rs}` —— 仅 Plan A Task 21 触碰。
- `ui/src/common/adapter/ipcBridge.ts` —— Plan B Task 3（伙伴档案镜像）与 Task 12（机器人 API）都改，按任务号顺序执行即可。
- 根 `Cargo.toml` / `Cargo.lock` —— Plan A 多个任务追加依赖；串行执行本计划即无冲突，若与 Plan B 并行则各自 rebase 时优先保留双方新增行。

## 附：真机验收清单（全部任务完成后）

1. 机器人配网页高级设置填 `http://<桌面机 LAN IP>:25808/robot/ota`（从 UI 的「添加机器人」弹窗复制），重启设备。
2. 设备屏幕显示并朗读 6 位激活码 → 在伙伴「远程控制」Tab 的「机器人连接」输码 → 设备退出激活态。
3. 唤醒词唤醒 → 说一句话 → 观察：屏幕出现转写文字、表情变化、听到回复、回复播完自动回到聆听（auto 模式连续对话）。
4. 说话中途再次唤醒打断 → 声音必须**立即**停（无拖尾），屏幕回到待机。
5. 让伙伴「向左看」「抬头」「回中」→ 云台按限位动作（需 Plan C 的固件已烧录）。
6. 按住说话（manual 模式）→ 松手即结束一轮，播完回到待机而非继续聆听。
7. 拔掉设备电源 → UI 的机器人状态在数秒内变为离线；重新上电后自动恢复（无需重新绑定）。
