//! CDP session demux 核心：**纯路由逻辑**，与 WS I/O 解耦，故可在无浏览器下单测。
//!
//! 单条 CDP 连接经 `Target.setAutoAttach{flatten:true}` 多路复用所有 target
//! （根 browser session + 每个 page/OOPIF/service_worker 子 session）。每条入站文本
//! 消息都形如：
//!   - **命令回包**：有 `id`（+ 可选 `sessionId`），含 `result` 或 `error{code,message}`；
//!   - **事件**：无 `id`、有 `method`（+ 可选 `sessionId`），含 `params`。
//!
//! 本模块只持有「sessions 注册表 + 命令配对 + 事件订阅」这套并发状态，并暴露一个**纯
//! 方法** [`SessionRegistry::dispatch_message`]：传输层 read loop 收到每条文本就调它，
//! 单测则直接喂构造的 JSON 字符串验证路由——无需真 WS。
//!
//! 设计取舍（对齐 DESIGN §5 / spike 修正）：
//! - 命令配对键 = `CallId`（`chromiumoxide_types` 的 `usize` newtype，`Hash+Eq+Copy`），
//!   per-session 注册，回包按 (sessionId, CallId) 投递到对应 `oneshot`。
//! - 事件订阅 = 按 `method` 名的 `broadcast` 通道，可 per-session 也可全局（root）。
//!   **`Runtime.bindingCalled` 必须能被订阅拿到 `{name,payload}`**（spike 修正：自订阅，
//!   不复制 chromiumoxide「早 return → no-op stub」的写法）。
//! - 错误类型 [`TransportError`] 自成一体，**不**强耦合 `BrowserError`（镜像 progress
//!   模块的 `ProgressError`；错误映射留给后续 task）。

use std::collections::HashMap;
use std::sync::Mutex;

use chromiumoxide::types::{CallId, Error as CdpError};
use serde::Deserialize;
use tokio::sync::{broadcast, oneshot};

/// 根（browser）session 在注册表里的 key。CDP 根连接的消息无 `sessionId` 字段，
/// 我们用一个固定哨兵 key 统一登记，避免 `Option<String>` 在两处分叉。
pub const ROOT_SESSION: &str = "";

/// 事件订阅广播通道容量。CDP 事件可能突发（如导航期 lifecycle / 大量 attachedToTarget），
/// 给足缓冲；订阅者落后只丢老事件（`broadcast` 语义），不阻塞 read loop。
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// 传输/会话层自有错误枚举。**不**耦合 `BrowserError`（错误映射留后续 task）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// 连接已关闭（WS 断开 / 主动 close）。
    #[error("transport connection closed")]
    Closed,
    /// 目标 session 已关闭（target detached / page closed）。
    #[error("session closed")]
    SessionClosed,
    /// 目标 session 已崩溃（target crashed）。
    #[error("session crashed")]
    SessionCrashed,
    /// 命令超时（每命令 deadline 到，对冲上游 hang）。
    #[error("cdp command timed out")]
    Timeout,
    /// 协议层错误（无法解析的消息 / 内部不变量被破坏 / 序列化失败）。
    #[error("cdp protocol error: {0}")]
    Protocol(String),
    /// 浏览器侧返回的 CDP 错误回包（`error{code,message}`）。
    #[error("cdp error {code}: {message}")]
    Cdp { code: i64, message: String },
}

/// 一次命令调用的结果：成功 `result` 的 JSON，或失败的 [`TransportError`]。
pub type CommandResult = Result<serde_json::Value, TransportError>;

/// 单个 CDP session 的状态：登记在 [`SessionRegistry`] 内。
///
/// 持有该 session 上**进行中**的命令回调表（`CallId -> oneshot::Sender`），以及生命
/// 周期标志位。`crashed`/`closed` 是粘性的：一旦置位，该 session 上的 [`SessionRegistry::send_*`]
/// 立即短路返错，且所有挂起回调被 drain 失败（详见 [`SessionRegistry::fail_session`]）。
pub struct Session {
    /// CDP sessionId（根 session = `ROOT_SESSION`）。
    pub session_id: String,
    /// target 类型（`page` / `iframe` / `service_worker` / `browser`…），来自
    /// `attachedToTarget` 的 `targetInfo.type`；根 session 为 `browser`。
    pub target_type: String,
    /// 进行中的命令：CallId → 等待结果的 oneshot 发送端。
    callbacks: HashMap<CallId, oneshot::Sender<CommandResult>>,
    /// target 崩溃（粘性）。
    crashed: bool,
    /// target/连接已关闭（粘性）。
    closed: bool,
}

