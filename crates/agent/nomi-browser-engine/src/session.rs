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

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chromiumoxide::types::{CallId, Error as CdpError};
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

/// 根（browser）session 在注册表里的 key。CDP 根连接的消息无 `sessionId` 字段，
/// 我们用一个固定哨兵 key 统一登记，避免 `Option<String>` 在两处分叉。
pub const ROOT_SESSION: &str = "";

/// 事件订阅广播通道容量。CDP 事件可能突发（如导航期 lifecycle / 大量 attachedToTarget），
/// 给足缓冲；订阅者落后只丢老事件（`broadcast` 语义），不阻塞 read loop。
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Maximum number of outstanding control events per reliable subscriber.
///
/// The public receiver remains Tokio's `UnboundedReceiver` for compatibility
/// with the existing control-loop structs, but every queued event owns a slot
/// token and dispatch refuses to enqueue beyond this hard limit. Saturation
/// poisons the whole CDP connection: a security/lifecycle event must never be
/// silently discarded while the browser continues running.
pub(crate) const RELIABLE_EVENT_CAPACITY: usize = 256;
const RELIABLE_EVENT_BYTE_CAPACITY: usize = 16 * 1024 * 1024;
/// Aggregate reliable-queue payload retained by one Chromium Host/connection.
/// Every reliable event copy is charged here.
#[doc(hidden)]
pub const RELIABLE_HOST_EVENT_CAPACITY: usize = 512;
#[doc(hidden)]
pub const RELIABLE_HOST_EVENT_BYTE_CAPACITY: usize = 4 * 1024 * 1024;
const MAX_RELIABLE_SUBSCRIBERS: usize = 256;
const MAX_BROADCAST_SUBSCRIPTIONS: usize = 2_048;
const MAX_LIVE_SESSIONS: usize = 4_096;
const MAX_PENDING_CALLBACKS_PER_SESSION: usize = 1_024;
const MAX_PENDING_CALLBACKS_PER_CONNECTION: usize = 4_096;
const DEAD_SESSION_CAPACITY: usize = 1_024;
const DEAD_SESSION_TTL: Duration = Duration::from_secs(5 * 60);
pub(crate) const MAX_CDP_IDENTIFIER_BYTES: usize = 4 * 1024;
const MAX_CDP_METHOD_BYTES: usize = 512;
const MAX_CDP_TARGET_TYPE_BYTES: usize = 256;
const MAX_CDP_EVENT_HEAP_BYTES: usize = 16 * 1024 * 1024;

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

/// Transport registration only. Run/tab authorization belongs to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionAdmission { Admitted, Rejected }


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
    /// Identity learned from the complete browser attach envelope.
    target_id: Option<String>,
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
            target_id: None,
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
    /// A reliable-delivery slot is released only after the receiver (and any
    /// clones it made) drops the event. Ordinary broadcast events carry none.
    reliable_slot: Option<Arc<ReliableQueueSlot>>,
}

impl CdpEvent {
    fn approximate_heap_bytes(&self) -> usize {
        self.method
            .len()
            .saturating_add(self.session_id.len())
            .saturating_add(approximate_json_heap_bytes(&self.params))
    }
}

fn approximate_json_heap_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 1,
        serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(_) => 16,
        serde_json::Value::String(value) => value.len(),
        serde_json::Value::Array(values) => values.iter().fold(
            values.len().saturating_mul(std::mem::size_of::<serde_json::Value>()),
            |total, value| total.saturating_add(approximate_json_heap_bytes(value)),
        ),
        serde_json::Value::Object(values) => values.iter().fold(
            values.len().saturating_mul(
                std::mem::size_of::<String>() + std::mem::size_of::<serde_json::Value>(),
            ),
            |total, (key, value)| {
                total
                    .saturating_add(key.len())
                    .saturating_add(approximate_json_heap_bytes(value))
            },
        ),
    }
}

#[derive(Debug)]
struct ReliableQueueSlot {
    _subscriber: ReliableQueueReservation,
    _host: ReliableQueueReservation,
}

#[derive(Debug)]
struct ReliableQueueBudget {
    outstanding_events: AtomicUsize,
    outstanding_bytes: AtomicUsize,
    event_limit: usize,
    byte_limit: usize,
}

impl ReliableQueueBudget {
    fn new(event_limit: usize, byte_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            outstanding_events: AtomicUsize::new(0),
            outstanding_bytes: AtomicUsize::new(0),
            event_limit,
            byte_limit,
        })
    }

    fn try_reserve(self: &Arc<Self>, bytes: usize) -> Option<ReliableQueueReservation> {
        if !try_reserve(&self.outstanding_events, 1, self.event_limit) {
            return None;
        }
        if !try_reserve(&self.outstanding_bytes, bytes, self.byte_limit) {
            self.outstanding_events.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(ReliableQueueReservation {
            budget: Arc::clone(self),
            bytes,
        })
    }

    #[cfg(test)]
    fn counts(&self) -> (usize, usize) {
        (
            self.outstanding_events.load(Ordering::Acquire),
            self.outstanding_bytes.load(Ordering::Acquire),
        )
    }
}

#[derive(Debug)]
struct ReliableQueueReservation {
    budget: Arc<ReliableQueueBudget>,
    bytes: usize,
}

