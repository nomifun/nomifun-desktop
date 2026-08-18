# nomifun-bridge-server 实施计划（Plan B）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付独立可部署的中继服务器 `nomifun-bridge-server`（类 rustdesk-server），按协议 v1 §3 盲转发 E2E 密文帧。

**Architecture:** 单二进制 Rust 服务：axum 0.8 提供 `/healthz` 与 `/ws`；每个 WS 连接先完成 HMAC 注册握手，再进入转发循环；内存注册表 `device_id → mpsc::Sender`，无持久化、不解密业务帧。

**Tech Stack:** Rust (edition 2024), axum 0.8 (ws), tokio, serde/serde_json, hmac+sha2+hex, clap 4, tracing；集成测试用 tokio-tungstenite。

## Global Constraints

- 规范文档（字段/常量/算法不得偏离）：`nomifun-tauri/docs/superpowers/specs/2026-08-03-bridge-protocol-v1.md` §3
- 默认监听 `0.0.0.0:21190`；auth = `hex(HMAC-SHA256(key=utf8(server_key), msg=utf8(id + ":" + ts)))`；时钟偏差 ±300_000ms
- 单帧 ≤ 64 KB；每连接 30 帧/秒；90s 空闲断开；同 id 重复注册顶替旧连接
- 仓库位置 `/home/developer/src/nomifun-bridge-server`（与 nomifun-tauri 同级）；git author/committer 必须是 `NomiFun Contributor <nomifun@users.noreply.github.com>`；提交信息不得含任何 AI 署名 trailer；禁止创建 `.github/workflows/`
- 测试命令统一 `cargo test`（本机内存有限，必要时 `-- --test-threads=2`）

---

### Task 1: 仓库脚手架

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `.gitignore`, `README.md`

**Interfaces:**
- Produces: 可编译空二进制 `nomifun-bridge-server`

- [ ] **Step 1: 创建项目**

```bash
mkdir -p /home/developer/src/nomifun-bridge-server && cd /home/developer/src/nomifun-bridge-server
git init -b main
git config user.name "NomiFun Contributor" && git config user.email "nomifun@users.noreply.github.com"
```

`Cargo.toml`:

```toml
[package]
name = "nomifun-bridge-server"
version = "0.1.0"
edition = "2024"
license = "MIT"

[dependencies]
axum = { version = "0.8", features = ["ws"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
futures-util = "0.3"

[dev-dependencies]
tokio-tungstenite = "0.26"
```

`src/main.rs`（暂时最小）:

```rust
fn main() {
    println!("nomifun-bridge-server");
}
```

`.gitignore`:

```
/target
```

`README.md` 先写一行标题 `# nomifun-bridge-server`（Task 7 完善）。

- [ ] **Step 2: 验证编译**

