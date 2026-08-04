//! 注入管线：把 **vendor 的 Playwright InjectedScript**（Apache-2.0，bundle 进
//! [`INJECTED_SOURCE`]）注入到 Chromium 的 **isolated world**，并经 `Runtime.callFunctionOn`
//! 把 injected 实例当 receiver `this` 调它的方法（actionability / ARIA / aria-snapshot /
//! 选择器都走这条句柄）。这是「薄 Rust CDP 编排 + vendor PW 注入 JS 跑 isolated world」
//! 混合架构的 **keystone**（DESIGN §6）。
//!
//! ## 为什么 isolated world
//! 页面看不见、改不了我们的工具箱（与未来 secret 逻辑）。`grantUniveralAccess` 让跨域
//! OOPIF 的 isolated world 仍可访问其文档。
//!
//! ## 生命周期 4 步（DESIGN §6，对标 PW `dom.ts` / `frames.ts` / `crPage.ts`）
//! 1. **物化**：空 source `Page.addScriptToEvaluateOnNewDocument{ source:"",
//!    worldName:<utility>, runImmediately:false }` —— 让 utility world 在**每个新文档**
//!    自动物化（导航后无需重新登记 world，浏览器替我们建）。
//! 2. **补现存 frame**：对当前每个 frame `Page.createIsolatedWorld{ frameId,
//!    worldName:<utility>, grantUniveralAccess:true }`（**CDP 故意拼错的字段名，照抄**）
//!    —— 覆盖「arm 时已经打开」的页面（步骤 1 只管未来文档）。
//! 3. **登记 contextId**：订阅 `Runtime.executionContextCreated`，按
//!    `auxData.frameId` + `name == <utility>` 把 (frameId → executionContextId) 登记；
//!    `executionContextDestroyed`（按 system-unique `uniqueId`）/ `executionContextsCleared`
//!    （导航）失效缓存，下次用时重建。
//! 4. **lazy 注入本体**：首次动作时在该 contextId `Runtime.evaluate`（[`INJECTED_SOURCE`]
//!    + `new <global>.InjectedScript(globalThis, opts)`）得 `objectId` 句柄并缓存。
//!
//! ## callFunctionOn 真实契约（DESIGN §6 / §7）
//! `call_injected(method, args)` = `Runtime.callFunctionOn{ object_id:<injected handle>,
//! function_declaration:"function(...args){ return this.<method>(...args) }",
//! arguments:[...] }`，**injected 实例当 receiver `this`**（省掉 PW 的 UtilityScript 间接
//! 层）。元素句柄作 `arguments[i].objectId` 传入（必须与 injected 同一 world，见 §7）。
//!
//! ## 本 task 范围（P0）与 P1 跟进
//! - **做**：单 page-session 的 world 物化 + 现存 frame 补建 + contextId 登记/失效 + 本体
//!   lazy 注入 + callFunctionOn 封装 + `#[ignore]` 冒烟（`generateAriaTree`/`ariaSnapshot`
//!   返回非空 YAML）。
//! - **P1 跟进**：① 逐帧（OOPIF）跨 session 注入 + frame 缝合（见 §7 复合键
//!   `(sessionId, backendNodeId)`）；② **binding 双向通道**（DESIGN §6/§114 约束：**绝不**
//!   走传输层容量受限 broadcast，须 per-name 回调 / unbounded mpsc / seq→oneshot；本 task
//!   的 generateAriaTree 经 callFunctionOn 直接返值**无需** binding，故 binding 留 P1）；
//!   ③ objectGroup 元素句柄生命周期 + releaseObjectGroup；④ 陈旧检测分层。

use std::collections::HashMap;
use std::sync::Mutex;

use chromiumoxide::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams,
    EnableParams as PageEnableParams, GetFrameTreeParams,
};
use chromiumoxide::cdp::js_protocol::runtime::{
    CallArgument, CallFunctionOnParams, EnableParams as RuntimeEnableParams, EvaluateParams,
    ExecutionContextId, ReleaseObjectGroupParams, RemoteObjectId,
};
use serde_json::Value;

use crate::engine::CssRect;
use crate::transport::{Connection, TransportError};

/// Maximum utility-world contexts retained by one page/OOPIF session.
/// Normal pages stay far below this; a frame churn storm must not turn the
/// lifecycle indexes or cached remote handles into unbounded session state.
pub const MAX_FRAME_WORLDS: usize = 256;

/// 预编译的 vendor PW InjectedScript bundle（单 IIFE，暴露
/// `<global>.InjectedScript`）。**build 时不依赖 Node**：`injected/build.sh` 手动重生成、
/// 产物 check-in 进仓（DESIGN §24）。bundle 头部带 Apache-2.0 attribution；NOTICE 见
/// `injected/NOTICE`（署名 Microsoft/Playwright + 固定 commit）。
pub const INJECTED_SOURCE: &str = include_str!("../injected/dist/injected.js");

/// bundle 的 IIFE 全局名（与 `injected/build.sh` 的 `--global-name` 严格一致）。
/// esbuild 的 IIFE 格式把命名导出**直接**挂成该全局的属性，故
/// `new <GLOBAL>.InjectedScript(globalThis, opts)` 可直接 `new`（**不是** PW CJS bundle 的
/// `new (module.exports.InjectedScript())(...)` 双调用——那是 CJS lazy getter 形态）。
const INJECTED_GLOBAL: &str = "__nomiInjectedExports";

/// utility world 名的前缀（随机 hex 拼成 `__nomi_<hex>__`，每会话唯一）。随机化用于
/// ①防与页面自有全局/world 名冲突；②轻度反检测（DESIGN §6 binding 命名同理）。
const WORLD_NAME_PREFIX: &str = "__nomi_";

/// 注入管线错误。复用传输层 [`TransportError`] 作底层错误源；本枚举只加注入特有的语义
/// （context 未就绪 / evaluate 抛异常 / 句柄拿不到）。**绝不 panic**。
#[derive(Debug, thiserror::Error)]
pub enum InjectError {
    /// 底层 CDP 传输/会话错误。
    #[error("transport: {0}")]
    Transport(#[from] TransportError),
    /// 目标 frame 的 utility-world execution context 尚未就绪（导航中 / 还没物化）。
    /// 调用方可短重试（world 在下一拍 `executionContextCreated` 就会到）。
    #[error("utility world context not ready for frame {frame_id}")]
    ContextNotReady { frame_id: String },
    /// 页面侧 JS 抛了异常（evaluate / callFunctionOn 的 `exceptionDetails`）。
    #[error("injected script threw: {0}")]
    JsException(String),
    /// CDP 回包形状与预期不符（缺 objectId / result 等）——我方不变量问题。
    #[error("unexpected CDP shape: {0}")]
    Protocol(String),
    /// This session produced more live utility worlds than can be represented
    /// safely.  Existing entries are not evicted; callers must simplify or
    /// reload the frame tree before addressing an untracked frame.
    #[error("utility world context capacity exceeded (limit={limit})")]
    ContextCapacityExceeded { limit: usize },
    /// The current observe generation would retain more element refs than the
    /// task-wide generation budget.  The just-created JavaScript snapshot map
    /// is invalidated before this error is returned; callers must invalidate
    /// any earlier frame maps from the same in-flight observe as well.
    #[error(
        "observe ref capacity exceeded (limit={limit}, current={current}, frame_refs={required})"
    )]
    RefCapacityExceeded {
        limit: usize,
        current: usize,
        required: usize,
    },
    /// The current observe would return more serialized snapshot bytes than a
    /// single task generation may retain.  The frame's JavaScript ref map is
    /// cleared before the by-value CDP result is returned.
    #[error(
        "observe byte capacity exceeded (limit={limit}, current={current}, frame_bytes={frame_bytes})"
    )]
    ObservationCapacityExceeded {
        limit: usize,
        current: usize,
        frame_bytes: usize,
    },
}

/// A spawned task is not owned until it has been committed into its durable
/// parent.  Dropping Tokio's `JoinHandle` detaches, so every fallible
/// initialization seam wraps a new task in this guard and calls `into_inner`
/// only after all later `await`s have succeeded.
pub(crate) struct AbortOnDropTask {
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl AbortOnDropTask {
    pub(crate) fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    pub(crate) fn into_inner(mut self) -> tokio::task::JoinHandle<()> {
        self.handle
            .take()
            .expect("AbortOnDropTask handle is present until committed")
    }
}

impl Drop for AbortOnDropTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// 生成 `__nomi_<hex>__` 形态的随机名（utility world / 未来 binding 名共用此构造）。
/// 8 字节 CSPRNG → 16 hex 字符。`getrandom` 失败（极罕见）回退到时间熵——绝不 panic，
/// 名字唯一性退化但功能不破。
fn random_suffixed_name(prefix: &str) -> String {
    let mut buf = [0u8; 8];
    let hex = if getrandom::getrandom(&mut buf).is_ok() {
        hex::encode(buf)
    } else {
        // 兜底：纳秒 ^ pid，弱熵但不 panic（仅极端环境无 CSPRNG 时触发）。
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        format!("{:08x}{:08x}", nanos, std::process::id())
    };
    format!("{prefix}{hex}__")
}

/// 构造 InjectedScript 的运行时选项（对标 PW `dom.ts` 的 `InjectedScriptOptions`）。
/// **`browserName` 固定 `"chromium"`** 让 vendor bundle 里 WebKit/FF 的运行时分支
/// dead-at-runtime（DESIGN §6：整包 vendor 不 fork，靠构造期固定收敛）。
fn injected_options_json() -> Value {
    serde_json::json!({
        "isUnderTest": false,
        "sdkLanguage": "javascript",
        "testIdAttributeName": "data-testid",
        "stableRafCount": 2,
        "browserName": "chromium",
        "isUtilityWorld": true,
        "customEngines": [],
    })
}

/// 单个 page-session 的注入管线。持有 [`Connection`] 句柄（克隆友好）+ 该 page 的
/// `session_id` + utility world 名 + per-frame 的 contextId/injected-handle 缓存。
///
/// **并发**：缓存是 `Arc<Mutex<FrameWorldState>>`——既被本管线的方法访问，也被 arm 起的
/// 后台登记循环（`'static` 任务）共享同一份真相。临界区短、不跨 await 持锁；CDP I/O 全
/// 经 [`Connection`] 的 async 路径，锁只护内存表。
///
/// **`Clone`**：所有字段克隆友好（`Connection` 克隆共享底层传输；`shared` 是 `Arc<Mutex>`，
/// 克隆共享同一份 context 缓存真相）。observe 的 OOPIF 路径锁内克隆出 manager 句柄、释放锁后
/// 锁外逐帧 `.await`（避免跨 await 持 `oopif_managers` 锁阻塞 arm 循环插入）。克隆**不**复制后台
/// 登记循环——循环由原始 manager（连同其 `JoinHandle`）保活，克隆经共享 `Arc` 读同一份更新。
#[derive(Clone)]
pub struct InjectionManager {
    conn: Connection,
    /// 目标 page 的 CDP sessionId。
    session_id: String,
    /// 本会话随机化的 utility world 名（`__nomi_<hex>__`）。
    world_name: String,
    /// per-frame world 缓存（与后台登记循环共享）。
    shared: Shared,
}

/// per-frame world 缓存：frameId → 已登记的 utility context；+ 该 context 上已注入的
/// InjectedScript 实例 objectId（lazy，首次动作才填）。
#[derive(Default)]
struct FrameWorldState {
    /// frameId → utility world 的 executionContextId（数字 id，发 evaluate/callFunctionOn 用）。
    context_ids: HashMap<String, i64>,
    /// Numeric context id -> frameId.  Unlike `uniqueId`, this is also
    /// available from `Page.createIsolatedWorld` during lossy-stream rebuild.
    context_to_frame: HashMap<i64, String>,
    /// system-unique uniqueId → frameId（`executionContextDestroyed` 只给 uniqueId，
    /// 据此反查 frameId 失效该 frame 的 context + handle）。
    unique_to_frame: HashMap<String, String>,
    /// Reverse index ensures replacing a frame context also removes its old
    /// unique-id entry instead of accumulating one entry per navigation.
    frame_to_unique: HashMap<String, String>,
    /// frameId → 已注入的 InjectedScript 实例 objectId（lazy 缓存）。
    injected_handles: HashMap<String, String>,
    /// Sticky while at least one live frame could not be admitted.  Lookups for
    /// represented frames continue to work; an unrepresented frame receives a
    /// clear bounded-refusal error rather than a misleading transient miss.
    capacity_exceeded: bool,
}

impl FrameWorldState {
    fn clear(&mut self) {
        self.context_ids.clear();
        self.context_to_frame.clear();
        self.unique_to_frame.clear();
        self.frame_to_unique.clear();
        self.injected_handles.clear();
        self.capacity_exceeded = false;
    }