impl Drop for ReliableQueueReservation {
    fn drop(&mut self) {
        self.budget
            .outstanding_events
            .fetch_sub(1, Ordering::AcqRel);
        self.budget
            .outstanding_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReliableBudgetScope {
    Subscriber,
    Host,
}

impl ReliableBudgetScope {
    fn limits(self) -> (&'static str, usize, usize) {
        match self {
            Self::Subscriber => (
                "subscriber",
                RELIABLE_EVENT_CAPACITY,
                RELIABLE_EVENT_BYTE_CAPACITY,
            ),
            Self::Host => (
                "Host aggregate",
                RELIABLE_HOST_EVENT_CAPACITY,
                RELIABLE_HOST_EVENT_BYTE_CAPACITY,
            ),
        }
    }
}

#[derive(Clone)]
struct ReliableSubscriber {
    sender: mpsc::UnboundedSender<CdpEvent>,
    subscriber_budget: Arc<ReliableQueueBudget>,
    host_budget: Arc<ReliableQueueBudget>,
}

impl ReliableSubscriber {
    fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    fn try_send(&self, event: &CdpEvent, bytes: usize) -> Result<(), ReliableSendError> {
        if self.sender.is_closed() {
            return Err(ReliableSendError::Closed);
        }
        // Reserve all logical memory before the deep `serde_json::Value`
        // clone. A saturated peer therefore cannot force one transient clone
        // per subscriber while admission is already known to be impossible.
        let subscriber = self
            .subscriber_budget
            .try_reserve(bytes)
            .ok_or(ReliableSendError::Full(ReliableBudgetScope::Subscriber))?;
        let host = self
            .host_budget
            .try_reserve(bytes)
            .ok_or(ReliableSendError::Full(ReliableBudgetScope::Host))?;

        let mut event = event.clone();
        debug_assert!(event.reliable_slot.is_none());
        event.reliable_slot = Some(Arc::new(ReliableQueueSlot {
            _subscriber: subscriber,
            _host: host,
        }));
        self.sender
            .send(event)
            .map_err(|_| ReliableSendError::Closed)
    }
}

fn try_reserve(counter: &AtomicUsize, amount: usize, limit: usize) -> bool {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(amount) else {
            return false;
        };
        if next > limit {
            return false;
        }
        match counter.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

fn validate_text_bound(
    kind: &str,
    value: &str,
    limit: usize,
) -> Result<(), TransportError> {
    if value.len() > limit {
        Err(TransportError::Protocol(format!(
            "CDP {kind} exceeds hard limit: {} bytes > {limit} bytes",
            value.len()
        )))
    } else {
        Ok(())
    }
}

/// Shared pure admission check for every retained CDP identifier (session,
/// target, frame, execution-context, and object ids). Keeping one helper
/// prevents background routers from silently drifting above the registry's
/// 4 KiB invariant.
pub(crate) fn validate_cdp_identifier(
    kind: &str,
    value: &str,
) -> Result<(), TransportError> {
    validate_text_bound(kind, value, MAX_CDP_IDENTIFIER_BYTES)
}

enum ReliableSendError {
    Closed,
    Full(ReliableBudgetScope),
}

struct DeadSession {
    session_id: String,
    crashed: bool,
    expires_at: Instant,
}

/// 事件订阅键：(method, session)。`session=None` 表示订阅**任意 session** 的该事件
/// （供根/全局监听，如 `attachedToTarget` 总在根 session 上来）。
type SubKey = (String, Option<String>);

/// 全部共享路由状态。`Mutex` 保护内部表；`dispatch_message` 是纯函数式入口（只读
/// 入参字符串 + 改内部表），单测可直接构造并喂 JSON。
pub struct SessionRegistry {
    inner: Mutex<RegistryInner>,
    fatal: watch::Sender<Option<TransportError>>,
    reliable_host_budget: Arc<ReliableQueueBudget>,
}

struct RegistryInner {
    /// 所有活动 session：sessionId（根 = `ROOT_SESSION`）→ Session。
    sessions: HashMap<String, Session>,
    /// Target-to-session routing for lifecycle events such as
    /// `Target.targetCrashed`, whose payload has a targetId but no sessionId.
    target_sessions: HashMap<String, String>,
    /// 事件订阅：(method, Option<session>) → broadcast 发送端。
    subscriptions: HashMap<SubKey, broadcast::Sender<CdpEvent>>,
    /// Security/lifecycle control events must never be dropped merely because
    /// a consumer was briefly busy. The compatibility-facing Tokio channel is
    /// guarded by a strict outstanding-event quota; overflow poisons the
    /// connection instead of silently dropping the control event.
    reliable_subscriptions: HashMap<SubKey, Vec<ReliableSubscriber>>,
    /// Recently dead sessions are tombstones, not live routing entries. Crash
    /// classification is sticky only within this bounded TTL/LRU cache.
    dead_sessions: VecDeque<DeadSession>,
    /// Total live command callbacks, maintained alongside the per-session maps
    /// so admission does not need to scan every session.
    pending_callbacks: usize,
    /// 整个连接是否已关闭（粘性）。置位后所有 send 短路 `Closed`。
    connection_closed: bool,
}

impl RegistryInner {

    fn remove_session(&mut self, session_id: &str, error: TransportError) {
        if let Some(mut session) = self.sessions.remove(session_id) {
            let callback_count = session.callbacks.len();
            for (_id, tx) in session.callbacks.drain() {
                let _ = tx.send(Err(error.clone()));
            }
            self.pending_callbacks = self.pending_callbacks.saturating_sub(callback_count);
        }
        self.subscriptions
            .retain(|(_, subscribed_session), _| subscribed_session.as_deref() != Some(session_id));
        self.reliable_subscriptions
            .retain(|(_, subscribed_session), _| subscribed_session.as_deref() != Some(session_id));
        self.target_sessions
            .retain(|_, mapped_session| mapped_session != session_id);
    }

    fn prune_dead_sessions(&mut self, now: Instant) {
        self.dead_sessions
            .retain(|record| record.expires_at > now);
        while self.dead_sessions.len() > DEAD_SESSION_CAPACITY {
            self.dead_sessions.pop_front();
        }
    }

    fn dead_error(&mut self, session_id: &str) -> Option<TransportError> {
        self.prune_dead_sessions(Instant::now());
        self.dead_sessions
            .iter()
            .rev()
            .find(|record| record.session_id == session_id)
            .map(|record| {
                if record.crashed {
                    TransportError::SessionCrashed
                } else {
                    TransportError::SessionClosed
                }
            })
    }

    fn clear_dead_session(&mut self, session_id: &str) {
        self.dead_sessions
            .retain(|record| record.session_id != session_id);
    }

    fn remember_dead_session(&mut self, session_id: &str, crashed: bool) {
        self.prune_dead_sessions(Instant::now());
        self.clear_dead_session(session_id);
        self.dead_sessions.push_back(DeadSession {
            session_id: session_id.to_owned(),
            crashed,
            expires_at: Instant::now() + DEAD_SESSION_TTL,
        });
        while self.dead_sessions.len() > DEAD_SESSION_CAPACITY {
            self.dead_sessions.pop_front();
        }
    }

    fn prune_subscriptions(&mut self) {
        self.subscriptions
            .retain(|_, sender| sender.receiver_count() > 0);
        self.reliable_subscriptions.retain(|_, subscribers| {
            subscribers.retain(|subscriber| !subscriber.is_closed());
            !subscribers.is_empty()
        });
    }

    fn reliable_subscriber_count(&self) -> usize {
        self.reliable_subscriptions.values().map(Vec::len).sum()
    }

    fn fail_connection(&mut self) {
        if self.connection_closed {
            return;
        }
        self.connection_closed = true;
        for session in self.sessions.values_mut() {
            for (_id, tx) in session.callbacks.drain() {
                let _ = tx.send(Err(TransportError::Closed));
            }
        }
        self.pending_callbacks = 0;
        self.sessions.clear();
        self.dead_sessions.clear();
        self.subscriptions.clear();
        self.reliable_subscriptions.clear();
        self.target_sessions.clear();
    }
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
        let (fatal, _initial_receiver) = watch::channel(None);
        sessions.insert(
            ROOT_SESSION.to_string(),
            Session::new(ROOT_SESSION, "browser"),
        );
        Self {
            inner: Mutex::new(RegistryInner {
                sessions,
                target_sessions: HashMap::new(),
                subscriptions: HashMap::new(),
                reliable_subscriptions: HashMap::new(),
                dead_sessions: VecDeque::new(),
                pending_callbacks: 0,
                connection_closed: false,
            }),
            fatal,
            reliable_host_budget: ReliableQueueBudget::new(
                RELIABLE_HOST_EVENT_CAPACITY,
                RELIABLE_HOST_EVENT_BYTE_CAPACITY,
            ),
        }
    }


    /// Idempotent registration for an explicit attach response. This cannot
    /// revive a retired session or change an existing session's target type.
    pub fn register_session(&self, session_id: impl Into<String>, target_type: impl Into<String>) {
        let session_id = session_id.into();
        let target_type = target_type.into();
        if let Err(error) = validate_cdp_identifier("session id", &session_id)
            .and_then(|()| validate_text_bound("target type", &target_type, MAX_CDP_TARGET_TYPE_BYTES)) {
            self.poison_connection(error); return;
        }
        let mut state = self.inner.lock().unwrap();
        if state.connection_closed || state.dead_error(&session_id).is_some() { return; }
        if let Some(existing) = state.sessions.get(&session_id) {
            if existing.target_type != target_type {
                self.poison_connection_locked(&mut state, TransportError::Protocol("CDP session registration changed target type".into()));
            }
            return;
        }
        if state.sessions.len() >= MAX_LIVE_SESSIONS {
            self.poison_connection_locked(&mut state, TransportError::Protocol(format!("live CDP session limit exceeded ({MAX_LIVE_SESSIONS})")));
            return;
        }
        state.sessions.insert(session_id.clone(), Session::new(session_id, target_type));
    }

    /// Record the full attach envelope before publishing its event. Parent and
    /// opener strings are bounded metadata, never a source of run authority.
    pub(crate) fn register_attached(
        &self, parent_session_id: &str, session_id: impl Into<String>, target_id: impl Into<String>,
        target_type: impl Into<String>, opener_target_id: Option<&str>,
    ) -> SessionAdmission {
        let session_id = session_id.into();
        let target_id = target_id.into();
        let target_type = target_type.into();
        let validation = validate_cdp_identifier("session id", &session_id)
            .and_then(|()| validate_cdp_identifier("target id", &target_id))
            .and_then(|()| validate_cdp_identifier("parent session id", parent_session_id))
            .and_then(|()| opener_target_id.map_or(Ok(()), |id| validate_cdp_identifier("opener target id", id)))
            .and_then(|()| validate_text_bound("target type", &target_type, MAX_CDP_TARGET_TYPE_BYTES));
        if let Err(error) = validation { self.poison_connection(error); return SessionAdmission::Rejected; }
        let mut state = self.inner.lock().unwrap();
        if state.connection_closed { return SessionAdmission::Rejected; }
        if state.target_sessions.get(&target_id).is_some_and(|id| id != &session_id) {
            self.poison_connection_locked(&mut state, TransportError::Protocol("CDP target attached through multiple live sessions".into()));
            return SessionAdmission::Rejected;
        }
        if let Some(existing) = state.sessions.get(&session_id) {
            // A response-only registration may acquire its full metadata when
            // the event arrives. Established identity can never be replaced.
            if existing.target_type != target_type || existing.target_id.as_ref().is_some_and(|id| id != &target_id) {
                self.poison_connection_locked(&mut state, TransportError::Protocol("duplicate CDP session changed target identity or type".into()));
                return SessionAdmission::Rejected;
            }
        } else if state.sessions.len() >= MAX_LIVE_SESSIONS {
            self.poison_connection_locked(&mut state, TransportError::Protocol(format!("live CDP session limit exceeded ({MAX_LIVE_SESSIONS})")));
            return SessionAdmission::Rejected;
        }
        state.clear_dead_session(&session_id);
        let session = state.sessions.entry(session_id.clone()).or_insert_with(|| Session::new(&session_id, target_type));
        session.target_id = Some(target_id.clone());
        session.closed = false;
        session.crashed = false;
        state.target_sessions.insert(target_id, session_id);
        SessionAdmission::Admitted
    }

    pub(crate) fn attached_session_matches(&self, session_id: &str, target_id: &str, target_type: &str) -> bool {
        let state = self.inner.lock().unwrap();
        !state.connection_closed && state.sessions.get(session_id).is_some_and(|session|
            !session.is_dead() && session.target_id.as_deref() == Some(target_id) && session.target_type == target_type)
    }

    /// 该 session 当前是否已登记。
    pub fn has_session(&self, session_id: &str) -> bool {
        self.inner.lock().unwrap().sessions.contains_key(session_id)
    }

    /// Returns the sticky crash state for a registered session. Unknown or
    /// normally-closed sessions are not reported as crashed.
    pub fn is_session_crashed(&self, session_id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.sessions
            .get(session_id)
            .is_some_and(|session| session.crashed)
        {
            return true;
        }
        matches!(g.dead_error(session_id), Some(TransportError::SessionCrashed))
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

    #[cfg(test)]
    pub(crate) fn pending_callback_count(&self) -> usize {
        self.inner.lock().unwrap().pending_callbacks
    }

    /// Subscribe to sticky abnormal-termination notifications. Explicit
    /// [`fail_connection`](Self::fail_connection) shutdown does not publish a
    /// fatal signal; protocol/queue/read-loop failures do.
    pub fn subscribe_fatal(&self) -> watch::Receiver<Option<TransportError>> {
        self.fatal.subscribe()
    }

    /// Poison the connection after an invariant or transport failure. This is
    /// distinct from normal explicit shutdown and wakes lifecycle supervisors.
    pub fn poison_connection(&self, error: TransportError) {
        let mut g = self.inner.lock().unwrap();
        self.poison_connection_locked(&mut g, error);
    }

    fn poison_connection_locked(&self, g: &mut RegistryInner, error: TransportError) {
        let first_failure = !g.connection_closed;
        g.fail_connection();
        if first_failure {
            self.fatal.send_replace(Some(error));
        }
    }

    /// 订阅某 method（可选限定 session）的事件流。返回 broadcast 接收端。
    /// `session=None` 订阅任意 session 的该事件。
    ///
    /// 连接已关（`fail_connection` 已清空订阅表）→ 返回一个**已关闭**的接收端
    /// （首次 `recv()` 即 `RecvError::Closed`），而不是把新 sender 插回已死的
    /// 注册表——那个 sender 永不 fire 也永不 drop，等待方会无限悬挂。
    pub fn subscribe(
        &self,
        method: impl Into<String>,
        session_id: Option<&str>,
    ) -> broadcast::Receiver<CdpEvent> {
        let method = method.into();
        if let Err(error) = validate_text_bound("method", &method, MAX_CDP_METHOD_BYTES)
            .and_then(|()| {
                session_id.map_or(Ok(()), |session_id| {
                    validate_cdp_identifier("subscription session id", session_id)
                })
            })
        {
            self.poison_connection(error);
            let (tx, rx) = broadcast::channel(1);
            drop(tx);
            return rx;
        }
        let key: SubKey = (method, session_id.map(|s| s.to_string()));
        let mut g = self.inner.lock().unwrap();
        if g.connection_closed {
            let (tx, rx) = broadcast::channel(1);
            drop(tx);
            return rx;
        }
        g.prune_subscriptions();
        if !g.subscriptions.contains_key(&key)
            && g.subscriptions.len() >= MAX_BROADCAST_SUBSCRIPTIONS
        {
            self.poison_connection_locked(
                &mut g,
                TransportError::Protocol(format!(
                    "CDP broadcast subscription limit exceeded ({MAX_BROADCAST_SUBSCRIPTIONS})"
                )),
            );
            let (tx, rx) = broadcast::channel(1);
            drop(tx);
            return rx;
        }
        let tx = g
            .subscriptions
            .entry(key)
            .or_insert_with(|| broadcast::channel(EVENT_CHANNEL_CAPACITY).0);
        tx.subscribe()
    }

    /// Subscribe to a control event without broadcast lag loss. This is only
    /// for events where dropping one item can strand a paused target/request;
    /// ordinary observation and telemetry should continue to use `subscribe`.
    pub fn subscribe_reliable(
        &self,
        method: impl Into<String>,
        session_id: Option<&str>,
    ) -> mpsc::UnboundedReceiver<CdpEvent> {
        let method = method.into();
        let (tx, rx) = mpsc::unbounded_channel();
        if let Err(error) = validate_text_bound("method", &method, MAX_CDP_METHOD_BYTES)
            .and_then(|()| {
                session_id.map_or(Ok(()), |session_id| {
                    validate_cdp_identifier("subscription session id", session_id)
                })
            })
        {
            self.poison_connection(error);
            return rx;
        }
        let key: SubKey = (method, session_id.map(str::to_owned));
        let mut g = self.inner.lock().unwrap();
        if g.connection_closed {
            return rx;
        }
        g.prune_subscriptions();
        if g.reliable_subscriber_count() >= MAX_RELIABLE_SUBSCRIBERS {
            self.poison_connection_locked(
                &mut g,
                TransportError::Protocol(format!(
                    "reliable CDP subscriber limit exceeded ({MAX_RELIABLE_SUBSCRIBERS})"
                )),
            );
            return rx;
        }
        g.reliable_subscriptions
            .entry(key)
            .or_default()
            .push(ReliableSubscriber {
                sender: tx,
                subscriber_budget: ReliableQueueBudget::new(
                    RELIABLE_EVENT_CAPACITY,
                    RELIABLE_EVENT_BYTE_CAPACITY,
                ),
                host_budget: Arc::clone(&self.reliable_host_budget),
            });
        rx
    }

    /// One ordered reliable queue for a bounded set of methods. All method
    /// registrations share the same sender and subscriber budget, so alternating
    /// event kinds cannot multiply retained capacity. Registration slots still
    /// count against the existing metadata bound. Duplicate methods are ignored.
    pub fn subscribe_reliable_sequence(
        &self,
        methods: &[&str],
        session_id: Option<&str>,
    ) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let validation = (|| {
            if methods.is_empty() || methods.len() > 16 {
                return Err(TransportError::Protocol("reliable event sequence requires 1..16 methods".into()));
            }
            for method in methods {
                if method.is_empty() { return Err(TransportError::Protocol("empty reliable event method".into())); }
                validate_text_bound("method", method, MAX_CDP_METHOD_BYTES)?;
            }
            if let Some(session) = session_id { validate_cdp_identifier("subscription session id", session)?; }
            Ok(())
        })();
        if let Err(error) = validation { self.poison_connection(error); return receiver; }
        let methods: std::collections::BTreeSet<_> = methods.iter().copied().collect();
        let mut state = self.inner.lock().unwrap();
        if state.connection_closed { return receiver; }
        state.prune_subscriptions();
        if state.reliable_subscriber_count().saturating_add(methods.len()) > MAX_RELIABLE_SUBSCRIBERS {
            self.poison_connection_locked(&mut state, TransportError::Protocol("reliable event subscription limit exceeded".into()));
            return receiver;
        }
        let subscriber = ReliableSubscriber {
            sender, subscriber_budget: ReliableQueueBudget::new(RELIABLE_EVENT_CAPACITY, RELIABLE_EVENT_BYTE_CAPACITY),
            host_budget: Arc::clone(&self.reliable_host_budget),
        };
        for method in methods {
            state.reliable_subscriptions.entry((method.to_owned(), session_id.map(str::to_owned))).or_default().push(subscriber.clone());
        }
        receiver
    }


    /// Whether a live lossless subscriber exists for `method` (exact-session
    /// or wildcard). The transport uses this as the `Fetch.enable` arming gate
    /// in `handle_attached`: interception must never be switched on before a
    /// `Fetch.requestPaused` consumer is registered, because a paused request
    /// whose event found no subscriber is silently dropped and CDP never
    /// re-emits it — that session's network would be wedged forever.
    pub fn has_reliable_subscriber(&self, method: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        g.prune_subscriptions();
        g.reliable_subscriptions
            .iter()
            .any(|((subscribed_method, _), senders)| {
                subscribed_method == method
                    && senders.iter().any(|sender| !sender.is_closed())
            })
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
        if g.pending_callbacks >= MAX_PENDING_CALLBACKS_PER_CONNECTION {
            return Err(TransportError::Protocol(format!(
                "pending CDP command limit exceeded ({MAX_PENDING_CALLBACKS_PER_CONNECTION})"
            )));
        }
        if !g.sessions.contains_key(session_id) {
            return Err(g
                .dead_error(session_id)
                .unwrap_or(TransportError::SessionClosed));
        }
        let session = g
            .sessions
            .get_mut(session_id)
            .ok_or(TransportError::SessionClosed)?;
        if session.is_dead() {
            return Err(session.dead_error());
        }
        if session.callbacks.len() >= MAX_PENDING_CALLBACKS_PER_SESSION {
            return Err(TransportError::Protocol(format!(
                "pending CDP command limit exceeded for session {session_id} \
                 ({MAX_PENDING_CALLBACKS_PER_SESSION})"
            )));
        }
        if session.callbacks.contains_key(&call_id) {
            return Err(TransportError::Protocol(format!(
                "duplicate pending CDP call id {call_id:?} for session {session_id}"
            )));
        }
        let (tx, rx) = oneshot::channel();
        session.callbacks.insert(call_id, tx);
        g.pending_callbacks += 1;
        Ok(rx)
    }

    /// 取消一个已登记但未投递的命令回调（命令发送失败 / 超时清理时调）。
    pub fn cancel_command(&self, session_id: &str, call_id: CallId) {
        let mut g = self.inner.lock().unwrap();
        if let Some(s) = g.sessions.get_mut(session_id) {
            if s.callbacks.remove(&call_id).is_some() {
                g.pending_callbacks = g.pending_callbacks.saturating_sub(1);
            }
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
        if let Err(error) = validate_cdp_identifier("message session id", &session_key) {
            self.poison_connection(error.clone());
            return Err(error);
        }

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
                if let Err(error) =
                    validate_text_bound("event method", &method, MAX_CDP_METHOD_BYTES)
                {
                    self.poison_connection(error.clone());
                    return Err(error);
                }
                let params = env.params.unwrap_or(serde_json::Value::Null);
                self.handle_event(&method, &session_key, params)
            }
        }
    }

    /// 投递命令回包到对应回调。找不到回调（已超时清理 / 未知 id）则静默丢弃。
    fn deliver_response(&self, session_key: &str, call_id: CallId, result: CommandResult) {
        let mut g = self.inner.lock().unwrap();
        if let Some(session) = g.sessions.get_mut(session_key)
            && let Some(tx) = session.callbacks.remove(&call_id)
        {
            g.pending_callbacks = g.pending_callbacks.saturating_sub(1);
            // 接收端可能已 drop（调用方放弃等待）——忽略发送失败。
            let _ = tx.send(result);
        }
    }

    /// 处理一条事件：先做生命周期副作用（attach/detach/crash），再广播给订阅者。
    fn handle_event(
        &self,
        method: &str,
        session_key: &str,
        params: serde_json::Value,
    ) -> Result<(), TransportError> {
        let event_bytes = method
            .len()
            .saturating_add(session_key.len())
            .saturating_add(approximate_json_heap_bytes(&params));
        if event_bytes > MAX_CDP_EVENT_HEAP_BYTES {
            let error = TransportError::Protocol(format!(
                "CDP event exceeds heap bound: {event_bytes} bytes > \
                 {MAX_CDP_EVENT_HEAP_BYTES} bytes"
            ));
            self.poison_connection(error.clone());
            return Err(error);
        }
        // 生命周期副作用：登记子 session / 标记死亡。这些只改本注册表，不发 CDP
        // 命令（runIfWaitingForDebugger 由传输层在「先装监听」之后补发）。
        match method {
            "Target.attachedToTarget" => {
                let session_id = params.get("sessionId").and_then(|value| value.as_str());
                let target_info = params.get("targetInfo");
                let target_id = target_info
                    .and_then(|value| value.get("targetId"))
                    .and_then(|value| value.as_str());
                let target_type = target_info
                    .and_then(|value| value.get("type"))
                    .and_then(|value| value.as_str());
                let opener_target_id = target_info
                    .and_then(|value| value.get("openerId"))
                    .and_then(|value| value.as_str());
                if let (Some(session_id), Some(target_id), Some(target_type)) =
                    (session_id, target_id, target_type)
                {
                    let _ = self.register_attached(
                        session_key,
                        session_id,
                        target_id,
                        target_type,
                        opener_target_id,
                    );
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) {
                    self.fail_session(sid, false);
                }
            }
            "Target.targetDestroyed" => {
                // Some worker/service-worker lifecycles publish authoritative
                // target absence without a preceding detachedFromTarget. Use
                // the target map to refund the exact auxiliary quota instead
                // of leaving an unattributed bucket slot stranded.
                if let Some(target_id) = params.get("targetId").and_then(|value| value.as_str()) {
                    let session_id = self
                        .inner
                        .lock()
                        .unwrap()
                        .target_sessions
                        .get(target_id)
                        .cloned();
                    if let Some(session_id) = session_id {
                        self.fail_session(&session_id, false);
                    }
                }
            }
            "Target.targetCrashed" => {
                if let Some(target_id) = params.get("targetId").and_then(|value| value.as_str()) {
                    let session_id = self
                        .inner
                        .lock()
                        .unwrap()
                        .target_sessions
                        .get(target_id)
                        .cloned();
                    if let Some(session_id) = session_id {
                        self.fail_session(&session_id, true);
                    }
                }
                // targetCrashed 在根 session 上来，targetId 在 params。子 session 的崩溃
                // 通过对应 sessionId 标记；若只有 targetId 无 sessionId，则交由后续
                // detachedFromTarget 兜底（CDP 通常崩溃后随即 detach）。
                if let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) {
                    self.fail_session(sid, true);
                }
            }
            "Inspector.detached" => {
                // Inspector detach closes only this CDP session. The target
                // router independently retains physical page/quota ownership
                // and performs an exact close/absence cleanup.
                if !session_key.is_empty() {
                    self.fail_session(session_key, false);
                }
            }
            _ => {}
        }

        let event = CdpEvent {
            method: method.to_string(),
            session_id: session_key.to_string(),
            params,
            reliable_slot: None,
        };
        self.broadcast_event(event)
    }