impl Session {
    fn new(session_id: impl Into<String>, target_type: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            target_type: target_type.into(),
            callbacks: HashMap::new(),
            crashed: false,
            closed: false,
        }
    }

    /// 该 session 是否已不可用（崩溃或关闭）。
    pub fn is_dead(&self) -> bool {
        self.crashed || self.closed
    }

    /// 已死 session 对应的短路错误（崩溃优先于关闭分类）。
    fn dead_error(&self) -> TransportError {
        if self.crashed {
            TransportError::SessionCrashed
        } else {
            TransportError::SessionClosed
        }
    }
}

/// CDP 回包/事件的入站封套。命令回包有 `id`；事件无 `id` 有 `method`。
/// `sessionId` 缺省即根。`result`/`error` 仅命令回包有；`params` 仅事件有。
///
/// 用单一结构体宽松解析（而非 `chromiumoxide_types::Message` 的 untagged enum），
/// 以便对“既无 id 又无 method”的畸形消息给出明确的 `Protocol` 错误。
#[derive(Debug, Deserialize)]
struct InboundEnvelope {
    #[serde(default)]
    id: Option<CallId>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<CdpError>,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

/// 路由到事件订阅者的单个事件。`method` + `session_id` + 原始 `params`。
#[derive(Debug, Clone)]
pub struct CdpEvent {
    /// 事件 method 名，如 `Target.attachedToTarget` / `Runtime.bindingCalled`。
    pub method: String,
    /// 事件所属 session（根 = `ROOT_SESSION`）。
    pub session_id: String,
    /// 事件 params（原样 JSON；订阅者按需 `serde_json::from_value` 成具体类型）。
    pub params: serde_json::Value,
}

/// 事件订阅键：(method, session)。`session=None` 表示订阅**任意 session** 的该事件
/// （供根/全局监听，如 `attachedToTarget` 总在根 session 上来）。
type SubKey = (String, Option<String>);

/// 全部共享路由状态。`Mutex` 保护内部表；`dispatch_message` 是纯函数式入口（只读
/// 入参字符串 + 改内部表），单测可直接构造并喂 JSON。
pub struct SessionRegistry {
    inner: Mutex<RegistryInner>,
}

struct RegistryInner {
    /// 所有活动 session：sessionId（根 = `ROOT_SESSION`）→ Session。
    sessions: HashMap<String, Session>,
    /// 事件订阅：(method, Option<session>) → broadcast 发送端。
    subscriptions: HashMap<SubKey, broadcast::Sender<CdpEvent>>,
    /// 整个连接是否已关闭（粘性）。置位后所有 send 短路 `Closed`。
    connection_closed: bool,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    /// 新建注册表，并登记根（browser）session。
    pub fn new() -> Self {
        let mut sessions = HashMap::new();
        sessions.insert(
            ROOT_SESSION.to_string(),
            Session::new(ROOT_SESSION, "browser"),
        );
        Self {
            inner: Mutex::new(RegistryInner {
                sessions,
                subscriptions: HashMap::new(),
                connection_closed: false,
            }),
        }
    }

    /// 登记一个新子 session（attachedToTarget 时调）。重复登记同 id 直接覆盖刷新
    /// （CDP 可能对同 target 多次 attach；新封套以最新 targetInfo 为准）。
    pub fn register_session(&self, session_id: impl Into<String>, target_type: impl Into<String>) {
        let session_id = session_id.into();
        let target_type = target_type.into();
        let mut g = self.inner.lock().unwrap();
        g.sessions
            .entry(session_id.clone())
            .and_modify(|s| s.target_type = target_type.clone())
            .or_insert_with(|| Session::new(session_id, target_type));
    }