Run: `cargo check`
Expected: 无错误

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "chore: scaffold nomifun-bridge-server crate"
git log -1 --format='%an <%ae>'   # 必须输出 NomiFun Contributor <nomifun@users.noreply.github.com>
```

### Task 2: 控制帧协议类型（proto.rs）

**Files:**
- Create: `src/proto.rs`
- Modify: `src/main.rs`（`mod proto;`）

**Interfaces:**
- Produces: `pub enum ControlFrame { Register{role,id,ts,auth}, Registered, Forward{to,frame}, Deliver{from,frame}, Error{code,to}, Ping, Pong }`、`pub enum Role { Desktop, Mobile }`、`pub enum ErrorCode { AuthFailed, PeerOffline, RateLimited, TooLarge, BadFrame }`

- [ ] **Step 1: 写失败测试**（`src/proto.rs` 底部 `#[cfg(test)]`）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_roundtrip_matches_protocol_json() {
        let f: ControlFrame = serde_json::from_str(
            r#"{"type":"register","role":"desktop","id":"a1b2","ts":1700000000000,"auth":"deadbeef"}"#,
        )
        .unwrap();
        assert_eq!(
            f,
            ControlFrame::Register { role: Role::Desktop, id: "a1b2".into(), ts: 1_700_000_000_000, auth: "deadbeef".into() }
        );
    }

    #[test]
    fn error_serializes_snake_case_and_skips_none_to() {
        let s = serde_json::to_string(&ControlFrame::Error { code: ErrorCode::PeerOffline, to: Some("x".into()) }).unwrap();
        assert_eq!(s, r#"{"type":"error","code":"peer_offline","to":"x"}"#);
        let s = serde_json::to_string(&ControlFrame::Error { code: ErrorCode::AuthFailed, to: None }).unwrap();
        assert_eq!(s, r#"{"type":"error","code":"auth_failed"}"#);
    }

    #[test]
    fn forward_keeps_frame_opaque() {
        let f: ControlFrame = serde_json::from_str(r#"{"type":"forward","to":"b","frame":{"v":1,"c":"zz"}}"#).unwrap();
        let ControlFrame::Forward { frame, .. } = f else { panic!() };
        assert_eq!(frame["v"], 1);
    }

    #[test]
    fn ping_pong() {
        assert_eq!(serde_json::to_string(&ControlFrame::Pong).unwrap(), r#"{"type":"pong"}"#);
        assert!(matches!(serde_json::from_str::<ControlFrame>(r#"{"type":"ping"}"#).unwrap(), ControlFrame::Ping));
    }
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test proto`
Expected: FAIL（类型未定义）

- [ ] **Step 3: 最小实现**（`src/proto.rs` 顶部）

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Desktop,
    Mobile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AuthFailed,
    PeerOffline,
    RateLimited,
    TooLarge,
    BadFrame,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlFrame {
    Register { role: Role, id: String, ts: i64, auth: String },
    Registered,
    Forward { to: String, frame: Value },
    Deliver { from: String, frame: Value },
    Error {
        code: ErrorCode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    Ping,
    Pong,
}
```

`src/main.rs` 增加 `mod proto;`。

- [ ] **Step 4: 运行验证通过**

Run: `cargo test proto`
Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: relay control frame types"
```

### Task 3: 注册鉴权（auth.rs）

**Files:**
- Create: `src/auth.rs`
- Modify: `src/main.rs`（`mod auth;`）

**Interfaces:**
- Produces: `pub fn compute_auth(server_key: &str, id: &str, ts: i64) -> String`；`pub fn verify_auth(server_key: &str, id: &str, ts: i64, auth_hex: &str, now_ms: i64) -> bool`；`pub const MAX_SKEW_MS: i64 = 300_000`

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ok() {
        let a = compute_auth("k1", "dev1", 1000);
        assert_eq!(a.len(), 64); // hex(32B)
        assert!(verify_auth("k1", "dev1", 1000, &a, 1000));
    }

    #[test]
    fn wrong_key_or_id_or_ts_fails() {
        let a = compute_auth("k1", "dev1", 1000);
        assert!(!verify_auth("k2", "dev1", 1000, &a, 1000));
        assert!(!verify_auth("k1", "dev2", 1000, &a, 1000));
        assert!(!verify_auth("k1", "dev1", 1001, &a, 1001));
    }

    #[test]
    fn skew_beyond_300s_fails() {
        let a = compute_auth("k1", "dev1", 1000);
        assert!(verify_auth("k1", "dev1", 1000, &a, 1000 + MAX_SKEW_MS));
        assert!(!verify_auth("k1", "dev1", 1000, &a, 1000 + MAX_SKEW_MS + 1));
        assert!(!verify_auth("k1", "dev1", 1000, &a, 1000 - MAX_SKEW_MS - 1));
    }

    #[test]
    fn garbage_hex_fails() {
        assert!(!verify_auth("k1", "dev1", 1000, "not-hex!!", 1000));
    }
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test auth`
Expected: FAIL

- [ ] **Step 3: 最小实现**

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const MAX_SKEW_MS: i64 = 300_000;

pub fn compute_auth(server_key: &str, id: &str, ts: i64) -> String {
    let mut mac = HmacSha256::new_from_slice(server_key.as_bytes()).expect("hmac accepts any key length");
    mac.update(format!("{id}:{ts}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_auth(server_key: &str, id: &str, ts: i64, auth_hex: &str, now_ms: i64) -> bool {
    if (now_ms - ts).abs() > MAX_SKEW_MS {
        return false;
    }
    let Ok(given) = hex::decode(auth_hex) else { return false };
    let mut mac = HmacSha256::new_from_slice(server_key.as_bytes()).expect("hmac accepts any key length");
    mac.update(format!("{id}:{ts}").as_bytes());
    mac.verify_slice(&given).is_ok()
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test auth`
Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: hmac register auth with clock-skew window"
```

### Task 4: 连接注册表（state.rs）

**Files:**
- Create: `src/state.rs`
- Modify: `src/main.rs`（`mod state;`）

**Interfaces:**
- Produces:

```rust
pub struct AppState { pub server_key: String, pub registry: Registry }
pub struct Registry(/* Mutex<HashMap<String, Peer>> + AtomicU64 */);
impl Registry {
    pub fn new() -> Self;
    /// 注册并返回本连接的 conn_id；若同 id 已在线，向旧连接发送 Close 并顶替。
    pub async fn register(&self, id: &str, tx: mpsc::Sender<Message>) -> u64;
    /// 仅当仍是本连接持有该 id 时移除（防止顶替后误删新连接）。
    pub async fn remove_if(&self, id: &str, conn_id: u64);
    pub async fn sender_of(&self, id: &str) -> Option<mpsc::Sender<Message>>;
}
```

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::Message;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn register_lookup_remove() {
        let r = Registry::new();
        let (tx, _rx) = mpsc::channel::<Message>(4);
        let conn = r.register("a", tx).await;
        assert!(r.sender_of("a").await.is_some());
        r.remove_if("a", conn).await;
        assert!(r.sender_of("a").await.is_none());
    }

    #[tokio::test]
    async fn duplicate_register_replaces_and_closes_old() {
        let r = Registry::new();
        let (tx1, mut rx1) = mpsc::channel::<Message>(4);
        let c1 = r.register("a", tx1).await;
        let (tx2, _rx2) = mpsc::channel::<Message>(4);
        let c2 = r.register("a", tx2).await;
        assert_ne!(c1, c2);
        // 旧连接收到 Close 指令
        assert!(matches!(rx1.recv().await, Some(Message::Close(_))));
        // 旧连接退出时的清理不能误删新连接
        r.remove_if("a", c1).await;
        assert!(r.sender_of("a").await.is_some());
        r.remove_if("a", c2).await;
        assert!(r.sender_of("a").await.is_none());
    }
}
```

- [ ] **Step 2: 运行验证失败**

Run: `cargo test state`
Expected: FAIL

- [ ] **Step 3: 最小实现**

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::Message;
use tokio::sync::{mpsc, Mutex};

pub struct AppState {
    pub server_key: String,
    pub registry: Registry,
}

struct Peer {
    tx: mpsc::Sender<Message>,
    conn_id: u64,
}

pub struct Registry {
    inner: Mutex<HashMap<String, Peer>>,
    next_conn: AtomicU64,
}

impl Registry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()), next_conn: AtomicU64::new(1) }
    }

    pub async fn register(&self, id: &str, tx: mpsc::Sender<Message>) -> u64 {
        let conn_id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let mut map = self.inner.lock().await;
        if let Some(old) = map.insert(id.to_string(), Peer { tx, conn_id }) {
            let _ = old.tx.try_send(Message::Close(None));
        }
        conn_id
    }

    pub async fn remove_if(&self, id: &str, conn_id: u64) {
        let mut map = self.inner.lock().await;
        if map.get(id).is_some_and(|p| p.conn_id == conn_id) {
            map.remove(id);
        }
    }

    pub async fn sender_of(&self, id: &str) -> Option<mpsc::Sender<Message>> {
        self.inner.lock().await.get(id).map(|p| p.tx.clone())
    }
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test state`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: in-memory device registry with replace-on-reregister"
```

### Task 5: 会话循环 + HTTP 装配（session.rs / main.rs）

**Files:**
- Create: `src/session.rs`
- Modify: `src/main.rs`（clap CLI、router、`mod session;`）
- Test: `tests/integration.rs`

**Interfaces:**
- Consumes: `proto::ControlFrame`、`auth::verify_auth`、`state::{AppState, Registry}`
- Produces: `pub async fn handle_socket(socket: WebSocket, state: Arc<AppState>)`；`pub fn build_router(state: Arc<AppState>) -> axum::Router`（`GET /healthz`、`GET /ws`）；二进制入口 `nomifun-bridge-server --listen 0.0.0.0:21190 --key <key>`
- 常量：`pub const MAX_FRAME_BYTES: usize = 64 * 1024;`、`pub const MAX_FRAMES_PER_SEC: u32 = 30;`、`pub const IDLE_TIMEOUT_SECS: u64 = 90;`、`pub const REGISTER_TIMEOUT_SECS: u64 = 10;`

- [ ] **Step 1: 写集成测试**（`tests/integration.rs`；测试内起服务器于临时端口）

```rust
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message as WsMsg;

type Client = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const KEY: &str = "test-key";

async fn spawn_server() -> String {
    let state = Arc::new(nomifun_bridge_server::state::AppState {
        server_key: KEY.to_string(),
        registry: nomifun_bridge_server::state::Registry::new(),
    });
    let app = nomifun_bridge_server::session::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://{addr}/ws")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

async fn connect_registered(url: &str, id: &str) -> Client {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let ts = now_ms();
    let auth = nomifun_bridge_server::auth::compute_auth(KEY, id, ts);
    let reg = json!({"type":"register","role":"mobile","id":id,"ts":ts,"auth":auth});
    ws.send(WsMsg::Text(reg.to_string().into())).await.unwrap();
    let resp: Value = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(resp["type"], "registered");
    ws
}

async fn next_json(ws: &mut Client) -> Value {
    loop {
        match ws.next().await.unwrap().unwrap() {
            WsMsg::Text(t) => return serde_json::from_str(&t).unwrap(),
            WsMsg::Ping(_) | WsMsg::Pong(_) => continue,
            other => panic!("unexpected: {other:?}"),
        }
    }
}

#[tokio::test]
async fn register_and_forward_delivers_with_server_stamped_from() {
    let url = spawn_server().await;
    let mut desktop = connect_registered(&url, "desk01").await;
    let mut mobile = connect_registered(&url, "mob01").await;
    mobile
        .send(WsMsg::Text(json!({"type":"forward","to":"desk01","frame":{"v":1,"from":"mob01","n":"bm9uY2U=","c":"Y2lwaGVy"}}).to_string().into()))
        .await
        .unwrap();
    let got = next_json(&mut desktop).await;
    assert_eq!(got["type"], "deliver");
    assert_eq!(got["from"], "mob01"); // 由服务器按注册身份盖章，不可伪造
    assert_eq!(got["frame"]["c"], "Y2lwaGVy");
}

#[tokio::test]
async fn bad_auth_rejected() {
    let url = spawn_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let reg = json!({"type":"register","role":"mobile","id":"x","ts":now_ms(),"auth":"00"});
    ws.send(WsMsg::Text(reg.to_string().into())).await.unwrap();
    let resp: Value = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(resp["code"], "auth_failed");
    assert!(matches!(ws.next().await, Some(Ok(WsMsg::Close(_))) | None));
}

#[tokio::test]
async fn forward_to_offline_peer_reports_peer_offline() {
    let url = spawn_server().await;
    let mut mobile = connect_registered(&url, "mob02").await;
    mobile
        .send(WsMsg::Text(json!({"type":"forward","to":"ghost","frame":{"v":1}}).to_string().into()))
        .await
        .unwrap();
    let got = next_json(&mut mobile).await;
    assert_eq!(got["code"], "peer_offline");
    assert_eq!(got["to"], "ghost");
}

#[tokio::test]
async fn oversize_frame_errors_and_closes() {
    let url = spawn_server().await;
    let mut mobile = connect_registered(&url, "mob03").await;
    let big = "x".repeat(65 * 1024);
    mobile
        .send(WsMsg::Text(json!({"type":"forward","to":"a","frame":{"v":1,"c":big}}).to_string().into()))
        .await
        .unwrap();
    let got = next_json(&mut mobile).await;
    assert_eq!(got["code"], "too_large");
    assert!(matches!(mobile.next().await, Some(Ok(WsMsg::Close(_))) | None));
}

#[tokio::test]
async fn duplicate_register_replaces_old_connection() {
    let url = spawn_server().await;
    let mut old = connect_registered(&url, "dup01").await;
    let mut new = connect_registered(&url, "dup01").await;
    // 旧连接被服务器关闭
    assert!(matches!(old.next().await, Some(Ok(WsMsg::Close(_))) | None));
    // 新连接仍可收转发
    let mut sender = connect_registered(&url, "peer01").await;
    sender
        .send(WsMsg::Text(json!({"type":"forward","to":"dup01","frame":{"v":1}}).to_string().into()))
        .await
        .unwrap();
    assert_eq!(next_json(&mut new).await["type"], "deliver");
}

#[tokio::test]
async fn rate_limit_closes_connection() {
    let url = spawn_server().await;
    let mut flooder = connect_registered(&url, "flood01").await;
    // 两轮各 40 帧：即使首轮跨越 1s 窗口边界，第二轮也必然整体落入同一窗口
    for _ in 0..2 {
        for _ in 0..40 {
            let _ = flooder.send(WsMsg::Text(json!({"type":"ping"}).to_string().into())).await;
        }
    }
    let saw = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut saw = false;
        while let Some(Ok(msg)) = flooder.next().await {
            if let WsMsg::Text(t) = msg {
                let v: Value = serde_json::from_str(&t).unwrap();
                if v["code"] == "rate_limited" {
                    saw = true;
                }
            } else if matches!(msg, WsMsg::Close(_)) {
                break;
            }
        }
        saw
    })
    .await
    .unwrap_or(false);
    assert!(saw);
}

#[tokio::test]
async fn ping_gets_pong_and_healthz_ok() {
    let url = spawn_server().await;
    let mut c = connect_registered(&url, "p01").await;
    c.send(WsMsg::Text(json!({"type":"ping"}).to_string().into())).await.unwrap();
    assert_eq!(next_json(&mut c).await["type"], "pong");
    let http = url.replace("ws://", "http://").replace("/ws", "/healthz");
    let body = reqwest_free_get(&http).await;
    assert!(body.contains("\"ok\":true"));
}

// 避免引入 reqwest：用原始 TCP 发 HTTP/1.1
async fn reqwest_free_get(url: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let mut s = tokio::net::TcpStream::connect(host).await.unwrap();
    s.write_all(format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    buf
}
```

注意：集成测试要访问库代码，需把 crate 同时组织为 lib + bin —— 新增 `src/lib.rs`：

```rust
pub mod auth;
pub mod proto;
pub mod session;
pub mod state;
```

`src/main.rs` 改为使用 `nomifun_bridge_server::…`（去掉自身的 `mod` 声明）。

- [ ] **Step 2: 运行验证失败**

Run: `cargo test --test integration`
Expected: FAIL（`session::build_router` 未定义）

- [ ] **Step 3: 实现 session.rs**

```rust
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;

use crate::auth::verify_auth;
use crate::proto::{ControlFrame, ErrorCode};
use crate::state::AppState;

pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_FRAMES_PER_SEC: u32 = 30;
pub const IDLE_TIMEOUT_SECS: u64 = 90;
pub const REGISTER_TIMEOUT_SECS: u64 = 10;

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { axum::Json(json!({"ok": true})) }))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn text(frame: &ControlFrame) -> Message {
    Message::Text(serde_json::to_string(frame).unwrap().into())
}

pub async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Message>(32);

    // 写端任务：注册表/本会话都通过 tx 出帧；收到 Close 写出后结束。
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let is_close = matches!(msg, Message::Close(_));
            if sink.send(msg).await.is_err() || is_close {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // 注册握手（10s 超时）
    let registered = tokio::time::timeout(Duration::from_secs(REGISTER_TIMEOUT_SECS), stream.next()).await;
    let id = match registered {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str::<ControlFrame>(&t) {
            Ok(ControlFrame::Register { id, ts, auth, .. })
                if verify_auth(&state.server_key, &id, ts, &auth, now_ms()) =>
            {
                id
            }
            _ => {
                let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::AuthFailed, to: None })).await;
                let _ = tx.send(Message::Close(None)).await;
                let _ = writer.await;
                return;
            }
        },
        _ => {
            let _ = tx.send(Message::Close(None)).await;
            let _ = writer.await;
            return;
        }
    };

    let conn_id = state.registry.register(&id, tx.clone()).await;
    let _ = tx.send(text(&ControlFrame::Registered)).await;
    tracing::info!(device = %id, conn_id, "registered");

    let mut window_start = Instant::now();
    let mut window_count: u32 = 0;

    loop {
        let msg = match tokio::time::timeout(Duration::from_secs(IDLE_TIMEOUT_SECS), stream.next()).await {
            Ok(Some(Ok(m))) => m,
            _ => break, // 空闲超时 / 断开 / 协议错误
        };
        let payload = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Binary(_) => {
                let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::BadFrame, to: None })).await;
                continue;
            }
        };

        // 限速：滑动 1s 窗口
        if window_start.elapsed() >= Duration::from_secs(1) {
            window_start = Instant::now();
            window_count = 0;
        }
        window_count += 1;
        if window_count > MAX_FRAMES_PER_SEC {
            let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::RateLimited, to: None })).await;
            let _ = tx.send(Message::Close(None)).await;
            break;
        }

        if payload.len() > MAX_FRAME_BYTES {
            let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::TooLarge, to: None })).await;
            let _ = tx.send(Message::Close(None)).await;
            break;
        }

        match serde_json::from_str::<ControlFrame>(&payload) {
            Ok(ControlFrame::Ping) => {
                let _ = tx.send(text(&ControlFrame::Pong)).await;
            }
            Ok(ControlFrame::Pong) => {}
            Ok(ControlFrame::Forward { to, frame }) => {
                if !frame.is_object() {
                    let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::BadFrame, to: None })).await;
                    continue;
                }
                match state.registry.sender_of(&to).await {
                    Some(peer_tx) => {
                        let deliver = ControlFrame::Deliver { from: id.clone(), frame };
                        if peer_tx.send(text(&deliver)).await.is_err() {
                            let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::PeerOffline, to: Some(to) })).await;
                        }
                    }
                    None => {
                        let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::PeerOffline, to: Some(to) })).await;
                    }
                }
            }
            _ => {
                let _ = tx.send(text(&ControlFrame::Error { code: ErrorCode::BadFrame, to: None })).await;
            }
        }
    }

    state.registry.remove_if(&id, conn_id).await;
    drop(tx);
    let _ = writer.await;
    tracing::info!(device = %id, conn_id, "disconnected");
}
```

`src/main.rs`:

```rust
use std::sync::Arc;