    fn remove_frame(&mut self, frame_id: &str) {
        if let Some(context_id) = self.context_ids.remove(frame_id) {
            self.context_to_frame.remove(&context_id);
        }
        if let Some(unique_id) = self.frame_to_unique.remove(frame_id) {
            self.unique_to_frame.remove(&unique_id);
        }
        self.injected_handles.remove(frame_id);
        // `capacity_exceeded` is deliberately sticky until an authoritative
        // clear/rebuild.  Removing one represented frame does not prove that
        // every frame rejected while the table was full has also disappeared.
        // Clearing it here would turn a real bounded-refusal condition into a
        // misleading transient ContextNotReady for those still-live frames.
    }

    fn register_context(
        &mut self,
        frame_id: String,
        context_id: i64,
        unique_id: Option<String>,
    ) -> Result<(), ()> {
        if let Some(previous_frame) = self.context_to_frame.get(&context_id).cloned()
            && previous_frame != frame_id
        {
            self.remove_frame(&previous_frame);
        }
        if !self.context_ids.contains_key(&frame_id)
            && self.context_ids.len() >= MAX_FRAME_WORLDS
        {
            self.capacity_exceeded = true;
            return Err(());
        }

        // A new document/world for the same frame supersedes all reverse
        // indexes and the old injected object handle.
        let previous_context = self.context_ids.insert(frame_id.clone(), context_id);
        if let Some(previous_context) = previous_context {
            self.context_to_frame.remove(&previous_context);
            if previous_context != context_id && unique_id.is_none()
                && let Some(previous_unique) = self.frame_to_unique.remove(&frame_id)
            {
                self.unique_to_frame.remove(&previous_unique);
            }
        }
        self.context_to_frame.insert(context_id, frame_id.clone());
        self.injected_handles.remove(&frame_id);

        if let Some(unique_id) = unique_id.filter(|id| !id.is_empty()) {
            if let Some(previous_frame) = self.unique_to_frame.get(&unique_id).cloned()
                && previous_frame != frame_id
            {
                self.remove_frame(&previous_frame);
            }
            if let Some(previous_unique) = self.frame_to_unique.insert(frame_id.clone(), unique_id.clone()) {
                self.unique_to_frame.remove(&previous_unique);
            }
            self.unique_to_frame.insert(unique_id, frame_id);
        }
        Ok(())
    }
}

/// 共享缓存的别名：本管线方法与 arm 起的 `'static` 后台登记循环共用同一份真相。
type Shared = std::sync::Arc<Mutex<FrameWorldState>>;

struct ContextLoopCleanup(Shared);

impl Drop for ContextLoopCleanup {
    fn drop(&mut self) {
        self.0.lock().unwrap().clear();
    }
}

impl InjectionManager {
    /// 新建管线（**不**自动 arm）。`session_id` 是目标 page 的 CDP sessionId。
    /// world 名按会话随机化。调用 [`InjectionManager::arm`] 才真正物化 world 并起监听。
    pub fn new(conn: Connection, session_id: impl Into<String>) -> Self {
        Self {
            conn,
            session_id: session_id.into(),
            world_name: random_suffixed_name(WORLD_NAME_PREFIX),
            shared: std::sync::Arc::new(Mutex::new(FrameWorldState::default())),
        }
    }

    /// 本会话的 utility world 名（测试 / 诊断用）。
    pub fn world_name(&self) -> &str {
        &self.world_name
    }

    /// 本管线绑定的 page/iframe CDP sessionId（observe（Task 6）填 [`crate::aria_ref::RefRecord`]
    /// 的 `session_id`、act 路由 CDP 命令时用）。
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 底层 [`Connection`] 句柄（observe 需在此 session 上发裸 DOM 命令——
    /// `DOM.getFrameOwner`/`resolveNode` 等做 iframe→子帧路由；克隆友好）。
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// 枚举本 session 下**同进程**帧树里所有 frameId（深度优先；含主帧）。
    /// observe 逐帧产 [`crate::observe::FrameSnapshot`] 时用此列帧。OOPIF 跨进程子帧不在
    /// 本 session 的 frameTree 里（它们另起子 session），故此处只覆盖同进程帧。
    pub async fn frame_ids(&self) -> Result<Vec<String>, InjectError> {
        let tree = self
            .conn
            .send::<GetFrameTreeParams>(&self.session_id, &GetFrameTreeParams::default())
            .await?;
        let mut ids = Vec::new();
        collect_frame_ids(tree.get("frameTree"), &mut ids);
        Ok(ids)
    }

    /// 本 session 的主帧 frameId（frameTree 根 frame.id；同进程子帧均以它为祖先）。
    /// page session 的主帧 id == 其 page target 的 targetId（CDP 约定），但此处以
    /// frameTree 为权威，不依赖外部传入的 targetId。
    pub async fn main_frame_id(&self) -> Result<String, InjectError> {
        let tree = self
            .conn
            .send::<GetFrameTreeParams>(&self.session_id, &GetFrameTreeParams::default())
            .await?;
        tree.get("frameTree")
            .and_then(|t| t.get("frame"))
            .and_then(|f| f.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| InjectError::Protocol(format!("getFrameTree missing root frame id: {tree}")))
    }