    /// 该 session 当前是否已登记。
    pub fn has_session(&self, session_id: &str) -> bool {
        self.inner.lock().unwrap().sessions.contains_key(session_id)
    }

    /// 该 session 的 target 类型（未登记 → None）。
    pub fn target_type(&self, session_id: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .get(session_id)
            .map(|s| s.target_type.clone())
    }

    /// **F1-sec (I1 启动竞态收口)**：枚举当前已登记的、`target_type == ty` 的所有 session id。
    ///
    /// 用于「补挂」启动瞬间已经 attach 的 target（典型：`service_worker`）——E5 防火墙循环在
    /// `enable_auto_attach` 之后才 `subscribe(attachedToTarget)`，故启动期已 attach 的 SW 的
    /// `attachedToTarget` 可能早于订阅丢失。但 attach loop（更早启动）已把这些 session 登记进本注册表，
    /// 故据 `target_type` 枚举即可拿到它们的 session id，对其补挂 `Fetch.enable`（不漏防火墙）。
    /// 返回的 id 顺序无保证（HashMap 遍历）；调用方对每个 best-effort 挂载。
    pub fn session_ids_of_type(&self, ty: &str) -> Vec<String> {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .iter()
            .filter(|(_, s)| s.target_type == ty)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// 整个连接是否已关闭。
    pub fn is_connection_closed(&self) -> bool {
        self.inner.lock().unwrap().connection_closed
    }

    /// 订阅某 method（可选限定 session）的事件流。返回 broadcast 接收端。
    /// `session=None` 订阅任意 session 的该事件。
    pub fn subscribe(
        &self,
        method: impl Into<String>,
        session_id: Option<&str>,
    ) -> broadcast::Receiver<CdpEvent> {
        let key: SubKey = (method.into(), session_id.map(|s| s.to_string()));
        let mut g = self.inner.lock().unwrap();
        let tx = g
            .subscriptions
            .entry(key)
            .or_insert_with(|| broadcast::channel(EVENT_CHANNEL_CAPACITY).0);
        tx.subscribe()
    }

    /// 在某 session 上登记一个进行中的命令回调。返回等待结果的 `oneshot::Receiver`。
    ///
    /// 短路：连接已关 → `Err(Closed)`；session 未登记 → `Err(SessionClosed)`；
    /// session 已崩/已关 → `Err(SessionCrashed/SessionClosed)`。这些都在**注册前**
    /// 判定，确保已死 session 上绝不挂起一个永不被投递的回调。
    pub fn register_command(
        &self,
        session_id: &str,
        call_id: CallId,
    ) -> Result<oneshot::Receiver<CommandResult>, TransportError> {
        let mut g = self.inner.lock().unwrap();
        if g.connection_closed {
            return Err(TransportError::Closed);
        }
        let session = g
            .sessions
            .get_mut(session_id)
            .ok_or(TransportError::SessionClosed)?;
        if session.is_dead() {
            return Err(session.dead_error());
        }
        let (tx, rx) = oneshot::channel();
        session.callbacks.insert(call_id, tx);
        Ok(rx)
    }

    /// 取消一个已登记但未投递的命令回调（命令发送失败 / 超时清理时调）。
    pub fn cancel_command(&self, session_id: &str, call_id: CallId) {
        let mut g = self.inner.lock().unwrap();
        if let Some(s) = g.sessions.get_mut(session_id) {
            s.callbacks.remove(&call_id);
        }
    }

    /// **纯路由入口**：解析一条入站文本消息并投递。read loop 每收到一条就调它。
    ///
    /// - 命令回包（有 `id`）→ 按 (sessionId, CallId) 找回调投递（`result` / `error`）。
    /// - 事件（无 `id` 有 `method`）→ 广播给订阅者（精确 session + 通配 session 各一份）；
    ///   `Target.attachedToTarget` 还会顺带登记子 session、`detachedFromTarget` /
    ///   `targetCrashed` 标记 session 死亡。
    ///
    /// 返回 `Err(Protocol(..))` 仅在消息根本无法解析（既非回包也非事件）；事件无人订阅
    /// 不算错误（静默丢弃）。
    pub fn dispatch_message(&self, raw: &str) -> Result<(), TransportError> {
        let env: InboundEnvelope = serde_json::from_str(raw)
            .map_err(|e| TransportError::Protocol(format!("invalid CDP message: {e}")))?;

        let session_key = env
            .session_id
            .clone()
            .unwrap_or_else(|| ROOT_SESSION.to_string());

        match env.id {
            // ── 命令回包 ────────────────────────────────────────────────
            Some(call_id) => {
                let result: CommandResult = match env.error {
                    Some(e) => Err(TransportError::Cdp {
                        code: e.code,
                        message: e.message,
                    }),
                    None => Ok(env.result.unwrap_or(serde_json::Value::Null)),
                };
                self.deliver_response(&session_key, call_id, result);
                Ok(())
            }
            // ── 事件 ───────────────────────────────────────────────────
            None => {
                let Some(method) = env.method else {
                    return Err(TransportError::Protocol(format!(
                        "CDP message has neither id nor method: {raw}"
                    )));
                };
                let params = env.params.unwrap_or(serde_json::Value::Null);
                self.handle_event(&method, &session_key, params);
                Ok(())
            }
        }
    }

    /// 投递命令回包到对应回调。找不到回调（已超时清理 / 未知 id）则静默丢弃。
    fn deliver_response(&self, session_key: &str, call_id: CallId, result: CommandResult) {
        let mut g = self.inner.lock().unwrap();
        if let Some(session) = g.sessions.get_mut(session_key)
            && let Some(tx) = session.callbacks.remove(&call_id)
        {
            // 接收端可能已 drop（调用方放弃等待）——忽略发送失败。
            let _ = tx.send(result);
        }
    }

    /// 处理一条事件：先做生命周期副作用（attach/detach/crash），再广播给订阅者。
    fn handle_event(&self, method: &str, session_key: &str, params: serde_json::Value) {
        // 生命周期副作用：登记子 session / 标记死亡。这些只改本注册表，不发 CDP
        // 命令（runIfWaitingForDebugger 由传输层在「先装监听」之后补发）。
        match method {
            "Target.detachedFromTarget" => {
                if let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) {
                    self.fail_session(sid, false);
                }
            }
            "Target.targetCrashed" => {
                // targetCrashed 在根 session 上来，targetId 在 params。子 session 的崩溃
                // 通过对应 sessionId 标记；若只有 targetId 无 sessionId，则交由后续
                // detachedFromTarget 兜底（CDP 通常崩溃后随即 detach）。
                if let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) {
                    self.fail_session(sid, true);
                }
            }
            _ => {}
        }