use clap::Parser;
use nomifun_bridge_server::session::build_router;
use nomifun_bridge_server::state::{AppState, Registry};

#[derive(Parser)]
#[command(name = "nomifun-bridge-server", about = "E2E-encrypted relay for NomiFun mobile <-> desktop")]
struct Args {
    /// Listen address
    #[arg(long, default_value = "0.0.0.0:21190")]
    listen: String,
    /// Shared server key (clients must present matching HMAC)
    #[arg(long, env = "NOMIFUN_BRIDGE_KEY")]
    key: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args = Args::parse();
    let state = Arc::new(AppState { server_key: args.key, registry: Registry::new() });
    let listener = tokio::net::TcpListener::bind(&args.listen).await.expect("bind listen address");
    tracing::info!(addr = %args.listen, "nomifun-bridge-server listening");
    axum::serve(listener, build_router(state)).await.expect("server error");
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cargo test -- --test-threads=2`
Expected: 单测 + 7 个集成测试全部 PASS

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: ws session loop with register auth, blind forward, limits"
```

### Task 6: Docker 与 README

**Files:**
- Create: `Dockerfile`, `docker-compose.yml`, `.dockerignore`
- Modify: `README.md`

**Interfaces:**
- Produces: `docker compose up -d` 一键部署

- [ ] **Step 1: 写 Dockerfile**

```dockerfile
FROM rust:1.97-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN useradd -r -s /usr/sbin/nologin bridge
COPY --from=build /src/target/release/nomifun-bridge-server /usr/local/bin/
USER bridge
EXPOSE 21190
ENTRYPOINT ["nomifun-bridge-server"]
CMD ["--listen", "0.0.0.0:21190"]
```

`.dockerignore`:

```
target
.git
```

`docker-compose.yml`:

```yaml
services:
  bridge:
    build: .
    restart: unless-stopped
    ports:
      - "21190:21190"
    environment:
      NOMIFUN_BRIDGE_KEY: ${NOMIFUN_BRIDGE_KEY:?set a strong shared key}
```

- [ ] **Step 2: README 完整化**（中文；包含：功能简介、协议指针（指向 nomifun-tauri 仓库协议文档路径）、裸机运行 `cargo run --release -- --key <key>`、Docker 运行、`/healthz` 探活、安全说明：服务器无法解密转发内容、key 只防滥用不参与 E2E）

- [ ] **Step 3: 验证构建（可选，若本机有 docker）**

Run: `docker build -t nomifun-bridge-server . || echo "docker 不可用，跳过"`
Expected: 构建成功或明确跳过

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "chore: dockerfile, compose and deployment docs"
```

### Task 7: 收尾验证

- [ ] **Step 1: 全量测试** — Run: `cargo test -- --test-threads=2`；Expected: 全 PASS
- [ ] **Step 2: clippy** — Run: `cargo clippy -- -D warnings`；修复所有告警后重跑
- [ ] **Step 3: 确认无 GitHub Actions** — Run: `ls .github/workflows 2>/dev/null; true`；Expected: 不存在
- [ ] **Step 4: 确认作者合规** — Run: `git log --format='%an <%ae>' | sort -u`；Expected: 仅 `NomiFun Contributor <nomifun@users.noreply.github.com>`
- [ ] **Step 5: Commit（如有 clippy 修复）**

```bash
git add -A && git commit -m "chore: clippy fixes" || true
```