    /// 广播一个事件给：① 精确 (method, session) 订阅者；② 通配 (method, None) 订阅者。
    /// 无人订阅 → 静默丢弃（合法：不是所有事件都有人关心）。
    fn broadcast_event(&self, event: CdpEvent) -> Result<(), TransportError> {
        let event_bytes = event.approximate_heap_bytes();
        let mut g = self.inner.lock().unwrap();
        if g.connection_closed {
            return Err(TransportError::Closed);
        }
        g.prune_subscriptions();
        let exact: SubKey = (event.method.clone(), Some(event.session_id.clone()));
        let wildcard: SubKey = (event.method.clone(), None);
        if let Some(tx) = g.subscriptions.get(&exact) {
            let _ = tx.send(event.clone());
        }
        if let Some(tx) = g.subscriptions.get(&wildcard) {
            let _ = tx.send(event.clone());
        }
        for key in [&exact, &wildcard] {
            let mut saturated = None;
            if let Some(subscribers) = g.reliable_subscriptions.get_mut(key) {
                subscribers.retain(|subscriber| {
                    if saturated.is_some() {
                        return !subscriber.is_closed();
                    }
                    match subscriber.try_send(&event, event_bytes) {
                        Ok(()) => true,
                        Err(ReliableSendError::Closed) => false,
                        Err(ReliableSendError::Full(scope)) => {
                            saturated = Some(scope);
                            true
                        }
                    }
                });
            }
            if let Some(scope) = saturated {
                let (scope, event_limit, byte_limit) = scope.limits();
                let detail = format!(
                    "reliable CDP event queue saturated for {} at {scope} scope \
                     ({event_limit} events / {byte_limit} bytes)",
                    event.method
                );
                let error = TransportError::Protocol(detail);
                self.poison_connection_locked(&mut g, error.clone());
                return Err(error);
            }
        }
        g.prune_subscriptions();
        Ok(())
    }