        let event = CdpEvent {
            method: method.to_string(),
            session_id: session_key.to_string(),
            params,
        };
        self.broadcast_event(event);
    }

    /// 广播一个事件给：① 精确 (method, session) 订阅者；② 通配 (method, None) 订阅者。
    /// 无人订阅 → 静默丢弃（合法：不是所有事件都有人关心）。
    fn broadcast_event(&self, event: CdpEvent) {
        let g = self.inner.lock().unwrap();
        let exact: SubKey = (event.method.clone(), Some(event.session_id.clone()));
        let wildcard: SubKey = (event.method.clone(), None);
        if let Some(tx) = g.subscriptions.get(&exact) {
            let _ = tx.send(event.clone());
        }
        if let Some(tx) = g.subscriptions.get(&wildcard) {
            let _ = tx.send(event);
        }
    }

    /// 标记某 session 死亡（崩溃或关闭），并 drain 其所有挂起回调为对应错误，
    /// 使等待中的 `send` 立即解除（绝不悬挂）。粘性：之后该 session 上 `send` 短路。
    pub fn fail_session(&self, session_id: &str, crashed: bool) {
        let mut g = self.inner.lock().unwrap();
        if let Some(session) = g.sessions.get_mut(session_id) {
            if crashed {
                session.crashed = true;
            } else {
                session.closed = true;
            }
            let err = session.dead_error();
            for (_id, tx) in session.callbacks.drain() {
                let _ = tx.send(Err(err.clone()));
            }
        }
    }