    /// 取某帧 `document.body` 在该帧 utility-world 里的 objectId（喂注入侧 aria 方法的 node 句柄）。
    /// 帧 context 未就绪 → `ContextNotReady`（调用方可短重试或跳过该帧）。body 为 null（空文档）→
    /// `Protocol`（调用方按需容错）。
    pub async fn body_object_id(&self, frame_id: &str) -> Result<String, InjectError> {
        let context_id = self.context_id_for(frame_id)?;
        let mut params = EvaluateParams::new("document.body".to_string());
        params.context_id = Some(ExecutionContextId::new(context_id));
        params.return_by_value = Some(false);
        params.await_promise = Some(false);
        let result = self
            .conn
            .send::<EvaluateParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .and_then(|r| r.get("objectId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                InjectError::Protocol(format!(
                    "evaluate(document.body) returned no objectId for frame {frame_id}: {result}"
                ))
            })
    }

    /// **D5 password ref 收集**：在 `frame_id` 的 utility-world contextId `Runtime.evaluate`
    /// 一段只读脚本，遍历 DOM（含 open shadow root）找出所有 **`input[type=password]`** 及
    /// **`autocomplete` 含 `password`** 的输入控件，读取注入侧 `incrementalAriaSnapshot` 给它们
    /// 打的 `_ariaRef.ref` expando，返回这些 ref 列表。observe 据此宿主侧抹掉对应 YAML 行的
    /// 内联 value（[`crate::redact::blank_secret_values`]），实现 D5「password DOM value 永不进
    /// LLM」。
    ///
    /// **为何在同一 utility world**：`_ariaRef` expando 由注入侧在该 world 写到元素上；同一
    /// world 才读得到（页面主 world 看不见）。脚本须在**快照之后**跑（此时 ref 已分配）。
    ///
    /// 信号基于 **DOM 的 `type`/`autocomplete`**（非文本启发式）。closed shadow 内的字段读不到
    /// （与 aria 快照口径一致：closed shadow 本就不进树，故其 value 也不在 YAML，无需抹）。
    /// 任一步失败 / 该帧无 password 字段 → 返回空 `Vec`（best-effort，绝不 panic）。
    pub async fn password_refs(&self, frame_id: &str) -> Result<Vec<String>, InjectError> {
        let context_id = self.context_id_for(frame_id)?;
        // 只读遍历：document + 递归 open shadow root，收 type=password / autocomplete~=password 的
        // input/textarea 的 _ariaRef.ref。无 ref（未被快照分配）的跳过。返回 by-value 字符串数组。
        let expression = r#"
            (() => {
              const refs = [];
              const isSecret = (el) => {
                const tag = el.tagName;
                if (tag !== 'INPUT' && tag !== 'TEXTAREA') return false;
                const type = (el.getAttribute('type') || '').toLowerCase();
                if (type === 'password') return true;
                const ac = (el.getAttribute('autocomplete') || '').toLowerCase();
                return ac.includes('password');
              };
              const walk = (root) => {
                let nodes;
                try { nodes = root.querySelectorAll('*'); } catch (e) { return; }
                for (const el of nodes) {
                  if (isSecret(el)) {
                    const r = el._ariaRef && el._ariaRef.ref;
                    if (r) refs.push(r);
                  }
                  if (el.shadowRoot) walk(el.shadowRoot);
                }
              };
              try { walk(document); } catch (e) {}
              return refs;
            })()
        "#;
        let mut params = EvaluateParams::new(expression.to_string());
        params.context_id = Some(ExecutionContextId::new(context_id));
        params.return_by_value = Some(true);
        params.await_promise = Some(false);
        let result = self
            .conn
            .send::<EvaluateParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        // result.result.value 是字符串数组（by-value）。形状异常 → 当作无 password 字段（空）。
        let refs = result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(refs)
    }

    /// **arm 注入管线**（生命周期步骤①②③ 的编排起点；④ 在首次动作 lazy 注入）：
    /// - **先**订阅 `executionContextCreated/Destroyed/Cleared` 并起后台登记循环
    ///   （先装监听、后物化，避免漏掉 world 创建事件——`Runtime.enable` 会补发已存在
    ///   context 的 created 事件，故监听必须在 enable **之前**装好）；
    /// - 再 `Runtime.enable` + `Page.enable`（开 executionContext* / frame 事件）；
    /// - 步骤①：空 source `addScriptToEvaluateOnNewDocument(worldName, runImmediately=false)`
    ///   让 utility world 在每新文档物化；
    /// - 步骤②：对当前 frame tree 的每个 frame `createIsolatedWorld(grantUniveralAccess=true)`
    ///   补现存页——其 `executionContextCreated` 被登记循环按 worldName 收下（步骤③）。
    ///
    /// 返回后台登记循环的 `JoinHandle`（调用方保活；连接关闭时该循环自然退出）。
    pub async fn arm(&self) -> Result<tokio::task::JoinHandle<()>, InjectError> {
        let session = self.session_id.as_str();

        // 先订阅三个 context 生命周期事件（限定本 session），再起循环，最后才 enable。
        // 顺序铁律：subscribe → spawn loop → enable → addScript → createIsolatedWorld。
        let created_rx = self
            .conn
            .subscribe("Runtime.executionContextCreated", Some(session));
        let destroyed_rx = self
            .conn
            .subscribe("Runtime.executionContextDestroyed", Some(session));
        let cleared_rx = self
            .conn
            .subscribe("Runtime.executionContextsCleared", Some(session));

        // 起后台登记循环（消费上述事件，维护 context_ids 缓存）。
        let loop_handle = AbortOnDropTask::new(
            self.spawn_context_loop(created_rx, destroyed_rx, cleared_rx),
        );

        // 现在才 enable（其补发的 executionContextCreated 会被上面的循环收到）。
        self.conn
            .send::<RuntimeEnableParams>(session, &RuntimeEnableParams::default())
            .await?;
        self.conn
            .send::<PageEnableParams>(session, &PageEnableParams::default())
            .await?;

        // 步骤①：空 source，让 utility world 每新文档物化。
        let add = AddScriptToEvaluateOnNewDocumentParams {
            source: String::new(),
            world_name: Some(self.world_name.clone()),
            include_command_line_api: None,
            run_immediately: Some(false),
        };
        self.conn
            .send::<AddScriptToEvaluateOnNewDocumentParams>(session, &add)
            .await?;

        // 步骤②：对当前每个 frame 补建 isolated world（覆盖 arm 时已打开的页面）。
        self.create_isolated_worlds_for_existing_frames().await?;

        Ok(loop_handle.into_inner())
    }

    /// 枚举当前 frame tree，对每个 frame 发 `createIsolatedWorld`。
    /// `createIsolatedWorld` 的 executionContextId is also registered directly.
    /// Normally the event loop has already registered the same id; the direct
    /// path is what lets a lossy-event rebuild recover existing worlds without
    /// waiting for Chromium to replay `executionContextCreated`.
    async fn create_isolated_worlds_for_existing_frames(&self) -> Result<(), InjectError> {
        let session = self.session_id.as_str();
        let tree = self
            .conn
            .send::<GetFrameTreeParams>(session, &GetFrameTreeParams::default())
            .await?;
        let mut frame_ids = Vec::new();
        collect_frame_ids(tree.get("frameTree"), &mut frame_ids);

        let mut capacity_exceeded = false;
        for fid in frame_ids {
            let params = CreateIsolatedWorldParams {
                frame_id: fid.clone().into(),
                world_name: Some(self.world_name.clone()),
                grant_univeral_access: Some(true),
            };
            // 单 frame 失败不致命（可能正在导航 / 已销毁）；其 world 也会在下次新文档
            // 经步骤①物化。
            match self
                .conn
                .send::<CreateIsolatedWorldParams>(session, &params)
                .await
            {
                Ok(result) => {
                    let Some(context_id) = result
                        .get("executionContextId")
                        .and_then(Value::as_i64)
                    else {
                        tracing::warn!(
                            target: "nomi_browser_engine::injected",
                            frame_id = %fid,
                            "createIsolatedWorld returned no executionContextId"
                        );
                        continue;
                    };
                    if self
                        .shared
                        .lock()
                        .unwrap()
                        .register_context(fid.clone(), context_id, None)
                        .is_err()
                    {
                        capacity_exceeded = true;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "nomi_browser_engine::injected",
                        frame_id = %fid, error = %e,
                        "createIsolatedWorld failed for existing frame (non-fatal)"
                    );
                }
            }
        }
        if capacity_exceeded {
            Err(InjectError::ContextCapacityExceeded {
                limit: MAX_FRAME_WORLDS,
            })
        } else {
            Ok(())
        }
    }

    /// 后台登记循环：消费 executionContext{Created,Destroyed,Cleared} 维护缓存。
    /// 与管线共享同一份 `Shared` 缓存；连接关闭时三路订阅 `Closed`，循环退出。
    fn spawn_context_loop(
        &self,
        mut created_rx: tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        mut destroyed_rx: tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        mut cleared_rx: tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
    ) -> tokio::task::JoinHandle<()> {
        let manager = self.clone();
        tokio::spawn(async move {
            // Aborting a JoinHandle drops this future.  The cleanup guard makes
            // cancellation/session detach release cached ids and remote handles
            // immediately instead of leaving stale state reachable through
            // cloned managers.
            let _cleanup = ContextLoopCleanup(manager.shared.clone());
            loop {
                tokio::select! {
                    ev = created_rx.recv() => match ev {
                        Ok(ev) => Self::on_context_created(&manager.shared, &manager.world_name, &ev.params),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            manager.rebuild_after_context_loss("created", skipped).await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    ev = destroyed_rx.recv() => match ev {
                        Ok(ev) => Self::on_context_destroyed(&manager.shared, &ev.params),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            manager.rebuild_after_context_loss("destroyed", skipped).await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    ev = cleared_rx.recv() => match ev {
                        Ok(_) => Self::on_contexts_cleared(&manager.shared),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            manager.rebuild_after_context_loss("cleared", skipped).await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                }
            }
        })
    }

    async fn rebuild_after_context_loss(&self, stream: &str, skipped: u64) {
        Self::on_contexts_cleared(&self.shared);
        tracing::warn!(
            target: "nomi_browser_engine::injected",
            session_id = %self.session_id,
            stream,
            skipped,
            "context lifecycle stream lagged; rebuilding bounded frame-world state"
        );
        if let Err(error) = self.create_isolated_worlds_for_existing_frames().await {
            tracing::warn!(
                target: "nomi_browser_engine::injected",
                session_id = %self.session_id,
                %error,
                "context lifecycle rebuild did not fully recover"
            );
        }
    }

    /// 登记一个新建的 execution context（仅当它是我们的 utility world）。
    fn on_context_created(
        shared: &Mutex<FrameWorldState>,
        world_name: &str,
        params: &Value,
    ) {
        let Some(ctx) = params.get("context") else { return };
        // 只认 name == 我们的 utility world 名的 context。
        if ctx.get("name").and_then(|v| v.as_str()) != Some(world_name) {
            return;
        }
        let Some(id) = ctx.get("id").and_then(|v| v.as_i64()) else { return };
        let unique_id = ctx
            .get("uniqueId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        // frameId 在 auxData.frameId。
        let frame_id = ctx
            .get("auxData")
            .and_then(|a| a.get("frameId"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if frame_id.is_empty() {
            return;
        }
        let mut g = shared.lock().unwrap();
        if g
            .register_context(frame_id.clone(), id, Some(unique_id))
            .is_err()
        {
            tracing::warn!(
                target: "nomi_browser_engine::injected",
                frame_id = %frame_id,
                limit = MAX_FRAME_WORLDS,
                "utility-world context rejected at hard capacity"
            );
        }
    }

    /// 某 context 被销毁（按 system-unique uniqueId）：失效对应 frame 的 context + handle。
    fn on_context_destroyed(shared: &Mutex<FrameWorldState>, params: &Value) {
        let unique_id = params
            .get("executionContextUniqueId")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let context_id = params
            .get("executionContextId")
            .and_then(Value::as_i64);
        let mut g = shared.lock().unwrap();
        let frame_id = unique_id
            .as_deref()
            .and_then(|id| g.unique_to_frame.get(id).cloned())
            .or_else(|| context_id.and_then(|id| g.context_to_frame.get(&id).cloned()));
        if let Some(frame_id) = frame_id {
            // Ignore a delayed destroy for an older context of a frame that has
            // already published a replacement context.
            let unique_matches = unique_id.as_deref().is_some_and(|id| {
                g.frame_to_unique.get(&frame_id).map(String::as_str) == Some(id)
            });
            let numeric_matches = context_id.is_some_and(|id| {
                g.context_ids.get(&frame_id).copied() == Some(id)
            });
            if unique_matches || numeric_matches {
                g.remove_frame(&frame_id);
            }
        }
    }

    /// 所有 context 清空（多见于主 frame 跨文档导航）：整表失效，下次用时重建。
    fn on_contexts_cleared(shared: &Mutex<FrameWorldState>) {
        shared.lock().unwrap().clear();
    }

    /// 取某 frame 的 utility-world contextId（未登记 → `ContextNotReady`，调用方可短重试）。
    /// `pub`：observe（Task 6）与契约测试需据此判 world 就绪、并在该 context 上 evaluate
    /// `document.body` 取元素句柄喂注入侧的 aria 方法。
    pub fn context_id_for(&self, frame_id: &str) -> Result<i64, InjectError> {
        let state = self.shared.lock().unwrap();
        if let Some(context_id) = state.context_ids.get(frame_id).copied() {
            return Ok(context_id);
        }
        if state.capacity_exceeded {
            Err(InjectError::ContextCapacityExceeded {
                limit: MAX_FRAME_WORLDS,
            })
        } else {
            Err(InjectError::ContextNotReady {
                frame_id: frame_id.to_string(),
            })
        }
    }

    /// 取（必要时 lazy 创建）某 frame 的 InjectedScript 实例 objectId（生命周期步骤④）。
    ///
    /// 首次：在该 frame 的 utility contextId `Runtime.evaluate`（[`INJECTED_SOURCE`] +
    /// `new <global>.InjectedScript(globalThis, opts)`），拿回 `result.objectId` 并缓存。
    /// 后续：直接返缓存。`executionContextDestroyed/Cleared` 会清缓存触发重注。
    pub async fn injected_handle(&self, frame_id: &str) -> Result<String, InjectError> {
        // 快路径：已缓存。
        if let Some(h) = self
            .shared
            .lock()
            .unwrap()
            .injected_handles
            .get(frame_id)
            .cloned()
        {
            return Ok(h);
        }

        let context_id = self.context_id_for(frame_id)?;
        let opts = injected_options_json();
        // 注入表达式：IIFE bundle 物化 <global>，然后 new 实例并作为表达式结果返回
        // （returnByValue=false → 拿 objectId 句柄）。bundle 自身是 `var <global> = (...)();`，
        // 已挂到该 world 的 globalThis；这里直接 `new <global>.InjectedScript(...)`。
        let expression = format!(
            "{src}\n(new {g}.InjectedScript(globalThis, {opts}))",
            src = INJECTED_SOURCE,
            g = INJECTED_GLOBAL,
            opts = opts,
        );

        // EvaluateParams 无 Default（含必填 expression）；用 new() 后逐字段设可选项。
        let mut params = EvaluateParams::new(expression);
        params.context_id = Some(ExecutionContextId::new(context_id));
        params.return_by_value = Some(false);
        params.await_promise = Some(false);
        // silent=false（默认即 None→不静默）：让 exceptionDetails 回来，据此报 JsException。
        let result = self
            .conn
            .send::<EvaluateParams>(&self.session_id, &params)
            .await?;

        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        let object_id = result
            .get("result")
            .and_then(|r| r.get("objectId"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                InjectError::Protocol(format!(
                    "Runtime.evaluate(new InjectedScript) returned no objectId: {result}"
                ))
            })?
            .to_string();

        let mut state = self.shared.lock().unwrap();
        if state.context_ids.get(frame_id).copied() != Some(context_id) {
            return Err(InjectError::ContextNotReady {
                frame_id: frame_id.to_string(),
            });
        }
        state
            .injected_handles
            .insert(frame_id.to_string(), object_id.clone());
        Ok(object_id)
    }

    /// **callFunctionOn 封装**（DESIGN §6 真实契约）：把 InjectedScript 实例当 receiver
    /// `this`，调它的 `method`。`args` 是 [`CallArgument`]（`value` 传字面值，`object_id`
    /// 传同 world 的元素句柄）。返回 `result` 的 [`RemoteObject`] JSON（`return_by_value`
    /// 由调用方按需——这里默认 by-value 取可序列化结果，适合 aria-snapshot 这类返字符串）。
    ///
    /// `function_declaration` = `function(...args){ return this.<method>(...args) }`。
    pub async fn call_injected(
        &self,
        frame_id: &str,
        method: &str,
        args: Vec<CallArgument>,
        return_by_value: bool,
    ) -> Result<Value, InjectError> {
        let object_id = self.injected_handle(frame_id).await?;
        let function_declaration =
            format!("function(...args) {{ return this.{method}(...args); }}");

        // CallFunctionOnParams 无 Default（含必填 functionDeclaration）；new() 后设可选项。
        let mut params = CallFunctionOnParams::new(function_declaration);
        params.object_id = Some(RemoteObjectId::new(object_id));
        params.arguments = Some(args);
        params.return_by_value = Some(return_by_value);
        params.await_promise = Some(true);
        let mut result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;

        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        // 返回 result.result（RemoteObject）。by-value 时 .value 是真实值；by-handle 时
        // .objectId 是句柄。统一返 RemoteObject 让调用方按需取。
        result
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(|| InjectError::Protocol("callFunctionOn returned no result".into()))
    }

    /// Run `incrementalAriaSnapshot` while enforcing the **task-wide** retained
    /// ref budget for the in-flight observe generation.
    ///
    /// `already_retained` is the sum of authoritative JavaScript element maps
    /// accepted from earlier frames in this same observe.  The capacity check
    /// executes inside the same JavaScript call that creates the map, so an
    /// oversized frame is invalidated before control returns to Rust.  A thrown
    /// or rejected snapshot also invalidates its partial map; unpublished refs
    /// can therefore never linger for a later `find_elements` call.
    pub async fn incremental_aria_snapshot_bounded(
        &self,
        frame_id: &str,
        node: CallArgument,
        options: CallArgument,
        already_retained: usize,
        already_retained_bytes: usize,
    ) -> Result<(Value, usize, usize), InjectError> {
        let function_declaration = r#"function(node, options, maxRetained, alreadyRetained, maxBytes, alreadyBytes) {
            const invalidate = () => {
                const snap = this._lastAriaSnapshotForQuery;
                if (snap && snap.elements) snap.elements.clear();
                // Drop the InjectedScript -> Map edge as well.  Element
                // expandos do not retain DOM nodes, and the next observe may
                // safely overwrite/reuse them in its new Rust generation.
                this._lastAriaSnapshotForQuery = undefined;
            };
            // JSON.stringify returns a UTF-16 JS string. Count the UTF-8 bytes
            // CDP/serde will actually retain without allocating a TextEncoder
            // buffer. Paired surrogates encode to four bytes; all other
            // non-ASCII code units encode to two or three bytes.
            const utf8Bytes = (text) => {
                let bytes = 0;
                for (let i = 0; i < text.length; i++) {
                    const code = text.charCodeAt(i);
                    if (code <= 0x7f) bytes += 1;
                    else if (code <= 0x7ff) bytes += 2;
                    else if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length) {
                        const low = text.charCodeAt(i + 1);
                        if (low >= 0xdc00 && low <= 0xdfff) {
                            bytes += 4;
                            i += 1;
                        } else {
                            bytes += 3;
                        }
                    } else {
                        bytes += 3;
                    }
                }
                return bytes;
            };
            const finish = (snapshot) => {
                const snap = this._lastAriaSnapshotForQuery;
                const retained = snap && snap.elements ? snap.elements.size : 0;
                if (alreadyRetained + retained > maxRetained) {
                    invalidate();
                    return {
                        ok: false,
                        error: 'ref_capacity_exceeded',
                        limit: maxRetained,
                        current: alreadyRetained + retained,
                        required: retained
                    };
                }
                // This check runs in the renderer before `returnByValue` lets
                // CDP materialize the snapshot in the Rust process. The prior
                // frame total makes this one task-generation cap, not 4 MiB
                // independently for every iframe.
                let serialized;
                try {
                    serialized = JSON.stringify(snapshot);
                } catch (error) {
                    return fail(error);
                }
                const serializedBytes = utf8Bytes(serialized);
                const currentBytes = alreadyBytes + serializedBytes;
                if (!Number.isSafeInteger(currentBytes) || currentBytes > maxBytes) {
                    invalidate();
                    return {
                        ok: false,
                        error: 'observation_capacity_exceeded',
                        limit: maxBytes,
                        current: currentBytes,
                        frameBytes: serializedBytes
                    };
                }
                return {
                    ok: true,
                    snapshot: snapshot,
                    retained: retained,
                    serializedBytes: serializedBytes
                };
            };
            const fail = (error) => {
                invalidate();
                throw error;
            };
            try {
                const result = this.incrementalAriaSnapshot(node, options);
                if (result && typeof result.then === 'function')
                    return result.then(finish, fail);
                return finish(result);
            } catch (error) {
                return fail(error);
            }
        }"#;
        let max_arg = CallArgument {
            value: Some(Value::from(crate::aria_ref::MAX_REFS_PER_GENERATION as u64)),
            ..Default::default()
        };
        let retained_arg = CallArgument {
            value: Some(Value::from(already_retained as u64)),
            ..Default::default()
        };
        let max_bytes_arg = CallArgument {
            value: Some(Value::from(
                crate::observe::MAX_OBSERVATION_RETAINED_BYTES as u64,
            )),
            ..Default::default()
        };
        let retained_bytes_arg = CallArgument {
            value: Some(Value::from(already_retained_bytes as u64)),
            ..Default::default()
        };
        let mut result = self
            .call_on_injected_handle(
                frame_id,
                function_declaration,
                vec![
                    node,
                    options,
                    max_arg,
                    retained_arg,
                    max_bytes_arg,
                    retained_bytes_arg,
                ],
                None,
                true,
            )
            .await?;
        let mut value = result
            .get_mut("value")
            .map(Value::take)
            .ok_or_else(|| {
                InjectError::Protocol(
                    "bounded incrementalAriaSnapshot returned no by-value payload".into(),
                )
            })?;
        if value.get("error").and_then(Value::as_str) == Some("ref_capacity_exceeded") {
            return Err(InjectError::RefCapacityExceeded {
                limit: value
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(crate::aria_ref::MAX_REFS_PER_GENERATION as u64)
                    as usize,
                current: value.get("current").and_then(Value::as_u64).unwrap_or(0) as usize,
                required: value.get("required").and_then(Value::as_u64).unwrap_or(0) as usize,
            });
        }
        if value.get("error").and_then(Value::as_str)
            == Some("observation_capacity_exceeded")
        {
            return Err(InjectError::ObservationCapacityExceeded {
                limit: value
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(crate::observe::MAX_OBSERVATION_RETAINED_BYTES as u64)
                    as usize,
                current: value.get("current").and_then(Value::as_u64).unwrap_or(0) as usize,
                frame_bytes: value
                    .get("frameBytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize,
            });
        }
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(InjectError::Protocol(
                "bounded incrementalAriaSnapshot returned an unexpected payload shape".into(),
            ));
        }
        let retained = value.get("retained").and_then(Value::as_u64).ok_or_else(|| {
            InjectError::Protocol(
                "bounded incrementalAriaSnapshot returned no retained count".into(),
            )
        })? as usize;
        let serialized_bytes = value
            .get("serializedBytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                InjectError::Protocol(
                    "bounded incrementalAriaSnapshot returned no serialized byte count".into(),
                )
            })? as usize;
        let snapshot = value
            .get_mut("snapshot")
            .map(Value::take)
            .ok_or_else(|| {
                InjectError::Protocol(
                    "bounded incrementalAriaSnapshot returned no snapshot".into(),
                )
            })?;
        Ok((snapshot, retained, serialized_bytes))
    }

    /// Invalidate the ref map retained by the latest snapshot for one frame.
    /// Used when another frame makes the task-wide observe generation overflow:
    /// no partially published generation remains queryable by `find_elements`.
    pub async fn clear_snapshot_refs(&self, frame_id: &str) -> Result<(), InjectError> {
        let function_declaration = r#"function() {
            const snap = this._lastAriaSnapshotForQuery;
            if (snap && snap.elements) snap.elements.clear();
            this._lastAriaSnapshotForQuery = undefined;
            return true;
        }"#;
        self.call_on_injected_handle(frame_id, function_declaration, vec![], None, true)
            .await
            .map(|_| ())
    }

    /// **callFunctionOn（自定义 function_declaration + objectGroup）** —— `call_injected`
    /// 只接受 `method` 名（拼成 `this.<method>(...)`）且不传 objectGroup；act 的 ref→element
    /// 反查（P2 命脉）需要：①内联一段自定义脚本（用注入侧 `parseSelector`/`querySelector`
    /// 跑 vendored `aria-ref=` selector engine），②给返回的元素句柄分配 **objectGroup**，让动作
    /// 结束后能一次 `releaseObjectGroup` 释放该次动作产生的全部句柄（生命周期收口，防句柄泄漏）。
    ///
    /// 与 [`Self::call_injected`] 的差异：
    /// - `function_declaration` 由调用方提供完整声明（`this` 仍是 InjectedScript 实例）；
    /// - `object_group` 非 None 时透传给 `Runtime.callFunctionOn` 的 `objectGroup` —— 结果对象
    ///   （及其传播出的句柄）归入该组，可被 [`Self::release_object_group`] 整组释放。
    ///
    /// 返回 `result` 的 [`RemoteObject`] JSON（by-handle 时 `.objectId` 是元素句柄；by-value 时
    /// `.value` 是结果；元素不存在时 `.subtype == "null"` 且无 objectId）。**绝不 panic**。
    pub async fn call_on_injected_handle(
        &self,
        frame_id: &str,
        function_declaration: &str,
        args: Vec<CallArgument>,
        object_group: Option<&str>,
        return_by_value: bool,
    ) -> Result<Value, InjectError> {
        let object_id = self.injected_handle(frame_id).await?;
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(object_id));
        params.arguments = Some(args);
        params.return_by_value = Some(return_by_value);
        params.await_promise = Some(true);
        params.object_group = object_group.map(|s| s.to_string());
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// **callFunctionOn（在某元素句柄上，读其属性）** —— act 反查链的「层③ role/name 二次校验」
    /// 用它在拿到的元素 objectId 上读 `_ariaRef`（注入侧给元素打的 expando），与 RefRecord 的
    /// role/name 比对，防 backendNodeId 复用导致静默点错。
    ///
    /// `object_id` 是同 utility world 的元素句柄；`function_declaration` 以该元素为 `this`
    /// （如 `function(){ return this._ariaRef || null; }`）。结果默认 by-value（属性是可序列化的小对象）。
    /// **绝不 panic**。
    pub async fn call_on_element(
        &self,
        object_id: &str,
        function_declaration: &str,
        return_by_value: bool,
    ) -> Result<Value, InjectError> {
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(object_id.to_string()));
        params.return_by_value = Some(return_by_value);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// 释放一个 objectGroup 下的全部远程对象句柄（`Runtime.releaseObjectGroup`）。
    /// **actionability：在元素句柄上批量判 `checkElementStates`**（DESIGN §11，设计裁决②）——
    /// vendored PW 的 `InjectedScript.prototype.checkElementStates`（injectedScript.ts:640，async）
    /// 以**实例**为 `this`、**元素 node 作首参**、`states` 数组作次参，批量判 visible/stable/enabled/
    /// editable 等，返 `undefined`（全过）| `{missingState}` | `'error:notconnected'`。
    ///
    /// **为何要 `frame_id`**：`checkElementStates` 是实例方法，必须以该帧 utility world 的
    /// InjectedScript **实例**为 `this`（[`Self::injected_handle`]）；元素 `object_id` 与该实例须**同
    /// 一 world**（act 反查产出的元素句柄正在该 utility world，见 §7）。`object_id` 作首个
    /// [`CallArgument`]（by-handle）、`states` 作次参（by-value 字符串数组）传入。
    ///
    /// `await_promise=true`（async 方法）、`return_by_value=true`（结果是 `undefined`/小对象/短字符串，
    /// 可序列化）。返回 `result` 的 [`RemoteObject`] JSON（`.value` 是 resolved 值；`undefined` 在
    /// by-value 下表现为缺 `value` 字段）。注入侧 editable 不可编辑特例经 `createStacklessError` 抛 →
    /// `exceptionDetails` → [`InjectError::JsException`]，由调用方（`actionability::check_states`）分类成
    /// `Blocked`。**绝不 panic**。
    pub async fn check_element_states(
        &self,
        frame_id: &str,
        element_object_id: &str,
        states: &[&str],
    ) -> Result<Value, InjectError> {
        let injected = self.injected_handle(frame_id).await?;
        // 函数声明：`this` = InjectedScript 实例；首参 = 元素 node（by-handle），次参 = states 数组。
        let function_declaration =
            "function(node, states) { return this.checkElementStates(node, states); }";
        let element_arg = CallArgument {
            object_id: Some(RemoteObjectId::new(element_object_id.to_string())),
            ..Default::default()
        };
        let states_value =
            Value::Array(states.iter().map(|s| Value::String((*s).to_string())).collect());
        let states_arg = CallArgument {
            value: Some(states_value),
            ..Default::default()
        };
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(injected));
        params.arguments = Some(vec![element_arg, states_arg]);
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// **fill：在元素句柄上跑 vendored PW 的 `fill(node, value)`**（C1，DESIGN §9/§11，
    /// injectedScript.ts:824）。注入 vendored `fill(node,value)` 设值并派发 input/change 事件，
    /// 返三态字符串 `'done'`/`'needsinput'`/`'error:notconnected'`。`fill` 是 InjectedScript **实例**
    /// 方法（首参元素 node，次参字符串 value），**同步**返三态字符串（不是 async，但经
    /// `await_promise=true` 无害）：
    /// - **`'done'`** —— 已完成（set-value 类控件 color/date/range/… 直接 set value + 派发
    ///   input/change；或本就无需键入）。type/set_value 据此结束，无需再 insertText。
    /// - **`'needsinput'`** —— 元素是 text/email/password/search/url/textarea/contenteditable，
    ///   `fill` 已 **focus + 全选**（`selectText`），但**值要靠调用方 `Input.insertText` 真键入**
    ///   （C1 的 type tier2）。
    /// - **`'error:notconnected'`** —— 元素已 detach（→ 调用方报 NotConnected，可重试/重拍）。
    ///
    /// 元素**类型根本不能填**（如 `input[type=file]` / 非可编辑元素）→ `fill` 内 `createStacklessError`
    /// 抛 → `exceptionDetails` → [`InjectError::JsException`]（调用方按 NonRecoverable 处理，禁重试）。
    ///
    /// 元素 `object_id` 与该帧 InjectedScript 实例须**同一 world**（act 反查产出的句柄正在该 utility
    /// world，见 §7）。`object_id` 作首个 by-handle 参数、`value` 作次参（by-value 字符串）传入。
    /// `return_by_value=true`（结果是短字符串）。返回 `result` 的 [`RemoteObject`] JSON
    /// （`.value` 是三态字符串）。**绝不 panic**。
    pub async fn fill_element(
        &self,
        frame_id: &str,
        element_object_id: &str,
        value: &str,
    ) -> Result<Value, InjectError> {
        let injected = self.injected_handle(frame_id).await?;
        let function_declaration = "function(node, value) { return this.fill(node, value); }";
        let element_arg = CallArgument {
            object_id: Some(RemoteObjectId::new(element_object_id.to_string())),
            ..Default::default()
        };
        let value_arg = CallArgument {
            value: Some(Value::String(value.to_string())),
            ..Default::default()
        };
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(injected));
        params.arguments = Some(vec![element_arg, value_arg]);
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// **select_options：在 `<select>` 句柄上跑 vendored PW 的 `selectOptions(node, options)`**
    /// （C2，DESIGN §9，injectedScript.ts:777）。注入 vendored `selectOptions` 按 value/label/index
    /// 匹配选项、设 `option.selected`、派发 input/change，返回**已选中项的 value 数组**或错误字符串。
    ///
    /// `selectOptions` 是 InjectedScript **实例**方法（首参元素 node，次参 `optionsToSelect` 数组）。
    /// C2 的 `options:Vec<String>` 每个串当成 `{ valueOrLabel: <s> }`（按 option.value **或** option.label
    /// 匹配——LLM 给的是它在 aria 里看到的可见文案/值，二者择一命中即可）。返回三态形态（by-value）：
    /// - **`string[]`**（已选 value 数组）—— 成功（多选时含全部命中项的 value）；
    /// - **`'error:notconnected'`** —— 元素 detach（→ 可重试/重拍）；
    /// - **`'error:optionsnotfound'`** —— 给的某些选项在 `<select>` 里找不到（→ 良性失败，如实 success=false）；
    /// - **`'error:optionnotenabled'`** —— 命中的 option 被 disabled（→ 良性失败）。
    ///
    /// 元素**不是 `<select>`** → `selectOptions` 内 `createStacklessError('Element is not a <select> element')`
    /// 抛 → `exceptionDetails` → [`InjectError::JsException`]（调用方按 NonRecoverable 处理，禁重试）。
    ///
    /// 元素 `object_id` 与该帧 InjectedScript 实例须**同一 world**。`object_id` 作首个 by-handle 参数、
    /// `options`（`[{valueOrLabel},...]`）作次参（by-value）传入。`return_by_value=true`（结果是
    /// 短字符串数组 / 错误字符串）。返回 `result` 的 [`RemoteObject`] JSON。**绝不 panic**。
    pub async fn select_options(
        &self,
        frame_id: &str,
        element_object_id: &str,
        options: &[String],
    ) -> Result<Value, InjectError> {
        let injected = self.injected_handle(frame_id).await?;
        let function_declaration =
            "function(node, options) { return this.selectOptions(node, options); }";
        let element_arg = CallArgument {
            object_id: Some(RemoteObjectId::new(element_object_id.to_string())),
            ..Default::default()
        };
        // 每个串当 `{ valueOrLabel: <s> }`：vendored selectOptions 按 option.value 或 option.label 匹配。
        let options_value = Value::Array(
            options
                .iter()
                .map(|s| serde_json::json!({ "valueOrLabel": s }))
                .collect(),
        );
        let options_arg = CallArgument {
            value: Some(options_value),
            ..Default::default()
        };
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(injected));
        params.arguments = Some(vec![element_arg, options_arg]);
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// **scroll_into_view：把元素滚进视口（4 alignment 逃 sticky）**（C2，DESIGN §11 设计裁决⑮）。
    /// 在元素句柄上 `Runtime.callFunctionOn` 一段内联脚本（**非** vendored，纯 DOM `scrollIntoView`），
    /// 按 `block` alignment（`"start"`/`"center"`/`"end"`/`"nearest"`）滚动并返回滚动**后**元素相对视口
    /// 的位置（`{ top, left, width, height, inViewport }`），调用方据 `inViewport` 判是否逃出 sticky 遮挡。
    ///
    /// **为何内联非 vendored**：vendored `scrollIntoViewIfNeeded` 是 async 且依赖 actionability 链；C2 的
    /// scroll 动作只需把目标带进视口的轻量原语（4 alignment 轮转在 Rust 侧编排），故用一段确定性内联
    /// `scrollIntoView({ block })` + 读回 `getBoundingClientRect`（DPR 无关——CSS 像素，与几何路径同坐标系）。
    ///
    /// `object_id` 是同 world 的元素句柄；`block` 是 alignment 字符串。`return_by_value=true`（结果是小对象）。
    /// 返回 `result` 的 [`RemoteObject`] JSON（`.value` 是位置对象）。**绝不 panic**。
    pub async fn scroll_into_view(
        &self,
        object_id: &str,
        block: &str,
    ) -> Result<Value, InjectError> {
        // 内联脚本：this = 元素；按 block alignment scrollIntoView，再读回相对视口位置 + 是否在视口内。
        // inViewport：元素的可见矩形与视口有非零交集（逃出 sticky header/footer 后中心仍可见即算）。
        let function_declaration = "function(block) { \
             try { this.scrollIntoView({ block: block, inline: 'nearest' }); } catch (e) {} \
             const r = this.getBoundingClientRect(); \
             const vw = window.innerWidth, vh = window.innerHeight; \
             const inViewport = r.bottom > 0 && r.right > 0 && r.top < vh && r.left < vw; \
             return { top: r.top, left: r.left, width: r.width, height: r.height, inViewport: inViewport }; \
         }";
        let block_arg = CallArgument {
            value: Some(Value::String(block.to_string())),
            ..Default::default()
        };
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(object_id.to_string()));
        params.arguments = Some(vec![block_arg]);
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// **find_elements：CSS 选择器查元素 + 登记可反解的 ref**（C3，DESIGN §9，injectedScript.ts:709-715）。
    ///
    /// 在 `frame_id` 的 utility world 跑一段脚本：① `document.querySelectorAll(selector)` 命中元素
    /// （含 open shadow root 不递归——与 vendored CSS engine 同口径，shadow 内由各自子树负责）；②对每个
    /// 命中元素**登记一个新 ref 到注入侧的 `_lastAriaSnapshotForQuery.elements` 缓存**——这正是 vendored
    /// `aria-ref=` selector engine（`_createAriaRefEngine`，injectedScript.ts:709-715）反解的**同一张表**，
    /// 故登记的 ref 后续能被 [`crate::backend::cdp::CdpBackend::resolve_ref_to_object`] 的层②反解回 objectId。
    ///
    /// **ref 复用 P1 范式不另造编号**：ref 生成镜像 vendored `computeAriaRef`（ariaSnapshot.ts:204-216）——
    /// 给元素打 `_ariaRef = {role, name, ref}` expando（层③ role 校验读它）+ `elements.set(ref, el)`
    /// （层② aria-ref engine 读它）。`ref = refPrefix + 'e' + n`，与 observe 的 `f<seq>e<n>` **同形**；
    /// `refPrefix` 由调用方传入（主帧 `f<seq>`，与该帧 observe 时一致），`n` 取注入侧一个**专用高位计数器**
    /// （`_nomiFindRefCounter`，与 snapshot 的 `lastRef` 互不串号；首用从一个大基数起跳，且与 `elements` 现有
    /// key 去重，杜绝撞已有 snapshot ref）。role/name 用注入实例的 `getAriaRole`/`getElementAccessibleName`
    /// （与 aria 快照同源，层③校验自洽）。
    ///
    /// **前置**：该帧须**新近 observe 过**（`_lastAriaSnapshotForQuery` 已物化）——否则无表可登记，返
    /// `'error:notobserved'`，调用方让模型先 observe（不 panic）。selector 非法 → querySelectorAll 抛 →
    /// `exceptionDetails` → [`InjectError::JsException`]（调用方按 Fatal 处理）。
    ///
    /// `return_by_value=true`：返 `{ ok: true, matches: [{ref, role, name}], total, removedRefs }` |
    /// `'error:notobserved'` | `{ok:false,error:'ref_capacity_exceeded',...}`
    /// （by-value）。`total` 是命中总数（可能 > matches.len()，若 cap 截断）。**只读零写**（不点不改 DOM；
    /// 仅给元素打 `_ariaRef` expando + 写注入侧 elements 缓存，这与 observe 给元素打 ref 同性质，非页面 DOM 改动）。
    /// `cap` 限制返回条数防超大命中爆 token（0 = 不限）。**绝不 panic**。
    pub async fn find_elements(
        &self,
        frame_id: &str,
        selector: &str,
        ref_prefix: &str,
        cap: u32,
    ) -> Result<Value, InjectError> {
        let injected = self.injected_handle(frame_id).await?;
        // this = InjectedScript 实例。querySelectorAll(parseSelector('css=...'), document) 跑 vendored
        // CSS engine（与 aria-ref engine 共享 _lastAriaSnapshotForQuery）；对每个命中登记 ref（镜像
        // computeAriaRef）。ref 计数器 _nomiFindRefCounter 从 1e6 起跳并与现有 elements key 去重，
        // 与 snapshot lastRef 互不串号。
        let function_declaration = r#"function(selector, refPrefix, cap, maxRetained) {
            const snap = this._lastAriaSnapshotForQuery;
            if (!snap || !snap.elements) return 'error:notobserved';
            let parsed;
            try { parsed = this.parseSelector('css=' + selector); }
            catch (e) { parsed = this.parseSelector(selector); }
            const hits = this.querySelectorAll(parsed, document);
            const elements = snap.elements;
            // Dynamic pages often replace nodes without another observe.  Drop
            // only entries proven disconnected; never evict a live ref merely
            // to make room.  The returned keys let Rust prune the same records.
            const removedRefs = [];
            for (const [ref, el] of elements.entries()) {
                // `_ariaRef` is a single current identity per element.  If it
                // now names another ref, this map entry is superseded and can
                // never pass the Rust role/name drift check again.
                if (!el || !el.isConnected || !el._ariaRef || el._ariaRef.ref !== ref) {
                    elements.delete(ref);
                    if (removedRefs.length < maxRetained) removedRefs.push(ref);
                }
            }
            // An oversized snapshot predating this guard is invalidated in one
            // step.  Keeping it would violate the hard retention invariant;
            // choosing arbitrary live victims would silently retarget refs.
            if (elements.size > maxRetained) {
                elements.clear();
                return { ok: false, error: 'ref_capacity_exceeded', limit: maxRetained,
                         current: 0, required: 0, reset: true, removedRefs: removedRefs };
            }
            // 专用高位计数器：从 1e6 起跳，避免与 snapshot 的 e<n>（从小自增）撞号。
            if (typeof this._nomiFindRefCounter !== 'number' || this._nomiFindRefCounter < 1000000)
                this._nomiFindRefCounter = 1000000;
            const total = hits.length;
            const limit = cap > 0 ? Math.min(cap, total) : total;
            const plans = [];
            const plannedRefs = new Set();
            let nextCounter = this._nomiFindRefCounter;
            let required = 0;
            for (let i = 0; i < limit; i++) {
                const el = hits[i];
                let role = '';
                let name = '';
                try { role = this.getAriaRole(el) || ''; } catch (e) {}
                try { name = this.getElementAccessibleName(el, false) || ''; } catch (e) {}
                // A ref is reusable only when this generation's authoritative
                // map still resolves it to this exact node.
                let ariaRef = el._ariaRef;
                if (ariaRef && ariaRef.role === role && ariaRef.name === name &&
                    elements.get(ariaRef.ref) === el) {
                    plans.push({ el: el, ariaRef: ariaRef, isNew: false });
                    continue;
                }
                if (ariaRef && elements.get(ariaRef.ref) === el) {
                    elements.delete(ariaRef.ref);
                    if (removedRefs.length < maxRetained) removedRefs.push(ariaRef.ref);
                }
                let ref;
                do { ref = refPrefix + 'e' + (++nextCounter); }
                while (elements.has(ref) || plannedRefs.has(ref));
                plannedRefs.add(ref);
                plans.push({ el: el, ariaRef: { role: role, name: name, ref: ref }, isNew: true });
                required++;
            }
            if (elements.size + required > maxRetained) {
                return { ok: false, error: 'ref_capacity_exceeded', limit: maxRetained,
                         current: elements.size, required: required, reset: false,
                         removedRefs: removedRefs };
            }
            const matches = [];
            for (const plan of plans) {
                const el = plan.el;
                const ariaRef = plan.ariaRef;
                if (plan.isNew) {
                    el._ariaRef = ariaRef;
                }
                // 登记进 aria-ref engine 反解的同一张表（resolve_ref_to_object 层② 读它）。
                elements.set(ariaRef.ref, el);
                matches.push({ ref: ariaRef.ref, role: ariaRef.role, name: ariaRef.name });
            }
            this._nomiFindRefCounter = nextCounter;
            return { ok: true, matches: matches, total: total, removedRefs: removedRefs };
        }"#;
        let selector_arg = CallArgument {
            value: Some(Value::String(selector.to_string())),
            ..Default::default()
        };
        let prefix_arg = CallArgument {
            value: Some(Value::String(ref_prefix.to_string())),
            ..Default::default()
        };
        let cap_arg = CallArgument {
            value: Some(Value::from(cap)),
            ..Default::default()
        };
        let retained_arg = CallArgument {
            value: Some(Value::from(crate::aria_ref::MAX_REFS_PER_GENERATION as u64)),
            ..Default::default()
        };
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(injected));
        params.arguments = Some(vec![selector_arg, prefix_arg, cap_arg, retained_arg]);
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// **P7B: ref_boxes — 一次 roundtrip 拿该帧所有可点击 ref 的 CSS 像素包围盒**（SoM overlay 用）。
    ///
    /// 在该帧的 InjectedScript 实例上跑一段内联脚本：遍历 `this._lastAriaSnapshotForQuery.elements`
    /// （ref→活 DOM 节点的同一张表，observe 期物化、generation 内常驻——与 [`Self::find_elements`] /
    /// aria-ref engine 读的是同一张），对每个 `isConnected` 元素取 `getBoundingClientRect()`（**CSS 像素、
    /// 视口相对、零 DPR**），返回 `{ ref: {x,y,width,height} }`。零宽/零高/不可见的退化框跳过。
    ///
    /// **前置**：该帧须**新近 observe 过**（`_lastAriaSnapshotForQuery` 已物化）——否则返 `'error:notobserved'`
    /// → 映射为空 map（best-effort：调用方拿不到框就不画 SoM，回落原始兜底，绝不致错）。**只读零写**（不点不改
    /// DOM、不打 expando），`return_by_value=true`。**绝不 panic**。坐标系：top-frame 的 viewport 即截图；
    /// 子帧需叠 iframe 偏移（方案②暂缓），故调用方目前只对主帧调本方法。
    pub async fn ref_boxes(&self, frame_id: &str) -> Result<HashMap<String, CssRect>, InjectError> {
        let injected = self.injected_handle(frame_id).await?;
        let function_declaration = r#"function() {
            const snap = this._lastAriaSnapshotForQuery;
            if (!snap || !snap.elements) return 'error:notobserved';
            const out = {};
            for (const [ref, el] of snap.elements.entries()) {
                if (!el || !el.isConnected) continue;
                let r;
                try { r = el.getBoundingClientRect(); } catch (e) { continue; }
                if (!r || !(r.width > 0) || !(r.height > 0)) continue;
                out[ref] = { x: r.left, y: r.top, width: r.width, height: r.height };
            }
            return { ok: true, boxes: out };
        }"#;
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(injected));
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        let value = result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null);
        // 'error:notobserved' (or any non-object) → empty map (best-effort, never an error).
        let Some(boxes_obj) = value.get("boxes").and_then(Value::as_object) else {
            return Ok(HashMap::new());
        };
        let mut out = HashMap::with_capacity(boxes_obj.len());
        for (ref_str, rect_v) in boxes_obj {
            let num = |k: &str| rect_v.get(k).and_then(Value::as_f64);
            if let (Some(x), Some(y), Some(width), Some(height)) =
                (num("x"), num("y"), num("width"), num("height"))
            {
                out.insert(ref_str.clone(), CssRect { x, y, width, height });
            }
        }
        Ok(out)
    }

    /// **get_dropdown_options：枚举 `<select>` 的 `<option>` 列表**（C3，DESIGN §9，只读）。
    /// 在元素句柄上跑一段内联脚本：若 `this` 是 `<select>`，枚举其 `options`，返
    /// `{ ok: true, options: [{value, label, selected, disabled}] }`；非 `<select>` → `'error:notselect'`；
    /// detach → `'error:notconnected'`。**只读零写**（不改 DOM，不派发事件——区别于 C2 的 select_options）。
    /// `return_by_value=true`（结果是小对象）。**绝不 panic**。
    pub async fn dropdown_options(&self, object_id: &str) -> Result<Value, InjectError> {
        let function_declaration = r#"function() {
            if (!this || !this.isConnected) return 'error:notconnected';
            if (this.tagName !== 'SELECT') return 'error:notselect';
            const options = [];
            for (const o of this.options) {
                options.push({
                    value: o.value,
                    label: o.label || o.textContent || '',
                    selected: !!o.selected,
                    disabled: !!o.disabled
                });
            }
            return { ok: true, options: options };
        }"#;
        let mut params = CallFunctionOnParams::new(function_declaration.to_string());
        params.object_id = Some(RemoteObjectId::new(object_id.to_string()));
        params.return_by_value = Some(true);
        params.await_promise = Some(true);
        let result = self
            .conn
            .send::<CallFunctionOnParams>(&self.session_id, &params)
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(InjectError::JsException(describe_exception(ex)));
        }
        result
            .get("result")
            .cloned()
            .ok_or_else(|| InjectError::Protocol(format!("callFunctionOn returned no result: {result}")))
    }

    /// 释放一个 objectGroup 下的全部远程对象句柄（`Runtime.releaseObjectGroup`）。
    ///
    /// （经 [`Self::call_on_injected_handle`] 分配 objectGroup 的）全部元素句柄一次释放，防 CDP
    /// 端 RemoteObject 泄漏。组名不存在/已释放是无害幂等（CDP 不报错）。**绝不 panic**。
    pub async fn release_object_group(&self, group: &str) -> Result<(), InjectError> {
        self.conn
            .send::<ReleaseObjectGroupParams>(&self.session_id, &ReleaseObjectGroupParams::new(group))
            .await?;
        Ok(())
    }

    /// Cancellation/panic fallback for action cleanup. This is synchronous and
    /// safe from `Drop`: the connection's single fixed worker owns a bounded,
    /// coalescing queue, so no per-action Tokio task is detached.
    pub(crate) fn defer_object_group_release(&self, group: &str) {
        self.conn
            .defer_object_group_release(&self.session_id, group);
    }
}