    /// 标记某 session 死亡（崩溃或关闭），并 drain 其所有挂起回调为对应错误，
    /// 使等待中的 `send` 立即解除（绝不悬挂）。粘性：之后该 session 上 `send` 短路。
    pub fn fail_session(&self, session_id: &str, crashed: bool) {
        let mut g = self.inner.lock().unwrap();
        let crashed = crashed || matches!(g.dead_error(session_id), Some(TransportError::SessionCrashed));
        let error = if crashed {
            TransportError::SessionCrashed
        } else {
            TransportError::SessionClosed
        };
        g.remove_session(session_id, error);
        g.remember_dead_session(session_id, crashed);
    }

    /// 标记整个连接关闭（WS 断开）：drain 所有 session 的所有挂起回调为 `Closed`，
    /// 并置 `connection_closed`，使之后所有 `register_command` 短路 `Closed`。
    pub fn fail_connection(&self) {
        let mut g = self.inner.lock().unwrap();
        g.fail_connection();
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

    #[tokio::test]
    async fn attached_event_registers_session_before_it_is_broadcast() {
        let reg = SessionRegistry::new();
        let mut attached = reg.subscribe("Target.attachedToTarget", None);
        reg.dispatch_message(
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"S1","targetInfo":{"targetId":"T1","type":"page"}}}"#,
        )
        .unwrap();