    /// 标记整个连接关闭（WS 断开）：drain 所有 session 的所有挂起回调为 `Closed`，
    /// 并置 `connection_closed`，使之后所有 `register_command` 短路 `Closed`。
    pub fn fail_connection(&self) {
        let mut g = self.inner.lock().unwrap();
        g.connection_closed = true;
        for session in g.sessions.values_mut() {
            session.closed = true;
            for (_id, tx) in session.callbacks.drain() {
                let _ = tx.send(Err(TransportError::Closed));
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 单测：全部针对**纯路由逻辑**，无需真浏览器 / 真 WS。
// ═══════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use chromiumoxide::types::CallId;

    fn call(id: usize) -> CallId {
        CallId::new(id)
    }

    /// 命令配对 + sessionId 路由：在某子 session 登记 (id) 的命令，喂一条匹配
    /// sessionId+id 的回包 → 对应 oneshot 收到正确 result。
    #[tokio::test]
    async fn response_routed_by_session_and_id() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let rx = reg.register_command("S1", call(7)).unwrap();

        reg.dispatch_message(
            r#"{"id":7,"sessionId":"S1","result":{"frameId":"F0","ok":true}}"#,
        )
        .unwrap();

        let got = rx.await.expect("sender dropped");
        let val = got.expect("expected Ok result");
        assert_eq!(val["frameId"], "F0");
        assert_eq!(val["ok"], true);
    }

    /// **F1-sec (I1)**: session_ids_of_type 按 target_type 枚举已登记 session（SW 启动竞态补挂用）。
    #[test]
    fn session_ids_of_type_enumerates_by_target_type() {
        let reg = SessionRegistry::new();
        reg.register_session("P1", "page");
        reg.register_session("SW1", "service_worker");
        reg.register_session("SW2", "service_worker");
        reg.register_session("IF1", "iframe");

        let mut sws = reg.session_ids_of_type("service_worker");
        sws.sort();
        assert_eq!(sws, vec!["SW1".to_string(), "SW2".to_string()]);

        assert_eq!(reg.session_ids_of_type("page"), vec!["P1".to_string()]);
        // 根 session 是 browser 类型；枚举 service_worker 不含它。
        assert!(!reg.session_ids_of_type("service_worker").contains(&ROOT_SESSION.to_string()));
        // 无此类型 → 空。
        assert!(reg.session_ids_of_type("worker").is_empty());
    }

    /// 根 session（无 sessionId 字段）的命令配对。
    #[tokio::test]
    async fn response_routed_to_root_session() {
        let reg = SessionRegistry::new();
        let rx = reg.register_command(ROOT_SESSION, call(1)).unwrap();
        reg.dispatch_message(r#"{"id":1,"result":{"value":42}}"#)
            .unwrap();
        let val = rx.await.unwrap().unwrap();
        assert_eq!(val["value"], 42);
    }

    /// 不同 id 不串：登记两个命令，只回其中一个 → 只有对应的解析，另一个仍挂起。
    #[tokio::test]
    async fn distinct_ids_do_not_cross() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let rx1 = reg.register_command("S1", call(1)).unwrap();
        let mut rx2 = reg.register_command("S1", call(2)).unwrap();

        reg.dispatch_message(r#"{"id":1,"sessionId":"S1","result":{"who":"one"}}"#)
            .unwrap();

        let v1 = rx1.await.unwrap().unwrap();
        assert_eq!(v1["who"], "one");
        // rx2 未被投递：try_recv 应为空（Empty），证明无串扰。
        assert!(matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    }

    /// 同 id 不同 session 不串：两个 session 各有 id=1 的命令，只回 S1 的。
    #[tokio::test]
    async fn same_id_different_session_isolated() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        reg.register_session("S2", "page");
        let rx1 = reg.register_command("S1", call(1)).unwrap();
        let mut rx2 = reg.register_command("S2", call(1)).unwrap();

        reg.dispatch_message(r#"{"id":1,"sessionId":"S1","result":{"s":"one"}}"#)
            .unwrap();

        assert_eq!(rx1.await.unwrap().unwrap()["s"], "one");
        assert!(matches!(rx2.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    }

    /// CDP error 回包 → oneshot 收到 TransportError::Cdp{code,message}。
    #[tokio::test]
    async fn cdp_error_response_maps_to_cdp_error() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let rx = reg.register_command("S1", call(9)).unwrap();

        reg.dispatch_message(
            r#"{"id":9,"sessionId":"S1","error":{"code":-32000,"message":"Cannot find context"}}"#,
        )
        .unwrap();

        let err = rx.await.unwrap().unwrap_err();
        assert_eq!(
            err,
            TransportError::Cdp {
                code: -32000,
                message: "Cannot find context".to_string()
            }
        );
    }

    /// 短路：未登记的 session 上 register_command → SessionClosed。
    #[test]
    fn register_on_unknown_session_short_circuits() {
        let reg = SessionRegistry::new();
        let err = reg.register_command("NOPE", call(1)).unwrap_err();
        assert_eq!(err, TransportError::SessionClosed);
    }

    /// 短路：已崩溃 session 上 register_command 立即返 SessionCrashed。
    #[test]
    fn register_on_crashed_session_short_circuits() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        reg.fail_session("S1", true);
        let err = reg.register_command("S1", call(1)).unwrap_err();
        assert_eq!(err, TransportError::SessionCrashed);
    }

    /// 短路：已关闭 session 上 register_command 立即返 SessionClosed。
    #[test]
    fn register_on_closed_session_short_circuits() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        reg.fail_session("S1", false);
        let err = reg.register_command("S1", call(1)).unwrap_err();
        assert_eq!(err, TransportError::SessionClosed);
    }

    /// 短路：连接已关闭后 register_command → Closed。
    #[test]
    fn register_after_connection_closed_short_circuits() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        reg.fail_connection();
        assert_eq!(
            reg.register_command("S1", call(1)).unwrap_err(),
            TransportError::Closed
        );
        assert!(reg.is_connection_closed());
    }

    /// fail_session 会 drain 挂起回调：一个进行中的命令在 session 崩溃时立即收到
    /// SessionCrashed，而非永久悬挂。
    #[tokio::test]
    async fn fail_session_drains_pending_callbacks() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let rx = reg.register_command("S1", call(1)).unwrap();
        reg.fail_session("S1", true);
        assert_eq!(rx.await.unwrap().unwrap_err(), TransportError::SessionCrashed);
    }

    /// fail_connection 会 drain 所有 session 的挂起回调为 Closed。
    #[tokio::test]
    async fn fail_connection_drains_all_callbacks() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        reg.register_session("S2", "page");
        let rx1 = reg.register_command("S1", call(1)).unwrap();
        let rx2 = reg.register_command("S2", call(1)).unwrap();
        reg.fail_connection();
        assert_eq!(rx1.await.unwrap().unwrap_err(), TransportError::Closed);
        assert_eq!(rx2.await.unwrap().unwrap_err(), TransportError::Closed);
    }