/// 深度优先收集 frame tree 里所有 frameId。`node` 是 `frameTree` JSON（或子树）。
fn collect_frame_ids(node: Option<&Value>, out: &mut Vec<String>) {
    let Some(node) = node else { return };
    if let Some(fid) = node
        .get("frame")
        .and_then(|f| f.get("id"))
        .and_then(|v| v.as_str())
    {
        out.push(fid.to_string());
    }
    if let Some(children) = node.get("childFrames").and_then(|v| v.as_array()) {
        for child in children {
            collect_frame_ids(Some(child), out);
        }
    }
}

/// 把 CDP `exceptionDetails` 压成一行可读诊断。
fn describe_exception(ex: &Value) -> String {
    // 优先 exception.description（带栈），退回 text。
    if let Some(desc) = ex
        .get("exception")
        .and_then(|e| e.get("description"))
        .and_then(|v| v.as_str())
    {
        return desc.to_string();
    }
    ex.get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown injected exception")
        .to_string()
}

// ═══════════════════════════════════════════════════════════════════════════
// 单测（无浏览器）：纯逻辑——名字随机化、frame tree 收集、异常描述、context 缓存
// 失效语义。真实注入端到端见 #[ignore] 冒烟。
// ═══════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_source_is_bundled_and_exposes_global() {
        // bundle 已 check-in 且非空，且暴露我们约定的全局名 + InjectedScript。
        assert!(INJECTED_SOURCE.len() > 50_000, "bundle too small: {}", INJECTED_SOURCE.len());
        assert!(INJECTED_SOURCE.contains(INJECTED_GLOBAL), "bundle missing global name");
        assert!(INJECTED_SOURCE.contains("InjectedScript"), "bundle missing InjectedScript");
    }

    #[test]
    fn world_name_is_random_and_prefixed() {
        let a = random_suffixed_name(WORLD_NAME_PREFIX);
        let b = random_suffixed_name(WORLD_NAME_PREFIX);
        assert!(a.starts_with("__nomi_") && a.ends_with("__"), "shape: {a}");
        assert_ne!(a, b, "two names should differ (random)");
        // `__nomi_` + 16 hex + `__` = 25 chars（CSPRNG 路径）。
        assert!(a.len() >= 17, "len: {} ({a})", a.len());
    }

    #[test]
    fn injected_options_fixes_chromium() {
        let o = injected_options_json();
        assert_eq!(o["browserName"], "chromium", "WebKit/FF branches must be dead-at-runtime");
        assert_eq!(o["isUtilityWorld"], true);
    }

    #[test]
    fn collect_frame_ids_walks_tree_depth_first() {
        let tree = serde_json::json!({
            "frame": {"id": "F0"},
            "childFrames": [
                {"frame": {"id": "F1"}},
                {"frame": {"id": "F2"}, "childFrames": [{"frame": {"id": "F3"}}]}
            ]
        });
        let mut out = Vec::new();
        collect_frame_ids(Some(&tree), &mut out);
        assert_eq!(out, vec!["F0", "F1", "F2", "F3"]);
    }

    #[test]
    fn describe_exception_prefers_description() {
        let ex = serde_json::json!({
            "text": "Uncaught",
            "exception": {"description": "TypeError: x is not a function\n  at <anonymous>"}
        });
        assert!(describe_exception(&ex).contains("TypeError"));
        let ex2 = serde_json::json!({"text": "Uncaught SyntaxError"});
        assert_eq!(describe_exception(&ex2), "Uncaught SyntaxError");
    }

    #[test]
    fn context_created_registers_only_our_world() {
        let shared: Shared = std::sync::Arc::new(Mutex::new(FrameWorldState::default()));
        let world = "__nomi_abc__";
        // 我们的 world：登记。
        InjectionManager::on_context_created(
            &shared,
            world,
            &serde_json::json!({"context": {
                "id": 7, "name": world, "uniqueId": "U7",
                "auxData": {"frameId": "F0", "isDefault": false, "type": "isolated"}
            }}),
        );
        // 别的 world（main/页面自己的）：忽略。
        InjectionManager::on_context_created(
            &shared,
            world,
            &serde_json::json!({"context": {
                "id": 8, "name": "", "uniqueId": "U8",
                "auxData": {"frameId": "F0", "isDefault": true, "type": "default"}
            }}),
        );
        let g = shared.lock().unwrap();
        assert_eq!(g.context_ids.get("F0"), Some(&7));
        assert_eq!(g.unique_to_frame.get("U7").map(|s| s.as_str()), Some("F0"));
        assert!(!g.context_ids.values().any(|&v| v == 8), "main world must not register");
    }

    #[test]
    fn context_destroyed_invalidates_by_unique_id() {
        let shared: Shared = std::sync::Arc::new(Mutex::new(FrameWorldState::default()));
        let world = "__nomi_abc__";
        InjectionManager::on_context_created(
            &shared,
            world,
            &serde_json::json!({"context": {
                "id": 7, "name": world, "uniqueId": "U7",
                "auxData": {"frameId": "F0"}
            }}),
        );
        // 模拟已 lazy 注入。
        shared.lock().unwrap().injected_handles.insert("F0".into(), "OBJ1".into());
        InjectionManager::on_context_destroyed(
            &shared,
            &serde_json::json!({"executionContextUniqueId": "U7"}),
        );
        let g = shared.lock().unwrap();
        assert!(g.context_ids.is_empty(), "context must be invalidated");
        assert!(g.context_to_frame.is_empty());
        assert!(g.injected_handles.is_empty(), "handle must be dropped on destroy");
        assert!(g.unique_to_frame.is_empty());
        assert!(g.frame_to_unique.is_empty());
    }

    #[test]
    fn delayed_destroy_for_replaced_context_does_not_remove_current_context() {
        let shared: Shared = std::sync::Arc::new(Mutex::new(FrameWorldState::default()));
        let world = "__nomi_replace__";
        for (id, unique) in [(7, "U-old"), (8, "U-current")] {
            InjectionManager::on_context_created(
                &shared,
                world,
                &serde_json::json!({"context": {
                    "id": id, "name": world, "uniqueId": unique,
                    "auxData": {"frameId": "F0"}
                }}),
            );
        }
        InjectionManager::on_context_destroyed(
            &shared,
            &serde_json::json!({
                "executionContextId": 7,
                "executionContextUniqueId": "U-old"
            }),
        );
        let state = shared.lock().unwrap();
        assert_eq!(state.context_ids.get("F0"), Some(&8));
        assert_eq!(
            state.unique_to_frame.get("U-current").map(String::as_str),
            Some("F0")
        );
        assert!(!state.unique_to_frame.contains_key("U-old"));
    }

    #[test]
    fn contexts_cleared_wipes_everything() {
        let shared: Shared = std::sync::Arc::new(Mutex::new(FrameWorldState::default()));
        {
            let mut g = shared.lock().unwrap();
            g.context_ids.insert("F0".into(), 1);
            g.context_ids.insert("F1".into(), 2);
            g.unique_to_frame.insert("U1".into(), "F0".into());
            g.injected_handles.insert("F0".into(), "OBJ".into());
        }
        InjectionManager::on_contexts_cleared(&shared);
        let g = shared.lock().unwrap();
        assert!(
            g.context_ids.is_empty()
                && g.context_to_frame.is_empty()
                && g.unique_to_frame.is_empty()
                && g.frame_to_unique.is_empty()
                && g.injected_handles.is_empty()
        );
    }

    #[test]
    fn repeated_context_replacement_does_not_accumulate_reverse_indexes() {
        let shared: Shared = std::sync::Arc::new(Mutex::new(FrameWorldState::default()));
        let world = "__nomi_churn__";
        for id in 1..=10_000i64 {
            InjectionManager::on_context_created(
                &shared,
                world,
                &serde_json::json!({"context": {
                    "id": id,
                    "name": world,
                    "uniqueId": format!("U{id}"),
                    "auxData": {"frameId": "F0"}
                }}),
            );
        }
        let g = shared.lock().unwrap();
        assert_eq!(g.context_ids.len(), 1);
        assert_eq!(g.context_to_frame.len(), 1);
        assert_eq!(g.unique_to_frame.len(), 1);
        assert_eq!(g.frame_to_unique.len(), 1);
        assert_eq!(g.context_ids.get("F0"), Some(&10_000));
        assert_eq!(g.unique_to_frame.get("U10000").map(String::as_str), Some("F0"));
    }

    #[test]
    fn context_capacity_refuses_new_frame_without_evicting_existing_frames() {
        let shared: Shared = std::sync::Arc::new(Mutex::new(FrameWorldState::default()));
        let world = "__nomi_capacity__";
        for id in 0..=MAX_FRAME_WORLDS {
            InjectionManager::on_context_created(
                &shared,
                world,
                &serde_json::json!({"context": {
                    "id": id as i64 + 1,
                    "name": world,
                    "uniqueId": format!("U{id}"),
                    "auxData": {"frameId": format!("F{id}")}
                }}),
            );
        }
        let g = shared.lock().unwrap();
        assert_eq!(g.context_ids.len(), MAX_FRAME_WORLDS);
        assert_eq!(g.context_to_frame.len(), MAX_FRAME_WORLDS);
        assert_eq!(g.unique_to_frame.len(), MAX_FRAME_WORLDS);
        assert!(g.context_ids.contains_key("F0"), "capacity never evicts");
        assert!(!g.context_ids.contains_key(&format!("F{MAX_FRAME_WORLDS}")));
        assert!(g.capacity_exceeded);
        drop(g);

        shared.lock().unwrap().remove_frame("F0");
        assert!(
            shared.lock().unwrap().capacity_exceeded,
            "one tracked-frame detach does not prove rejected live frames disappeared"
        );
    }

    #[tokio::test]
    async fn abort_on_drop_task_cancels_and_runs_context_cleanup() {
        let shared: Shared = std::sync::Arc::new(Mutex::new(FrameWorldState::default()));
        shared
            .lock()
            .unwrap()
            .register_context("F0".into(), 1, Some("U1".into()))
            .unwrap();
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let started_in_task = started.clone();
        let state_in_task = shared.clone();
        let task = tokio::spawn(async move {
            let _cleanup = ContextLoopCleanup(state_in_task);
            started_in_task.notify_one();
            std::future::pending::<()>().await;
        });
        started.notified().await;
        drop(AbortOnDropTask::new(task));
        for _ in 0..20 {
            if shared.lock().unwrap().context_ids.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let g = shared.lock().unwrap();
        assert!(g.context_ids.is_empty());
        assert!(g.unique_to_frame.is_empty());
    }

    #[tokio::test]
    async fn arm_failure_aborts_context_loop_and_drops_its_subscriptions() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake CDP");
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake CDP");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("websocket handshake");
            let request = websocket
                .next()
                .await
                .expect("Runtime.enable request")
                .expect("valid websocket message");
            let Message::Text(request) = request else {
                panic!("expected text request");
            };
            let request: Value = serde_json::from_str(request.as_ref()).unwrap();
            assert_eq!(request["method"], "Runtime.enable");
            websocket
                .send(Message::Text(
                    serde_json::json!({
                        "id": request["id"],
                        "error": {"code": -32000, "message": "forced arm failure"}
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send forced failure");
            std::future::pending::<()>().await;
        });

        let conn = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect fake CDP");
        let manager = InjectionManager::new(conn.clone(), crate::session::ROOT_SESSION);
        let world = manager.world_name().to_string();
        assert!(manager.arm().await.is_err(), "fault injection must fail arm");

        // If the pre-commit JoinHandle had merely been dropped (detached), its
        // three receivers would still consume this event and mutate the shared
        // cache.  The abort guard drops all receivers and cleanup leaves no id.
        conn.registry()
            .dispatch_message(
                &serde_json::json!({
                    "method": "Runtime.executionContextCreated",
                    "params": {"context": {
                        "id": 77,
                        "name": world,
                        "uniqueId": "U77",
                        "auxData": {"frameId": "F77"}
                    }}
                })
                .to_string(),
            )
            .expect("dispatch post-failure event");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(matches!(
            manager.context_id_for("F77"),
            Err(InjectError::ContextNotReady { .. })
        ));

        conn.shutdown().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn lossy_context_stream_rebuilds_from_authoritative_frame_tree() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake CDP");
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake CDP");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("websocket handshake");
            while let Some(message) = websocket.next().await {
                let Message::Text(request) = message.expect("valid websocket message") else {
                    continue;
                };
                let request: Value = serde_json::from_str(request.as_ref()).unwrap();
                let result = match request["method"].as_str().unwrap_or_default() {
                    "Page.getFrameTree" => serde_json::json!({
                        "frameTree": {"frame": {"id": "F-rebuilt"}}
                    }),
                    "Page.createIsolatedWorld" => serde_json::json!({
                        "executionContextId": 919
                    }),
                    method => panic!("unexpected rebuild command {method}"),
                };
                websocket
                    .send(Message::Text(
                        serde_json::json!({"id": request["id"], "result": result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .expect("send fake result");
            }
        });

        let conn = Connection::connect(&format!("ws://{address}"))
            .await
            .expect("connect fake CDP");
        let manager = InjectionManager::new(conn.clone(), crate::session::ROOT_SESSION);
        manager
            .shared
            .lock()
            .unwrap()
            .register_context("F-stale".into(), 17, Some("U-stale".into()))
            .unwrap();

        manager.rebuild_after_context_loss("created", 500).await;
        assert!(matches!(
            manager.context_id_for("F-stale"),
            Err(InjectError::ContextNotReady { .. })
        ));
        assert_eq!(manager.context_id_for("F-rebuilt").unwrap(), 919);
        let state = manager.shared.lock().unwrap();
        assert_eq!(state.context_ids.len(), 1);
        assert_eq!(state.context_to_frame.len(), 1);
        drop(state);

        conn.shutdown().await;
        let _ = server.await;
    }

    // ── 端到端注入冒烟（#[ignore]，本机 Windows 实跑）─────────────────────────
    //
    // 验证混合架构 keystone：launch → connect → flatten auto-attach → page session →
    // navigate（带内容）→ arm 注入管线（4 步）→ lazy 注入 InjectedScript →
    // callFunctionOn `ariaSnapshot(document.body, {mode:'ai'})` 拿回**非空 aria YAML**。
    //
    // 手动跑（set 系统 Chrome/Edge 的 chrome.exe，或走下载兜底）：
    //   set NOMIFUN_CHROME_BINARY=...\chrome.exe
    //   cargo nextest run -p nomi-browser-engine -- --ignored inject_aria_snapshot_smoke
    // 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。
    #[tokio::test]
    #[ignore = "需本机/打包 chrome，手动跑：set NOMIFUN_CHROME_BINARY 后 -- --ignored"]
    async fn inject_aria_snapshot_smoke() {
        use crate::launch::{launch_chrome, LaunchConfig};
        use chromiumoxide::cdp::browser_protocol::page::{
            EnableParams as PageEnable, NavigateParams,
        };
        use chromiumoxide::cdp::browser_protocol::target::{
            CreateTargetParams, EventAttachedToTarget,
        };
        use std::time::Duration;

        // 1) launch（headless）+ connect + 先 attach loop 后 enable_auto_attach。
        let chrome = crate::acquire::resolve_chrome_path(
            &std::env::temp_dir().join("nomifun-browser-data"),
            None,
        )
        .await
        .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
        let profile_root = tempfile::tempdir().expect("unique smoke profile root");
        let cfg = LaunchConfig {
            chrome_path: chrome,
            user_data_dir: profile_root.path().join("profile"),
            headful: false,
        };
        let launched = launch_chrome(&cfg, true).await.expect("launch chrome");
        // 保活精确 process/profile 清理权；transport 已交给连接。
        let (_process, conn) = launched.connect().await.expect("connect");
        let _attach_loop = conn.run_attach_loop();
        conn.enable_auto_attach().await.expect("auto attach");

        // 2) 取一个 page session（createTarget + 等其 attachedToTarget）。
        let mut attached = conn.subscribe(EventAttachedToTarget::IDENTIFIER, None);
        let create = CreateTargetParams::new("about:blank");
        let cr = conn
            .send::<CreateTargetParams>(crate::transport::ROOT_SESSION, &create)
            .await
            .expect("createTarget");
        let target_id = cr["targetId"].as_str().expect("targetId").to_string();
        let page_session = loop {
            let ev = tokio::time::timeout(Duration::from_secs(10), attached.recv())
                .await
                .expect("attach timeout")
                .expect("attach recv");
            if let Ok(att) = serde_json::from_value::<EventAttachedToTarget>(ev.params.clone()) {
                let tid: String = att.target_info.target_id.clone().into();
                if tid == target_id && att.target_info.r#type == "page" {
                    break String::from(att.session_id);
                }
            }
        };

        // 3) navigate 到带内容的页面（data: URL，含 button/textbox/checkbox 让 aria 非空）。
        conn.send::<PageEnable>(&page_session, &PageEnable::default())
            .await
            .expect("Page.enable");
        let html = "data:text/html,<body><h1>Smoke</h1>\
            <button>Submit order</button>\
            <label>Email <input type=text></label>\
            <label><input type=checkbox checked> Remember me</label></body>";
        let mut load_rx = conn.subscribe("Page.loadEventFired", Some(&page_session));
        conn.send::<NavigateParams>(&page_session, &NavigateParams::new(html))
            .await
            .expect("navigate");
        let _ = tokio::time::timeout(Duration::from_secs(15), load_rx.recv()).await;

        // 4) arm 注入管线 + 等 utility world context 就绪。
        let mgr = InjectionManager::new(conn.clone(), page_session.clone());
        let _ctx_loop = mgr.arm().await.expect("arm injection");

        // 主 frame 的 frameId = page target 的 targetId（CDP 约定）。等 context 登记。
        let frame_id = target_id.clone();
        let mut ready = false;
        for _ in 0..50 {
            if mgr.context_id_for(&frame_id).is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ready, "utility world context never registered for main frame {frame_id}");

        // 5) callFunctionOn：ariaSnapshot(document.body, {mode:'ai'})。
        //    第一个参数 document.body 需作为元素句柄传入——我们先在 utility world evaluate
        //    `document.body` 拿其 objectId，再当 CallArgument.object_id 传给 ariaSnapshot。
        let ctx_id = mgr.context_id_for(&frame_id).expect("ctx id");
        let mut body_eval = EvaluateParams::new("document.body".to_string());
        body_eval.context_id = Some(ExecutionContextId::new(ctx_id));
        body_eval.return_by_value = Some(false);
        let body_res = conn
            .send::<EvaluateParams>(&page_session, &body_eval)
            .await
            .expect("evaluate document.body");
        let body_obj_id = body_res["result"]["objectId"]
            .as_str()
            .expect("document.body objectId")
            .to_string();

        // node 参数走 objectId（同 utility world 的元素句柄）；opts 参数走 by-value。
        let node_arg = CallArgument {
            object_id: Some(RemoteObjectId::new(body_obj_id.clone())),
            ..Default::default()
        };
        let opts_arg = CallArgument {
            value: Some(serde_json::json!({"mode": "ai"})),
            ..Default::default()
        };

        let result = mgr
            .call_injected(&frame_id, "ariaSnapshot", vec![node_arg, opts_arg], true)
            .await
            .expect("call_injected ariaSnapshot");

        // by-value → result.value 是 aria-snapshot YAML 字符串。断言非空且含我们放的内容。
        let yaml = result["value"].as_str().unwrap_or("");
        eprintln!("=== aria-snapshot YAML ===\n{yaml}\n=== end ===");
        assert!(!yaml.trim().is_empty(), "aria-snapshot must be non-empty");
        assert!(
            yaml.contains("button") || yaml.contains("Submit order"),
            "aria-snapshot should mention the button; got:\n{yaml}"
        );

        // 6) Dynamic-DOM churn: every round disconnects the previous 50
        // elements and registers 50 replacements in the same observe
        // generation.  The injected map must prune disconnected nodes instead
        // of growing by 50 on every call.
        for round in 0..100u32 {
            let script = format!(
                "document.querySelectorAll('.churn').forEach(e => e.remove());\
                 for (let i=0;i<50;i++) {{ const b=document.createElement('button');\
                 b.className='churn'; b.textContent='r{round}-'+i; document.body.appendChild(b); }}"
            );
            let mut mutate = EvaluateParams::new(script);
            mutate.context_id = Some(ExecutionContextId::new(ctx_id));
            mutate.return_by_value = Some(true);
            conn.send::<EvaluateParams>(&page_session, &mutate)
                .await
                .expect("mutate churn DOM");
            let found = mgr
                .find_elements(&frame_id, ".churn", "f0", 50)
                .await
                .expect("find churn elements");
            assert_eq!(found["value"]["ok"], true, "round {round}: {found}");
        }
        // Accessible-name churn on still-connected nodes supersedes their old
        // refs.  Those stale live-node entries are also reclaimable (they can
        // no longer pass role/name drift validation) and must not accumulate.
        for round in 0..100u32 {
            let mut rename = EvaluateParams::new(format!(
                "document.querySelectorAll('.churn').forEach((e,i)=>e.textContent='rename-{round}-'+i)"
            ));
            rename.context_id = Some(ExecutionContextId::new(ctx_id));
            rename.return_by_value = Some(true);
            conn.send::<EvaluateParams>(&page_session, &rename)
                .await
                .expect("rename connected churn DOM");
            let found = mgr
                .find_elements(&frame_id, ".churn", "f0", 50)
                .await
                .expect("find renamed elements");
            assert_eq!(found["value"]["ok"], true, "rename round {round}: {found}");
        }
        let retained = mgr
            .call_on_injected_handle(
                &frame_id,
                "function(){ const es=this._lastAriaSnapshotForQuery.elements; let disconnected=0; for (const e of es.values()) if (!e || !e.isConnected) disconnected++; return {size:es.size,disconnected}; }",
                vec![],
                None,
                true,
            )
            .await
            .expect("inspect retained ref map");
        assert_eq!(retained["value"]["disconnected"], 0);
        assert!(
            retained["value"]["size"].as_u64().unwrap_or(u64::MAX)
                <= crate::aria_ref::MAX_REFS_PER_GENERATION as u64,
            "dynamic churn escaped the hard bound: {retained}"
        );

        // 7) Keep distinct nodes connected and address them in 50-element
        // batches.  Repeated calls must eventually refuse the cumulative
        // generation, with no live-ref eviction and an explicit fresh-observe
        // error shape.
        let mut add_capacity_nodes = EvaluateParams::new(
            "for(let batch=0;batch<50;batch++) for(let i=0;i<50;i++){const b=document.createElement('button');b.className='capacity';b.dataset.batch=String(batch);b.textContent='c'+batch+'-'+i;document.body.appendChild(b);}".to_string(),
        );
        add_capacity_nodes.context_id = Some(ExecutionContextId::new(ctx_id));
        add_capacity_nodes.return_by_value = Some(true);
        conn.send::<EvaluateParams>(&page_session, &add_capacity_nodes)
            .await
            .expect("add capacity DOM");
        let mut refused = None;
        for batch in 0..50u32 {
            let selector = format!(".capacity[data-batch='{batch}']");
            let found = mgr
                .find_elements(&frame_id, &selector, "f0", 50)
                .await
                .expect("find capacity batch");
            if found["value"]["error"] == "ref_capacity_exceeded" {
                refused = Some(found);
                break;
            }
        }
        let refused = refused.expect("cumulative calls must hit the generation bound");
        assert_eq!(
            refused["value"]["limit"],
            crate::aria_ref::MAX_REFS_PER_GENERATION as u64
        );
        assert_eq!(refused["value"]["reset"], false);

        // 8) Initial observe itself is subject to the same generation budget.
        // The bounded wrapper must discard an oversized vendor snapshot inside
        // the same JS call, leaving no map that a later find could reuse.
        let oversized_error = mgr
            .incremental_aria_snapshot_bounded(
                &frame_id,
                CallArgument {
                    object_id: Some(RemoteObjectId::new(body_obj_id)),
                    ..Default::default()
                },
                CallArgument {
                    value: Some(serde_json::json!({
                        "mode": "ai",
                        "refPrefix": "f0",
                        "depth": 0,
                        "track": ""
                    })),
                    ..Default::default()
                },
                0,
                0,
            )
            .await
            .expect_err("oversized initial observe must fail closed");
        assert!(matches!(
            oversized_error,
            InjectError::RefCapacityExceeded {
                limit: crate::aria_ref::MAX_REFS_PER_GENERATION,
                current,
                ..
            } if current > crate::aria_ref::MAX_REFS_PER_GENERATION
        ));
        let after_oversized = mgr
            .find_elements(&frame_id, ".capacity", "f0", 1)
            .await
            .expect("probe snapshot map after oversized observe");
        assert_eq!(after_oversized["value"], "error:notobserved");
    }
}