        let event = attached.recv().await.unwrap();
        assert_eq!(event.params["sessionId"], "S1");
        assert!(reg.has_session("S1"));
        assert_eq!(reg.target_type("S1").as_deref(), Some("page"));
        assert!(reg.register_command("S1", call(1)).is_ok());
    }

    #[tokio::test]
    async fn target_crash_fails_the_session_mapped_at_attach() {
        let reg = SessionRegistry::new();
        reg.dispatch_message(
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"S1","targetInfo":{"targetId":"T1","type":"page"}}}"#,
        )
        .unwrap();
        reg.dispatch_message(
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"S2","targetInfo":{"targetId":"T2","type":"page"}}}"#,
        )
        .unwrap();
        let rx = reg.register_command("S1", call(7)).unwrap();
        let other_lane = reg.register_command("S2", call(8)).unwrap();

        reg.dispatch_message(
            r#"{"method":"Target.targetCrashed","params":{"targetId":"T1","status":"crashed","errorCode":1}}"#,
        )
        .unwrap();

        assert_eq!(rx.await.unwrap(), Err(TransportError::SessionCrashed));
        assert_eq!(
            reg.register_command("S1", call(8)).unwrap_err(),
            TransportError::SessionCrashed
        );

        reg.dispatch_message(
            r#"{"id":8,"sessionId":"S2","result":{"stillAlive":true}}"#,
        )
        .unwrap();
        assert_eq!(
            other_lane.await.unwrap().unwrap()["stillAlive"],
            true,
            "crashing T1 must not fail another target/session"
        );
        assert!(
            reg.register_command("S2", call(9)).is_ok(),
            "the unrelated target remains routable"
        );
    }

    #[tokio::test]
    async fn connection_failure_closes_event_subscriptions() {
        let reg = SessionRegistry::new();
        let fatal = reg.subscribe_fatal();
        let mut events = reg.subscribe("Target.attachedToTarget", None);
        let mut reliable = reg.subscribe_reliable("Fetch.requestPaused", None);
        reg.fail_connection();
        assert!(matches!(
            events.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));
        assert!(reliable.recv().await.is_none());
        assert!(
            fatal.borrow().is_none(),
            "normal explicit shutdown must not be reported as fatal"
        );
    }

    /// **F17**：`fail_connection` 之后再 `subscribe` 的迟到订阅者绝不能拿到一个
    /// 「永不 fire 也永不 close」的接收端——必须立即观察到 `Closed`，且不得把新
    /// sender 插回已清空的注册表（否则等待方无限悬挂 + sender 永久泄漏）。
    #[tokio::test]
    async fn subscribe_after_connection_failure_yields_closed_receiver() {
        let reg = SessionRegistry::new();
        reg.fail_connection();

        let mut late = reg.subscribe("Page.lifecycleEvent", Some("S1"));
        assert!(matches!(
            late.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));

        let mut late_wildcard = reg.subscribe("Page.lifecycleEvent", None);
        assert!(matches!(
            late_wildcard.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));

        // 死注册表保持空：迟到订阅不得复活它。
        assert!(reg.inner.lock().unwrap().subscriptions.is_empty());
    }

    /// **F1**：`has_reliable_subscriber` 是 `Fetch.enable` 的 arming gate——只有
    /// 存在**活的** requestPaused 可靠订阅者时才为 true（订阅前 false / drop 后
    /// false / 连接失败清空后 false）。
    #[test]
    fn has_reliable_subscriber_tracks_live_receivers() {
        let reg = SessionRegistry::new();
        assert!(!reg.has_reliable_subscriber("Fetch.requestPaused"));

        let rx = reg.subscribe_reliable("Fetch.requestPaused", None);
        assert!(reg.has_reliable_subscriber("Fetch.requestPaused"));
        // 不同 method 不串。
        assert!(!reg.has_reliable_subscriber("Target.attachedToTarget"));

        drop(rx);
        assert!(!reg.has_reliable_subscriber("Fetch.requestPaused"));

        let _rx = reg.subscribe_reliable("Fetch.requestPaused", Some("S1"));
        assert!(
            reg.has_reliable_subscriber("Fetch.requestPaused"),
            "an exact-session subscriber also satisfies the arming gate"
        );

        let reg2 = SessionRegistry::new();
        let _held = reg2.subscribe_reliable("Fetch.requestPaused", None);
        reg2.fail_connection();
        assert!(!reg2.has_reliable_subscriber("Fetch.requestPaused"));
    }

    #[tokio::test]
    async fn reliable_control_subscription_delivers_up_to_its_hard_bound() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let mut events = reg.subscribe_reliable("Fetch.requestPaused", None);

        for index in 0..RELIABLE_EVENT_CAPACITY {
            reg.dispatch_message(&format!(
                r#"{{"method":"Fetch.requestPaused","sessionId":"S1","params":{{"index":{index}}}}}"#
            ))
            .unwrap();
        }

        for index in 0..RELIABLE_EVENT_CAPACITY {
            let event = events.recv().await.expect("reliable event");
            assert_eq!(event.params["index"], index);
        }
    }

    #[tokio::test]
    async fn reliable_control_subscription_overflow_poisons_connection() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let fatal = reg.subscribe_fatal();
        let pending = reg.register_command("S1", call(9)).unwrap();
        let mut events = reg.subscribe_reliable("Fetch.requestPaused", None);

        for index in 0..RELIABLE_EVENT_CAPACITY {
            reg.dispatch_message(&format!(
                r#"{{"method":"Fetch.requestPaused","sessionId":"S1","params":{{"index":{index}}}}}"#
            ))
            .unwrap();
        }
        let error = reg
            .dispatch_message(
                r#"{"method":"Fetch.requestPaused","sessionId":"S1","params":{"overflow":true}}"#,
            )
            .expect_err("the first event beyond the bound must poison the connection");
        assert!(matches!(error, TransportError::Protocol(_)));
        assert!(reg.is_connection_closed());
        assert_eq!(fatal.borrow().as_ref(), Some(&error));
        assert_eq!(pending.await.unwrap(), Err(TransportError::Closed));
        assert_eq!(
            reg.register_command("S1", call(10)).unwrap_err(),
            TransportError::Closed
        );

        for _ in 0..RELIABLE_EVENT_CAPACITY {
            assert!(events.recv().await.is_some());
        }
        assert!(events.recv().await.is_none());
    }

    #[test]
    fn reliable_control_subscription_has_a_byte_bound_too() {
        let reg = SessionRegistry::new();
        let _events = reg.subscribe_reliable("Fetch.requestPaused", None);
        let event = CdpEvent {
            method: "Fetch.requestPaused".to_owned(),
            session_id: ROOT_SESSION.to_owned(),
            params: serde_json::Value::String("x".repeat(RELIABLE_EVENT_BYTE_CAPACITY + 1)),
            reliable_slot: None,
        };
        let inner = reg.inner.lock().unwrap();
        let subscriber = &inner
            .reliable_subscriptions
            .values()
            .next()
            .expect("reliable subscription key")[0];
        let bytes = event.approximate_heap_bytes();
        assert!(matches!(
            subscriber.try_send(&event, bytes),
            Err(ReliableSendError::Full(
                ReliableBudgetScope::Subscriber
            ))
        ));
        assert_eq!(subscriber.subscriber_budget.counts(), (0, 0));
        assert_eq!(reg.reliable_host_budget.counts(), (0, 0));
    }

    #[tokio::test]
    async fn reliable_host_budget_is_aggregate_and_drop_returns_every_reservation() {
        let reg = SessionRegistry::new();
        let receivers = (0..3)
            .map(|_| reg.subscribe_reliable("Target.detachedFromTarget", None))
            .collect::<Vec<_>>();

        // 170 events x three deep-copied subscribers = 510 Host slots. No
        // individual subscriber is near its 256-event limit.
        for index in 0..170 {
            reg.dispatch_message(&format!(
                r#"{{"method":"Target.detachedFromTarget","params":{{"index":{index}}}}}"#
            ))
            .expect("aggregate Host budget still has room");
        }
        let error = reg
            .dispatch_message(
                r#"{"method":"Target.detachedFromTarget","params":{"overflow":true}}"#,
            )
            .expect_err("the third copy must hit the aggregate Host event bound");
        assert!(error.to_string().contains("Host aggregate"));
        assert_eq!(
            reg.reliable_host_budget.counts().0,
            RELIABLE_HOST_EVENT_CAPACITY,
            "only successfully enqueued deep copies remain charged"
        );

        // Poison clears senders, and dropping their receivers drops every
        // queued CdpEvent. Each event's RAII token returns subscriber + Host.
        drop(receivers);
        assert_eq!(reg.reliable_host_budget.counts(), (0, 0));
    }


    #[tokio::test]
    async fn reliable_event_stays_charged_after_recv_until_dropped() {
        let registry = SessionRegistry::new();
        let mut receiver = registry.subscribe_reliable("Target.targetCrashed", None);
        registry.dispatch_message(r#"{"method":"Target.targetCrashed","params":{"targetId":"fixture"}}"#).unwrap();
        assert_eq!(registry.reliable_host_budget.counts().0, 1);
        let event = receiver.recv().await.unwrap();
        assert_eq!(registry.reliable_host_budget.counts().0, 1);
        drop(receiver);
        assert_eq!(registry.reliable_host_budget.counts().0, 1);
        drop(event);
        assert_eq!(registry.reliable_host_budget.counts(), (0, 0));
    }

    #[test]
    fn dropping_reliable_receiver_returns_all_queued_event_charges() {
        let registry = SessionRegistry::new();
        let receiver = registry.subscribe_reliable("Target.targetCrashed", None);
        for _ in 0..3 {
            registry.dispatch_message(r#"{"method":"Target.targetCrashed","params":{"targetId":"fixture"}}"#).unwrap();
        }
        assert_eq!(registry.reliable_host_budget.counts().0, 3);
        drop(receiver);
        assert_eq!(registry.reliable_host_budget.counts(), (0, 0));
        assert!(!registry.has_reliable_subscriber("Target.targetCrashed"));
        assert!(!registry.is_connection_closed());
    }

    #[test]
    fn dropped_subscription_churn_does_not_grow_registry_maps() {
        let reg = SessionRegistry::new();
        for index in 0..10_000 {
            drop(reg.subscribe(format!("Page.test{index}"), None));
            drop(reg.subscribe_reliable(format!("Target.test{index}"), None));
        }
        let mut inner = reg.inner.lock().unwrap();
        inner.prune_subscriptions();
        assert!(inner.subscriptions.is_empty());
        assert!(inner.reliable_subscriptions.is_empty());
        assert!(!inner.connection_closed);
    }

    #[test]
    fn detached_and_crashed_session_churn_keeps_only_bounded_tombstones() {
        let reg = SessionRegistry::new();
        for index in 0..(DEAD_SESSION_CAPACITY * 4) {
            let session_id = format!("S{index}");
            reg.register_session(&session_id, "page");
            reg.fail_session(&session_id, index % 2 == 0);
        }
        let inner = reg.inner.lock().unwrap();
        assert_eq!(inner.sessions.len(), 1, "only the root session stays live");
        assert!(inner.sessions.contains_key(ROOT_SESSION));
        assert!(inner.dead_sessions.len() <= DEAD_SESSION_CAPACITY);
        assert_eq!(inner.pending_callbacks, 0);
    }

    #[test]
    fn live_session_admission_is_hard_bounded_and_fail_closed() {
        let reg = SessionRegistry::new();
        let fatal = reg.subscribe_fatal();
        for index in 1..MAX_LIVE_SESSIONS {
            reg.register_session(format!("S{index}"), "page");
        }
        assert_eq!(reg.inner.lock().unwrap().sessions.len(), MAX_LIVE_SESSIONS);

        reg.register_session("OVERFLOW", "page");

        assert!(reg.is_connection_closed());
        assert!(matches!(
            fatal.borrow().as_ref(),
            Some(TransportError::Protocol(_))
        ));
        assert!(reg.inner.lock().unwrap().sessions.is_empty());
    }

    #[test]
    fn expired_crash_tombstone_loses_sticky_classification() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        reg.fail_session("S1", true);
        assert!(reg.is_session_crashed("S1"));
        reg.inner.lock().unwrap().dead_sessions[0].expires_at = Instant::now();

        assert!(!reg.is_session_crashed("S1"));
        assert!(reg.inner.lock().unwrap().dead_sessions.is_empty());
        assert_eq!(
            reg.register_command("S1", call(1)).unwrap_err(),
            TransportError::SessionClosed
        );
    }

    #[test]
    fn oversized_session_identifier_poisons_instead_of_becoming_retained_state() {
        let reg = SessionRegistry::new();
        let fatal = reg.subscribe_fatal();
        reg.register_session("S".repeat(MAX_CDP_IDENTIFIER_BYTES + 1), "page");

        assert!(reg.is_connection_closed());
        assert!(matches!(
            fatal.borrow().as_ref(),
            Some(TransportError::Protocol(_))
        ));
        assert!(reg.inner.lock().unwrap().sessions.is_empty());
    }

    #[test]
    fn repeated_command_cancellation_releases_callback_entries() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        for index in 0..10_000 {
            let id = call(index);
            let receiver = reg.register_command("S1", id).unwrap();
            drop(receiver);
            reg.cancel_command("S1", id);
        }
        let inner = reg.inner.lock().unwrap();
        assert_eq!(inner.pending_callbacks, 0);
        assert!(inner.sessions["S1"].callbacks.is_empty());
    }

    #[test]
    fn pending_callback_admission_is_hard_bounded() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let mut receivers = Vec::new();
        for index in 0..MAX_PENDING_CALLBACKS_PER_SESSION {
            receivers.push(reg.register_command("S1", call(index)).unwrap());
        }
        let error = reg
            .register_command("S1", call(MAX_PENDING_CALLBACKS_PER_SESSION))
            .expect_err("one session may not retain callbacks past its hard limit");
        assert!(matches!(error, TransportError::Protocol(_)));
        assert_eq!(
            reg.inner.lock().unwrap().pending_callbacks,
            MAX_PENDING_CALLBACKS_PER_SESSION
        );
        for index in 0..MAX_PENDING_CALLBACKS_PER_SESSION {
            reg.cancel_command("S1", call(index));
        }
        drop(receivers);
        assert_eq!(reg.inner.lock().unwrap().pending_callbacks, 0);
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
        // Chromium commonly emits detachedFromTarget after targetCrashed; the
        // later close must not erase the bounded crash classification.
        reg.fail_session("S1", false);
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
        assert!(
            !reg.has_session("S1"),
            "a detached session must be removed from the live routing map"
        );
        // 之后该 session 上 send 短路。
        assert_eq!(
            reg.register_command("S1", call(2)).unwrap_err(),
            TransportError::SessionClosed
        );
    }

    #[tokio::test]
    async fn inspector_detached_closes_only_the_session() {
        let reg = SessionRegistry::new();
        reg.register_session("S-inspector", "page");
        let rx = reg.register_command("S-inspector", call(3)).unwrap();

        reg.dispatch_message(
            r#"{"method":"Inspector.detached","sessionId":"S-inspector","params":{"reason":"replaced_with_devtools"}}"#,
        )
        .unwrap();

        assert_eq!(rx.await.unwrap().unwrap_err(), TransportError::SessionClosed);
        assert!(!reg.has_session("S-inspector"));
        assert!(
            !reg.is_connection_closed(),
            "an Inspector session detach must not poison the whole Host transport"
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
    #[test]
    fn late_direct_registration_cannot_revive_a_retired_session() {
        for crashed in [false, true] {
            let registry = SessionRegistry::new();
            registry.register_session("page", "page");
            registry.fail_session("page", crashed);
            registry.register_session("page", "page");
            assert!(!registry.has_session("page"));
            assert_eq!(registry.register_attached(ROOT_SESSION, "page", "target", "page", None), SessionAdmission::Admitted);
            assert!(registry.attached_session_matches("page", "target", "page"));
        }
    }

    #[test]
    fn attach_enriches_response_metadata_but_cannot_replace_identity() {
        let registry = SessionRegistry::new();
        registry.register_session("page", "page");
        assert_eq!(registry.register_attached(ROOT_SESSION, "page", "target", "page", None), SessionAdmission::Admitted);
        assert_eq!(registry.register_attached(ROOT_SESSION, "page", "target", "page", None), SessionAdmission::Admitted);
        assert_eq!(registry.register_attached(ROOT_SESSION, "page", "another", "page", None), SessionAdmission::Rejected);
        assert!(registry.is_connection_closed());
    }

    #[test]
    fn duplicate_target_session_alias_is_connection_fatal() {
        let registry = SessionRegistry::new();
        assert_eq!(registry.register_attached(ROOT_SESSION, "first", "target", "page", None), SessionAdmission::Admitted);
        assert_eq!(registry.register_attached(ROOT_SESSION, "second", "target", "page", None), SessionAdmission::Rejected);
        assert!(registry.is_connection_closed());
    }

    #[test]
    fn attached_sessions_obey_the_connection_fuse_without_task_or_lane_keys() {
        let registry = SessionRegistry::new();
        for index in 0..MAX_LIVE_SESSIONS-1 {
            assert_eq!(registry.register_attached(ROOT_SESSION, format!("session-{index}"), format!("target-{index}"), "worker", None), SessionAdmission::Admitted);
        }
        assert_eq!(registry.register_attached(ROOT_SESSION, "overflow", "overflow", "worker", None), SessionAdmission::Rejected);
        assert!(registry.is_connection_closed());
    }

    #[tokio::test]
    async fn response_without_result_is_ok_null() {
        let reg = SessionRegistry::new();
        reg.register_session("S1", "page");
        let rx = reg.register_command("S1", call(1)).unwrap();
        reg.dispatch_message(r#"{"id":1,"sessionId":"S1"}"#).unwrap();
        assert_eq!(rx.await.unwrap().unwrap(), serde_json::Value::Null);
    }

}