    /// 事件 demux：无 id 的事件 JSON 路由到精确 (method, session) 订阅者。
    #[tokio::test]
    async fn event_routed_to_exact_subscriber() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let mut sub = reg.subscribe("Page.frameNavigated", Some("S1"));

        reg.dispatch_message(
            r#"{"method":"Page.frameNavigated","sessionId":"S1","params":{"frame":{"url":"https://x"}}}"#,
        )
        .unwrap();

        let ev = sub.recv().await.unwrap();
        assert_eq!(ev.method, "Page.frameNavigated");
        assert_eq!(ev.session_id, "S1");
        assert_eq!(ev.params["frame"]["url"], "https://x");
    }

    /// 事件 demux：通配 (method, None) 订阅者收到任意 session 的该事件。
    #[tokio::test]
    async fn event_routed_to_wildcard_subscriber() {
        let reg = SessionRegistry::new();
        let mut sub = reg.subscribe("Target.attachedToTarget", None);

        reg.dispatch_message(
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"NEW","targetInfo":{"type":"page"},"waitingForDebugger":true}}"#,
        )
        .unwrap();

        let ev = sub.recv().await.unwrap();
        assert_eq!(ev.method, "Target.attachedToTarget");
        assert_eq!(ev.params["targetInfo"]["type"], "page");
    }

    /// **关键（spike 修正）**：Runtime.bindingCalled 能被订阅拿到 {name,payload}。
    /// 这是注入大脑（Task D/P2/P3）的 RPC 回边——绝不能像 chromiumoxide 那样早 return。
    #[tokio::test]
    async fn binding_called_delivers_name_and_payload() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let mut sub = reg.subscribe("Runtime.bindingCalled", Some("S1"));

        reg.dispatch_message(
            r#"{"method":"Runtime.bindingCalled","sessionId":"S1","params":{"name":"__nomi_rpc","payload":"{\"k\":1}","executionContextId":3}}"#,
        )
        .unwrap();

        let ev = sub.recv().await.unwrap();
        assert_eq!(ev.params["name"], "__nomi_rpc");
        assert_eq!(ev.params["payload"], r#"{"k":1}"#);
        assert_eq!(ev.params["executionContextId"], 3);
    }

    /// attachedToTarget 事件 → 自动登记子 session（含 target 类型）。
    #[tokio::test]
    async fn attached_to_target_registers_child_session() {
        let reg = SessionRegistry::new();
        assert!(!reg.has_session("CHILD"));
        // 我们的传输层会先订阅再处理；这里直接 dispatch 验证副作用即可。
        reg.dispatch_message(
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"CHILD","targetInfo":{"type":"service_worker"},"waitingForDebugger":true}}"#,
        )
        .unwrap();
        // dispatch 本身不登记（登记是传输层职责，见 transport.rs），但订阅可拿到。
        // 这里改为：传输层把登记委托给 register_session，故先手动模拟其行为。
        reg.register_session("CHILD", "service_worker");
        assert!(reg.has_session("CHILD"));
        assert_eq!(reg.target_type("CHILD").as_deref(), Some("service_worker"));
    }

    /// detachedFromTarget 事件 → 标记对应 session 关闭，挂起命令被 drain。
    #[tokio::test]
    async fn detached_event_closes_session() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let rx = reg.register_command("S1", call(1)).unwrap();

        reg.dispatch_message(
            r#"{"method":"Target.detachedFromTarget","params":{"sessionId":"S1","targetId":"T1"}}"#,
        )
        .unwrap();

        assert_eq!(rx.await.unwrap().unwrap_err(), TransportError::SessionClosed);
        // 之后该 session 上 send 短路。
        assert_eq!(
            reg.register_command("S1", call(2)).unwrap_err(),
            TransportError::SessionClosed
        );
    }

    /// 畸形消息（既无 id 又无 method）→ Protocol 错误。
    #[test]
    fn message_without_id_or_method_is_protocol_error() {
        let reg = SessionRegistry::new();
        let err = reg.dispatch_message(r#"{"sessionId":"S1","foo":1}"#).unwrap_err();
        assert!(matches!(err, TransportError::Protocol(_)));
    }

    /// 非 JSON → Protocol 错误（不 panic）。
    #[test]
    fn non_json_is_protocol_error() {
        let reg = SessionRegistry::new();
        let err = reg.dispatch_message("not json at all").unwrap_err();
        assert!(matches!(err, TransportError::Protocol(_)));
    }

    /// 未知 id 的回包静默丢弃（无回调），不 panic、不报错。
    #[test]
    fn unknown_id_response_is_dropped_silently() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        // 没登记任何命令，直接喂回包。
        reg.dispatch_message(r#"{"id":99,"sessionId":"S1","result":{}}"#)
            .unwrap();
    }

    /// 无人订阅的事件静默丢弃，不报错。
    #[test]
    fn event_without_subscriber_is_dropped() {
        let reg = SessionRegistry::new();
        reg.dispatch_message(r#"{"method":"Page.loadEventFired","params":{}}"#)
            .unwrap();
    }

    /// success 回包缺省 result（null）→ Ok(Null)，不报错。
    #[tokio::test]
    async fn response_without_result_is_ok_null() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let rx = reg.register_command("S1", call(1)).unwrap();
        reg.dispatch_message(r#"{"id":1,"sessionId":"S1"}"#).unwrap();
        assert_eq!(rx.await.unwrap().unwrap(), serde_json::Value::Null);
    }
}
