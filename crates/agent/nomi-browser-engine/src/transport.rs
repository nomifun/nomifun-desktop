//! CDP 传输层：**单条 WS 连接 + sessionId 多路复用**。
//!
//! 一个浏览器进程 = 一条 `ws://127.0.0.1:<port>/devtools/browser/<id>` 连接。经
//! `Target.setAutoAttach{flatten:true}` 后，所有 target（page/OOPIF/service_worker）
//! 的命令与事件都复用这一条连接，靠 `sessionId` 区分（DESIGN §5 / spike 裁定：
//! chromiumoxide 高层 `Page::execute` 恒锁本页 session、主动 detach SW、binding 对子
//! 会话不可达——故自建薄 Handler）。
//!
//! 分层：
//! - [`Connection`] 持有 WS **写半边**（sink）+ 共享的 [`SessionRegistry`]（路由状态）+
//!   单调 CallId 计数器。后台 read loop 把 WS **读半边**收到的每条文本喂给
//!   `SessionRegistry::dispatch_message`（纯路由，见 `session.rs`）。
//! - 命令配对、短路、事件 demux 的全部纯逻辑在 `session.rs` 且已单测；本文件只接 WS I/O
//!   与 setAutoAttach 编排，真实 connect 走 `#[ignore]`（统一留 Task 7 的 launch+connect
//!   冒烟）。
//!
//! 错误：本模块自有 [`TransportError`]（定义在 `session.rs`），不耦合 `BrowserError`。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chromiumoxide::cdp::js_protocol::runtime::RunIfWaitingForDebuggerParams;
use chromiumoxide::cdp::browser_protocol::fetch::{
    EnableParams as FetchEnableParams, EventRequestPaused,
};
use chromiumoxide::cdp::browser_protocol::target::{
    EventAttachedToTarget, SetAutoAttachParams,
};
use chromiumoxide::types::{CallId, Command, MethodCall, MethodType};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};

pub use crate::session::{
    CdpEvent, CommandResult, SessionRegistry, TransportError, ROOT_SESSION,
};

/// 每条 CDP 命令的默认超时（对冲上游 hang；DESIGN §5/§22）。Task A 的 `Progress` 是
/// 更上层的取消地基；本传输层至少给每命令一个独立 deadline，绝不无限等回包。
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);


/// Hard wire-size limits for a single CDP JSON message. Screenshots and DOM
/// snapshots can be large, but an unlimited WebSocket/pipe frame lets a broken
/// renderer grow the host process without bound before JSON parsing starts.
pub const MAX_CDP_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CDP_WS_FRAME_BYTES: usize = MAX_CDP_MESSAGE_BYTES;

/// WS 写半边类型别名（split 后的 sink）。
type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>;

/// 运输写半边：CDP 协议不变,只是底层是 WS 帧还是 `--remote-debugging-pipe` 的 NUL 分隔字节流。
/// Unix 生产走 `Pipe`（浏览器在父死/管道 EOF 时自退,免疫 SIGKILL——见
/// docs/superpowers/specs/browser-use/2026-06-19-macos-pdeath-pipe-transport-design.md）;
/// Windows 的显式运行时/授权连接走 `Ws`。
enum TransportSink {
    Ws(WsSink),
    #[cfg(unix)]
    Pipe(tokio::net::unix::pipe::Sender),
}

/// 单条 CDP 连接。克隆友好（内部 `Arc`），可在多处持有以发命令 / 订阅事件。
#[derive(Clone)]
pub struct Connection {
    inner: Arc<ConnectionInner>,
    /// Handle-local response clock policy; never changes the shared transport
    /// or the ordinary write deadline used by any clone.
    response_pause: Option<tokio::sync::watch::Receiver<bool>>,
}

struct ConnectionInner {
    /// 运输写半边。`AsyncMutex` 串行化并发写。
    sink: AsyncMutex<TransportSink>,
    /// 共享路由状态（sessions / 回调 / 订阅）。read loop 与 send 路径共用。
    registry: Arc<SessionRegistry>,
    /// 单调 CallId 计数器。CDP 要求每 session 内 id 唯一；用全局单调更简单且足够。
    next_id: AtomicUsize,
    /// 每命令默认超时。
    command_timeout: Duration,
    /// **粘性防火墙 arm 标记（fail-closed 语义锚点）**：本连接上**曾经**注册过
    /// `Fetch.requestPaused` 可靠订阅者即置位，此后**永不**清除。它把
    /// `handle_attached` 的两种「无订阅者」区分开：
    /// - 从未 arm（`Launched::connect` 诊断路径）→ 跳过 `Fetch.enable`，CDP 默认
    ///   不拦截网络（与引入防火墙前一致，正确）；
    /// - **曾 arm 后消失**（防火墙任务 panic/死亡，接收端被 drop）→ **fail
    ///   closed**：拒绝放行新 session（保持 waiting-for-debugger），绝不静默退回
    ///   无拦截——那是出口防火墙的无声逃逸。恢复路径是重启 host（cdp.rs 的
    ///   watchdog 会在防火墙任务非 abort 死亡时把整条连接 fail 掉）。
    fetch_firewall_armed: AtomicBool,
    /// Supervised read-loop ownership. Explicit shutdown joins it; dropping
    /// the last connection clone aborts it so no detached task can retain the
    /// registry indefinitely.
    read_loop: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}


/// Removes a registered callback when a `send` future is cancelled or dropped
/// while it is queued on the shared sink or waiting for a response.
struct PendingCommand {
    registry: Arc<SessionRegistry>,
    session_id: String,
    call_id: CallId,
}

impl Drop for PendingCommand {
    fn drop(&mut self) {
        self.registry
            .cancel_command(&self.session_id, self.call_id);
    }
}

fn ensure_message_size(len: usize) -> Result<(), TransportError> {
    if len > MAX_CDP_MESSAGE_BYTES {
        Err(TransportError::Protocol(format!(
            "CDP message exceeds hard limit: {len} bytes > {MAX_CDP_MESSAGE_BYTES} bytes"
        )))
    } else {
        Ok(())
    }
}

impl ConnectionInner {
    async fn close_sink(&self) {
        let mut sink = self.sink.lock().await;
        match &mut *sink {
            TransportSink::Ws(ws) => {
                let _ = tokio::time::timeout(Duration::from_secs(1), ws.close()).await;
            }
            #[cfg(unix)]
            TransportSink::Pipe(pipe) => {
                use tokio::io::AsyncWriteExt;
                let _ = tokio::time::timeout(Duration::from_secs(1), pipe.shutdown()).await;
            }
        }
    }
}


impl Drop for ConnectionInner {
    fn drop(&mut self) {
        self.registry.fail_connection();
        let read_loop = self
            .read_loop
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(handle) = read_loop.take() {
            handle.abort();
        }
    }
}

impl Connection {

    /// 连接到给定的 CDP browser WebSocket URL（如
    /// `ws://127.0.0.1:9222/devtools/browser/<id>`），启动后台 read loop，并返回
    /// 已就绪的 [`Connection`]。
    ///
    /// WS 重组消息与单帧分别使用显式硬上限，避免畸形 renderer 在 JSON 解析前无限扩张
    /// transport 缓冲；上限仍为正常大 DOM / 截图保留空间。CDP 是 ws:// localhost，无需 TLS。
    ///
    /// **注意**：本方法只建连接 + 起 read loop，**不**自动 setAutoAttach。调用方在拿到
    /// 连接后显式调 [`Connection::enable_auto_attach`]（编排顺序见该方法 doc）。
    pub async fn connect(ws_url: &str) -> Result<Self, TransportError> {
        // Bound both the reassembled message and individual frames. The
        // explicit limits are shared with the pipe parser and outgoing path.
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_CDP_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_CDP_WS_FRAME_BYTES));

        let (ws, _resp) = tokio::time::timeout(
            DEFAULT_COMMAND_TIMEOUT,
            connect_async_with_config(ws_url, Some(config), false),
        )
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| TransportError::Protocol(format!("WS connect failed: {e}")))?;

        let (sink, mut stream) = ws.split();
        let registry = Arc::new(SessionRegistry::new());

        let inner = Arc::new(ConnectionInner {
            sink: AsyncMutex::new(TransportSink::Ws(sink)),
            registry: Arc::clone(&registry),
            next_id: AtomicUsize::new(1),
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            fetch_firewall_armed: AtomicBool::new(false),
            read_loop: StdMutex::new(None),
        });

        // 后台 read loop：每条文本喂纯路由；WS 关闭/出错 → poison + fatal signal。
        let reg_for_loop = Arc::clone(&registry);
        let mut fatal_for_loop = registry.subscribe_fatal();
        let weak_inner = Arc::downgrade(&inner);
        let read_loop = tokio::spawn(async move {
            let mut terminal_error = None;
            loop {
                let msg = tokio::select! {
                    biased;
                    changed = fatal_for_loop.changed() => {
                        if changed.is_ok() {
                            terminal_error = fatal_for_loop.borrow().clone();
                        }
                        break;
                    }
                    msg = stream.next() => msg,
                };
                let Some(msg) = msg else {
                    break;
                };
                match msg {
                    Ok(WsMessage::Text(text)) => {
                        if let Err(error) = ensure_message_size(text.len()) {
                            terminal_error = Some(error);
                            break;
                        }
                        if let Err(e) = reg_for_loop.dispatch_message(&text) {
                            terminal_error = Some(e);
                            break;
                        }
                    }
                    Ok(WsMessage::Binary(bytes)) => {
                        terminal_error = Some(TransportError::Protocol(format!(
                            "unexpected binary CDP WebSocket message ({} bytes)",
                            bytes.len()
                        )));
                        break;
                    }
                    Ok(WsMessage::Close(_)) => {
                        terminal_error = Some(TransportError::Closed);
                        break;
                    }
                    Err(error) => {
                        terminal_error = Some(TransportError::Protocol(format!(
                            "CDP WebSocket read failed: {error}"
                        )));
                        break;
                    }
                    // Ping/Pong/Frame：tungstenite 自动处理 ping/pong，这里无需动作。
                    _ => {}
                }
            }
            // Explicit shutdown marks the registry closed before closing the
            // sink, so it is not misreported as an abnormal read-loop exit.
            if !reg_for_loop.is_connection_closed() {
                reg_for_loop.poison_connection(terminal_error.unwrap_or(TransportError::Closed));
            }
            if let Some(inner) = weak_inner.upgrade() {
                inner.close_sink().await;
            }
        });
        *inner.read_loop.lock().unwrap() = Some(read_loop);

        Ok(Self { inner, response_pause: None })
    }

    /// 经 `--remote-debugging-pipe` 的 fd 连接（Unix）。`resp_reader` = chrome 写响应的管道读端
    /// （我们读）;`cmd_writer` = chrome 读命令的管道写端（我们写）。CDP 消息 NUL（`\0`）分隔。
    ///
    /// 不依赖端口 / DevToolsActivePort:管道即时可用。**浏览器在本进程死亡（含 SIGKILL）时,内核
    /// 关闭继承的 fd → Chromium DevTools 管道读到 EOF → 自行退出**,这是跨平台父死自清的最优解
    /// （Playwright 同款,见设计文档）。其余编排（`enable_auto_attach` 等）与 [`Connection::connect`]
    /// 完全一致——本方法只换运输,不换协议。
    #[cfg(unix)]
    pub async fn connect_pipe(
        resp_reader: std::os::fd::OwnedFd,
        cmd_writer: std::os::fd::OwnedFd,
    ) -> Result<Self, TransportError> {
        use tokio::net::unix::pipe;

        let sender = pipe::Sender::from_owned_fd(cmd_writer)
            .map_err(|e| TransportError::Protocol(format!("wrap pipe writer failed: {e}")))?;
        let mut receiver = pipe::Receiver::from_owned_fd(resp_reader)
            .map_err(|e| TransportError::Protocol(format!("wrap pipe reader failed: {e}")))?;

        let registry = Arc::new(SessionRegistry::new());
        let inner = Arc::new(ConnectionInner {
            sink: AsyncMutex::new(TransportSink::Pipe(sender)),
            registry: Arc::clone(&registry),
            next_id: AtomicUsize::new(1),
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            fetch_firewall_armed: AtomicBool::new(false),
            read_loop: StdMutex::new(None),
        });

        // 后台 read loop:逐字节按 NUL 切帧，未终止帧不得越过硬上限。
        let reg_for_loop = Arc::clone(&registry);
        let mut fatal_for_loop = registry.subscribe_fatal();
        let weak_inner = Arc::downgrade(&inner);
        let read_loop = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
            let mut chunk = vec![0u8; 64 * 1024];
            let mut terminal_error = None;
            'read_loop: loop {
                let read_result = tokio::select! {
                    biased;
                    changed = fatal_for_loop.changed() => {
                        if changed.is_ok() {
                            terminal_error = fatal_for_loop.borrow().clone();
                        }
                        break;
                    }
                    result = receiver.read(&mut chunk) => result,
                };
                match read_result {
                    Ok(0) => {
                        terminal_error = Some(if buf.is_empty() {
                            TransportError::Closed
                        } else {
                            TransportError::Protocol(format!(
                                "truncated CDP pipe message at EOF ({} bytes)",
                                buf.len()
                            ))
                        });
                        break;
                    }
                    Ok(n) => {
                        for &byte in &chunk[..n] {
                            if byte == 0 {
                                match std::str::from_utf8(&buf) {
                                    Ok(s) => {
                                        if let Err(e) = reg_for_loop.dispatch_message(s) {
                                            terminal_error = Some(e);
                                            break 'read_loop;
                                        }
                                    }
                                    Err(e) => {
                                        terminal_error = Some(TransportError::Protocol(format!(
                                            "non-UTF-8 CDP pipe message: {e}"
                                        )));
                                        break 'read_loop;
                                    }
                                }
                                buf.clear();
                            } else {
                                if buf.len() >= MAX_CDP_MESSAGE_BYTES {
                                    terminal_error = Some(TransportError::Protocol(format!(
                                        "CDP pipe message exceeds hard limit of \
                                         {MAX_CDP_MESSAGE_BYTES} bytes"
                                    )));
                                    break 'read_loop;
                                }
                                buf.push(byte);
                            }
                        }
                    }
                    Err(e) => {
                        terminal_error = Some(TransportError::Protocol(format!(
                            "CDP pipe read failed: {e}"
                        )));
                        break;
                    }
                }
            }
            if !reg_for_loop.is_connection_closed() {
                reg_for_loop.poison_connection(terminal_error.unwrap_or(TransportError::Closed));
            }
            if let Some(inner) = weak_inner.upgrade() {
                inner.close_sink().await;
            }
        });
        *inner.read_loop.lock().unwrap() = Some(read_loop);

        Ok(Self { inner, response_pause: None })
    }

    /// 从 [`launch`](crate::launch) 产物的运输连接（pipe/ws 二选一）。由显式进程 owner 使用。
    /// 与注入侧手测母本（`#[ignore]`）复用,避免各处重复 transport 分派。
    pub async fn connect_launched(
        transport: crate::launch::LaunchTransport,
    ) -> Result<Self, TransportError> {
        match transport {
            #[cfg(unix)]
            crate::launch::LaunchTransport::Pipe {
                cmd_writer,
                resp_reader,
            } => Self::connect_pipe(resp_reader, cmd_writer).await,
            crate::launch::LaunchTransport::Ws { ws_url } => Self::connect(&ws_url).await,
        }
    }

    /// 共享路由注册表的句柄（供订阅事件 / 查询 session 状态）。
    pub fn registry(&self) -> &Arc<SessionRegistry> {
        &self.inner.registry
    }

    /// Sticky signal for abnormal protocol/read-loop termination. Normal
    /// explicit [`shutdown`](Self::shutdown) closes without publishing here.
    pub fn subscribe_fatal(
        &self,
    ) -> tokio::sync::watch::Receiver<Option<TransportError>> {
        self.inner.registry.subscribe_fatal()
    }

    /// 订阅某 method（可选限定 session）的事件流。详见
    /// [`SessionRegistry::subscribe`]。
    pub fn subscribe(
        &self,
        method: impl Into<String>,
        session_id: Option<&str>,
    ) -> tokio::sync::broadcast::Receiver<CdpEvent> {
        self.inner.registry.subscribe(method, session_id)
    }

    /// Lossless control-event subscription used for events whose omission can
    /// leave a target or network request paused indefinitely.
    ///
    /// `Fetch.requestPaused` 的订阅额外**粘性置位** `fetch_firewall_armed`：从此
    /// `handle_attached` 视本连接为「防火墙曾 arm」——订阅者若消失（防火墙任务
    /// 死亡）即 fail closed，绝不静默退回无拦截（见字段 doc）。
    pub fn subscribe_reliable(
        &self,
        method: impl Into<String>,
        session_id: Option<&str>,
    ) -> tokio::sync::mpsc::UnboundedReceiver<CdpEvent> {
        let method = method.into();
        if method == EventRequestPaused::IDENTIFIER {
            self.inner.fetch_firewall_armed.store(true, Ordering::Release);
        }
        self.inner.registry.subscribe_reliable(method, session_id)
    }

    /// Receive the selected control methods in wire order through one shared,
    /// bounded reliable queue. This does not enable any browser domain.
    pub fn subscribe_reliable_sequence(&self, methods: &[&str], session_id: Option<&str>) -> tokio::sync::mpsc::UnboundedReceiver<CdpEvent> {
        if methods.contains(&EventRequestPaused::IDENTIFIER) {
            self.inner.fetch_firewall_armed.store(true, Ordering::Release);
        }
        self.inner.registry.subscribe_reliable_sequence(methods, session_id)
    }

    /// Clone only this handle with a pausable response budget. Socket writes
    /// remain bounded normally, and unrelated handles keep their own clocks.
    pub fn with_response_pause(&self, pause: tokio::sync::watch::Receiver<bool>) -> Self {
        Self { inner: Arc::clone(&self.inner), response_pause: Some(pause) }
    }


    /// 在指定 session 上发一条 CDP 命令并等回包（带每命令超时）。
    ///
    /// 流程：分配单调 CallId → 在注册表登记 oneshot（已死 session / 已关连接在此短路）→
    /// 序列化 [`MethodCall`]（serde `"sessionId"`）写 WS → 等 oneshot 与 deadline 竞速。
    /// 超时 → 清理回调并返 [`TransportError::Timeout`]。
    ///
    /// 类型参数 `C: Command` 取其 `C::IDENTIFIER`（method 名）；`session_id` 传
    /// [`ROOT_SESSION`] 即发给根 browser session。返回**原始 result JSON**（反序列化成
    /// `C::Response` 留给上层；本传输层只管路由）。
    pub async fn send<C>(&self, session_id: &str, params: &C) -> Result<serde_json::Value, TransportError>
    where
        C: Command + MethodType,
    {
        let call_id = self.alloc_id();

        // 注册回调（已死 session / 已关连接在此短路，绝不悬挂未投递回调）。
        let rx = self.inner.registry.register_command(session_id, call_id)?;
        let _pending = PendingCommand {
            registry: Arc::clone(&self.inner.registry),
            session_id: session_id.to_owned(),
            call_id,
        };
        let deadline = tokio::time::Instant::now() + self.inner.command_timeout;

        // 写 WS（失败则清理回调）。
        if let Err(e) =
            tokio::time::timeout_at(deadline, self.write_call::<C>(call_id, session_id, params))
                .await
                .map_err(|_| TransportError::Timeout)
                .and_then(|result| result)
        {
            self.inner.registry.cancel_command(session_id, call_id);
            return Err(e);
        }

        if let Some(pause) = self.response_pause.clone() {
            return wait_pausable_response(rx, deadline, pause).await;
        }
        // 等回包 vs deadline 竞速。
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(result)) => result,
            // oneshot 发送端被 drop（理论上只在连接解除时，已是 Err 结果）→ 视为 Closed。
            Ok(Err(_recv)) => Err(TransportError::Closed),
            Err(_elapsed) => {
                self.inner.registry.cancel_command(session_id, call_id);
                Err(TransportError::Timeout)
            }
        }
    }

    /// 清理类命令：对**已关/已崩 session** 或**已关连接**吞掉错误（不传播）。其余错误
    /// （超时 / CDP error / 协议错误）仍返回。用于退出路径上的尽力而为命令
    /// （如 detach 前的清理），避免「目标已经没了」时反向报错污染调用方。
    pub async fn send_may_fail<C>(&self, session_id: &str, params: &C) -> Result<(), TransportError>
    where
        C: Command + MethodType,
    {
        match self.send::<C>(session_id, params).await {
            Ok(_) => Ok(()),
            // 目标/连接已不在 → 静默成功（清理本就是为了让它消失）。
            Err(TransportError::Closed)
            | Err(TransportError::SessionClosed)
            | Err(TransportError::SessionCrashed) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// 启用 flatten 自动附着：对**根 browser session** 发
    /// `Target.setAutoAttach{auto_attach:true, wait_for_debugger_on_start:true, flatten:true}`。
    ///
    /// flatten=true 让所有子 target 复用本连接、以 `sessionId` 寻址（spike 锚点
    /// cdp.rs:106508）。`wait_for_debugger_on_start=true` 让新 target 暂停等调试器——
    /// 这给了我们「**先装监听、后放行**」的时间窗：调用方应**先**
    /// [`Connection::subscribe`]("Target.attachedToTarget", None)，**再**调本方法；
    /// 隔离 page owner 消费 attach 事件、装好该子 session 的监听，**最后**对它发
    /// `Runtime.runIfWaitingForDebugger` 放行（否则尤其 service_worker 永久卡）。
    pub(crate) async fn enable_auto_attach(&self) -> Result<(), TransportError> {
        let params = SetAutoAttachParams::builder()
            .auto_attach(true)
            .wait_for_debugger_on_start(true)
            .flatten(true)
            .build()
            .map_err(|e| {
                TransportError::Protocol(format!("SetAutoAttachParams build failed: {e}"))
            })?;
        self.send::<SetAutoAttachParams>(ROOT_SESSION, &params).await?;
        Ok(())
    }

    /// Handle an already registered attach envelope for an isolated page owner.
    /// Verify exact live identity, arm interception, then resume the target.
    ///
    /// **spike 坑（务必遵守）**：**不**对 service_worker 主动 `detachFromTarget`
    /// （chromiumoxide 的写法）——SW 出口流量防火墙（P2/P3）需保持对 SW 的 attach。
    /// 此方法不授予业务权限，也不创建、重新登记或关闭 target。
    ///
    /// 调用方应在 `enable_auto_attach` **之前**就订阅好 attach 事件，再把每个收到的
    /// 事件交给本方法。这保证「先装监听后放行」：放行（runIfWaitingForDebugger）发生
    /// 在子 session 已登记之后，故该子 session 上的后续事件不会丢。
    pub(crate) async fn handle_attached(&self, event: &EventAttachedToTarget) -> Result<(), TransportError> {
        let sid: String = event.session_id.clone().into();
        let ttype = event.target_info.r#type.clone();
        let target_id: String = event.target_info.target_id.clone().into();

        // Registration happened in the read loop before event publication.
        // Never re-register a stale queued attach or close a target here: the
        // isolated page owner has its own target/process cleanup authority.
        if !self.inner.registry.attached_session_matches(&sid, &target_id, &ttype) {
            return Err(TransportError::SessionClosed);
        }


        // 2) 在放行 target 前安装出口防火墙——**但仅当 Fetch.requestPaused 已有可靠
        //    订阅者**（F1）。无订阅者时的事件被静默丢弃且 CDP 不会重发 requestPaused，
        //    此时挂 Fetch.enable 会把该 session 的全部网络请求永久卡死（比无防火墙更糟
        //    且不可恢复）。启用流量拦截的 owner 必须在 attach 前注册可靠订阅；
        //    低层诊断路径（Launched::connect）无防火墙循环，保持 CDP 默认
        //    （不拦截）网络——与引入 Fetch.enable 前的行为一致。
        //    若 Fetch.enable 失败，保持 waiting-for-debugger 状态即为 fail-closed，
        //    绝不能让首批请求绕过策略。
        //    **防火墙曾 arm 后消失 ≠ 从未 arm**：`fetch_firewall_armed` 粘性标记
        //    区分二者——防火墙任务死亡（订阅者被 drop）时对新 session **fail
        //    closed**（报错、不放行），绝不静默跳过 Fetch.enable（无声出口逃逸）。
        if self
            .inner
            .registry
            .has_reliable_subscriber(EventRequestPaused::IDENTIFIER)
        {
            let fetch = FetchEnableParams::default();
            self.send::<FetchEnableParams>(&sid, &fetch).await?;
        } else if self.inner.fetch_firewall_armed.load(Ordering::Acquire) {
            return Err(TransportError::Protocol(format!(
                "egress firewall subscriber vanished after arming; refusing to release \
                 attached target (session {sid}) without interception (fail-closed)"
            )));
        }

        // 3) **级联 setAutoAttach 到 page/iframe 子 session**：OOPIF（跨进程 iframe）只在其**所属帧的
        //    session**上设了 setAutoAttach 才会自动 attach——browser-root 级 setAutoAttach 只覆盖顶层
        //    page,不覆盖其跨进程子帧（实测：headful + site-isolation 下 Chrome 确建了 type=="iframe"
        //    target,但缺本级联时引擎收不到它的 attachedToTarget → `spawn_oopif_arm_loop` 永不 arm →
        //    OOPIF 内容不缝合。见 docs/.../PLATFORM-VERIFICATION.md「macOS 校验结果」OOPIF 段）。
        //    须在 runIfWaitingForDebugger 放行**前**设,否则帧恢复后加载 OOPIF 时可能漏 attach。
        //    best-effort（page 可能已 detach;缺失退化为不缝该 OOPIF,不阻断主流程）。iframe 也级联以
        //    覆盖嵌套 OOPIF。
        if ttype == "page" || ttype == "iframe" {
            if let Ok(params) = SetAutoAttachParams::builder()
                .auto_attach(true)
                .wait_for_debugger_on_start(true)
                .flatten(true)
                .build()
            {
                let _ = self
                    .send_may_fail::<SetAutoAttachParams>(&sid, &params)
                    .await;
            }
        }

        // 4) 仅当该 target 在等调试器时才放行（waitForDebuggerOnStart=true 的产物）。
        //    放行命令用 send_may_fail：target 可能在我们处理前就 detach 了，吞掉即可。
        if event.waiting_for_debugger {
            let run = RunIfWaitingForDebuggerParams::default();
            self.send_may_fail::<RunIfWaitingForDebuggerParams>(&sid, &run)
                .await?;
        }
        Ok(())
    }



    /// Close the transport explicitly and release every pending command/event
    /// subscriber. This is idempotent and intentionally best-effort: process
    /// teardown remains authoritative even if a WebSocket close handshake or
    /// pipe shutdown cannot complete.
    pub async fn shutdown(&self) {
        self.inner.registry.fail_connection();
        self.inner.close_sink().await;
        let read_loop = self.inner.read_loop.lock().unwrap().take();
        if let Some(mut handle) = read_loop {
            if tokio::time::timeout(Duration::from_secs(1), &mut handle)
                .await
                .is_err()
            {
                handle.abort();
                let _ = handle.await;
            }
        }
    }

    /// 分配下一个单调 CallId。
    fn alloc_id(&self) -> CallId {
        CallId::new(self.inner.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// 序列化一条 [`MethodCall`] 并写入 WS sink。
    async fn write_call<C>(
        &self,
        call_id: CallId,
        session_id: &str,
        params: &C,
    ) -> Result<(), TransportError>
    where
        C: Command + MethodType,
    {
        let params_value = serde_json::to_value(params)
            .map_err(|e| TransportError::Protocol(format!("serialize params failed: {e}")))?;

        // 根 session 不带 sessionId 字段（MethodCall 的 skip_serializing_if）。
        let session_field = if session_id == ROOT_SESSION {
            None
        } else {
            Some(session_id.to_string())
        };

        let call = MethodCall {
            id: call_id,
            method: <C as MethodType>::method_id(),
            session_id: session_field,
            params: params_value,
        };

        let text = serde_json::to_string(&call)
            .map_err(|e| TransportError::Protocol(format!("serialize MethodCall failed: {e}")))?;
        ensure_message_size(text.len())?;

        let mut sink = self.inner.sink.lock().await;
        match &mut *sink {
            TransportSink::Ws(s) => s
                .send(WsMessage::Text(text.into()))
                .await
                .map_err(|e| TransportError::Protocol(format!("WS send failed: {e}")))?,
            #[cfg(unix)]
            TransportSink::Pipe(p) => {
                use tokio::io::AsyncWriteExt;
                // CDP 管道协议：每条消息 = JSON + 单个 NUL（`\0`）分隔符。
                p.write_all(text.as_bytes())
                    .await
                    .map_err(|e| TransportError::Protocol(format!("pipe write failed: {e}")))?;
                p.write_all(b"\0")
                    .await
                    .map_err(|e| TransportError::Protocol(format!("pipe write failed: {e}")))?;
            }
        }
        Ok(())
    }
}

async fn wait_pausable_response(
    mut response: tokio::sync::oneshot::Receiver<CommandResult>,
    deadline: tokio::time::Instant,
    mut pause: tokio::sync::watch::Receiver<bool>,
) -> CommandResult {
    let mut remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    loop {
        let paused = *pause.borrow_and_update();
        let started = tokio::time::Instant::now();
        let changed = if paused {
            tokio::select! {
                biased;
                result = &mut response => return result.unwrap_or(Err(TransportError::Closed)),
                changed = pause.changed() => changed,
            }
        } else {
            let changed = tokio::select! {
                biased;
                result = &mut response => return result.unwrap_or(Err(TransportError::Closed)),
                changed = pause.changed() => changed,
                _ = tokio::time::sleep(remaining) => return Err(TransportError::Timeout),
            };
            remaining = remaining.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(TransportError::Timeout);
            }
            changed
        };
        if changed.is_err() {
            // Losing the owner of the pause signal cannot create an infinite
            // wait. Resume the remaining response budget without resetting it.
            return match tokio::time::timeout(remaining, response).await {
                Ok(result) => result.unwrap_or(Err(TransportError::Closed)),
                Err(_) => Err(TransportError::Timeout),
            };
        }
    }
}

#[cfg(test)]
#[path = "transport_dialog_tests.rs"]
mod dialog_support_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use chromiumoxide::cdp::browser_protocol::target::EventAttachedToTarget;

    /// 迷你 CDP fake：单客户端 WS server，对**每条**命令回 `{"id":id,"result":{}}`
    /// 并按序记录请求原文。客户端 shutdown 后 join 拿全部请求做断言。
    async fn recording_fake_ws_server() -> (
        String,
        tokio::task::JoinHandle<Vec<serde_json::Value>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind transport fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let mut requests = Vec::new();
            while let Some(Ok(message)) = websocket.next().await {
                let WsMessage::Text(text) = message else {
                    break;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("fake received valid json");
                let id = request["id"].as_u64().expect("fake request has id");
                // 回包须回显 sessionId，否则子 session 命令的回调路由不到。
                let result = serde_json::json!({});
                let mut response = serde_json::json!({ "id": id, "result": result });
                if let Some(session_id) = request.get("sessionId") {
                    response["sessionId"] = session_id.clone();
                }
                requests.push(request);
                websocket
                    .send(WsMessage::Text(response.to_string().into()))
                    .await
                    .expect("fake sends generic success");
            }
            requests
        });
        (format!("ws://{address}"), server)
    }

    async fn reliable_overflow_fake_ws_server() -> (
        String,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind overflow fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            let _ = release_rx.await;
            for index in 0..=crate::session::RELIABLE_EVENT_CAPACITY {
                let event = serde_json::json!({
                    "method": EventRequestPaused::IDENTIFIER,
                    "params": { "index": index }
                });
                if websocket
                    .send(WsMessage::Text(event.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            while websocket.next().await.is_some() {}
        });
        (format!("ws://{address}"), release_tx, server)
    }


    async fn nonresponding_fake_ws_server() -> (
        String,
        tokio::sync::oneshot::Receiver<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cancellation fake websocket");
        let address = listener.local_addr().expect("read fake websocket address");
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept fake client");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete fake websocket handshake");
            if matches!(websocket.next().await, Some(Ok(WsMessage::Text(_)))) {
                let _ = seen_tx.send(());
            }
            while websocket.next().await.is_some() {}
        });
        (format!("ws://{address}"), seen_rx, server)
    }

    fn page_attach_event(session_id: &str) -> EventAttachedToTarget {
        let event_json = format!(
            r#"{{"sessionId":"{session_id}","targetInfo":{{"targetId":"T-{session_id}","type":"page","title":"","url":"","attached":true,"canAccessOpener":false}},"waitingForDebugger":true}}"#
        );
        serde_json::from_str(&event_json).expect("valid attach event")
    }

    fn register_attach_event(conn: &Connection, event: &EventAttachedToTarget) {
        conn.registry().dispatch_message(&serde_json::json!({
            "method":"Target.attachedToTarget", "params":event
        }).to_string()).unwrap();
    }

    fn service_worker_attach_event(
        session_id: &str,
        target_id: &str,
    ) -> EventAttachedToTarget {
        serde_json::from_value(serde_json::json!({
            "sessionId": session_id,
            "targetInfo": {
                "targetId": target_id,
                "type": "service_worker",
                "title": "",
                "url": "https://attacker.invalid/sw.js",
                "attached": true,
                "canAccessOpener": false
            },
            "waitingForDebugger": true
        }))
        .expect("valid service-worker attach event")
    }

    #[test]
    fn cdp_wire_message_size_has_an_explicit_hard_limit() {
        assert!(ensure_message_size(MAX_CDP_MESSAGE_BYTES).is_ok());
        let error = ensure_message_size(MAX_CDP_MESSAGE_BYTES + 1)
            .expect_err("one byte over the wire limit must be rejected");
        assert!(matches!(error, TransportError::Protocol(_)));
    }

    #[tokio::test]
    async fn explicit_shutdown_joins_read_loop_without_fatal_signal() {
        let (ws_url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");
        let fatal = conn.subscribe_fatal();

        conn.shutdown().await;

        assert!(fatal.borrow().is_none());
        assert!(conn.inner.read_loop.lock().unwrap().is_none());
        let _ = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake server must observe transport shutdown")
            .expect("fake server joins");
    }

    #[tokio::test]
    async fn dropping_last_connection_closes_external_registry_receivers() {
        let (ws_url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");
        let registry = Arc::clone(conn.registry());
        let mut events = registry.subscribe("Page.lifecycleEvent", None);

        drop(conn);

        assert!(registry.is_connection_closed());
        assert!(matches!(
            events.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
        let _ = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("dropping the transport must close the fake socket")
            .expect("fake server joins");
    }

    #[tokio::test]
    async fn cancelled_send_future_removes_its_pending_callback() {
        let (ws_url, seen, server) = nonresponding_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");
        let sender = conn.clone();
        let send_task = tokio::spawn(async move {
            let params = SetAutoAttachParams::builder()
                .auto_attach(true)
                .wait_for_debugger_on_start(true)
                .flatten(true)
                .build()
                .unwrap();
            sender.send::<SetAutoAttachParams>(ROOT_SESSION, &params).await
        });
        tokio::time::timeout(Duration::from_secs(2), seen)
            .await
            .expect("fake server must receive the command")
            .expect("command observation sender remains live");
        assert_eq!(conn.registry().pending_callback_count(), 1);

        send_task.abort();
        let _ = send_task.await;
        tokio::task::yield_now().await;
        assert_eq!(
            conn.registry().pending_callback_count(),
            0,
            "PendingCommand::drop must unregister a cancelled send"
        );

        conn.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake server must observe cancellation-test shutdown")
            .expect("fake server joins");
    }

    #[tokio::test]
    async fn external_registry_poison_wakes_and_stops_read_loop() {
        let (ws_url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");
        let expected = TransportError::Protocol("forced invariant failure".to_owned());
        let fatal = conn.subscribe_fatal();

        conn.registry().poison_connection(expected.clone());

        assert_eq!(fatal.borrow().as_ref(), Some(&expected));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let finished = conn
                    .inner
                    .read_loop
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(tokio::task::JoinHandle::is_finished);
                if finished {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fatal signal must stop the supervised read loop");
        conn.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake server must observe fatal sink shutdown")
            .expect("fake server joins");
    }

    #[tokio::test]
    async fn reliable_overflow_emits_fatal_and_stops_the_read_loop() {
        let (ws_url, release, server) = reliable_overflow_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");
        let _events = conn.subscribe_reliable(EventRequestPaused::IDENTIFIER, None);
        let mut fatal = conn.subscribe_fatal();
        release.send(()).expect("release overflow producer");

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if fatal.borrow().is_some() {
                    break;
                }
                fatal.changed().await.expect("fatal sender remains live");
            }
        })
        .await
        .expect("overflow must publish a fatal signal");
        assert!(matches!(
            fatal.borrow().as_ref(),
            Some(TransportError::Protocol(_))
        ));
        assert!(conn.registry().is_connection_closed());

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let finished = conn
                    .inner
                    .read_loop
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(tokio::task::JoinHandle::is_finished);
                if finished {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fatal overflow must stop the read loop");

        conn.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("fake server must observe fail-closed sink shutdown")
            .expect("fake server joins");
    }


    /// **F18 回归**：无任何 `Fetch.requestPaused` 可靠订阅者（`Launched::connect`
    /// 诊断路径的形态）时，`handle_attached` **绝不能**发 `Fetch.enable`——否则
    /// 被拦请求的事件无人消费即被丢弃，该 target 的网络永久卡死。
    #[tokio::test]
    async fn handle_attached_without_paused_consumer_does_not_arm_fetch() {
        let (ws_url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");

        register_attach_event(&conn, &page_attach_event("S1"));
        conn.handle_attached(&page_attach_event("S1"))
            .await
            .expect("handle_attached succeeds");
        assert!(conn.registry().has_session("S1"));

        conn.shutdown().await;
        let requests = server.await.expect("fake server joins");
        let methods: Vec<&str> = requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect();
        assert!(
            !methods.contains(&"Fetch.enable"),
            "no requestPaused consumer exists, so Fetch.enable must not be issued: {methods:?}"
        );
        assert!(
            methods.contains(&"Runtime.runIfWaitingForDebugger"),
            "the target must still be released: {methods:?}"
        );
    }

    /// **F1 不变量**：存在 `Fetch.requestPaused` 可靠订阅者（生产防火墙形态——
    /// 构造器在 attach loop 之前注册）时，`handle_attached` 在放行
    /// （runIfWaitingForDebugger）**之前**发 `Fetch.enable`（fail-closed 顺序）。
    #[tokio::test]
    async fn handle_attached_arms_fetch_before_release_when_firewall_subscribed() {
        let (ws_url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");
        // 模拟生产编排：防火墙的可靠订阅先于 attach 处理注册。
        let _paused_rx = conn.subscribe_reliable(EventRequestPaused::IDENTIFIER, None);

        register_attach_event(&conn, &page_attach_event("S2"));
        conn.handle_attached(&page_attach_event("S2"))
            .await
            .expect("handle_attached succeeds");

        conn.shutdown().await;
        let requests = server.await.expect("fake server joins");
        let methods: Vec<&str> = requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect();
        let fetch_at = methods
            .iter()
            .position(|method| *method == "Fetch.enable")
            .expect("Fetch.enable must be issued when a paused consumer exists");
        let release_at = methods
            .iter()
            .position(|method| *method == "Runtime.runIfWaitingForDebugger")
            .expect("target release must follow");
        assert!(
            fetch_at < release_at,
            "firewall must be armed before the target is released: {methods:?}"
        );
        assert_eq!(
            requests[fetch_at]["sessionId"], "S2",
            "Fetch.enable must target the attached child session"
        );
    }

    /// **防火墙死亡 fail-closed**：一旦本连接上注册过 `Fetch.requestPaused` 可靠
    /// 订阅者（生产防火墙已 arm），此后订阅者消失（防火墙任务 panic/死亡，接收端
    /// 被 drop）时，`handle_attached` 对新 session **必须失败**——绝不能退回
    /// 「静默跳过 Fetch.enable + 照常放行」（那是无声出口逃逸）。目标保持
    /// waiting-for-debugger 即 fail-closed；恢复路径是重启 host。
    #[tokio::test]
    async fn handle_attached_fails_closed_when_armed_firewall_subscriber_vanishes() {
        let (ws_url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&ws_url).await.expect("connect fake");
        // 生产编排：防火墙先注册可靠订阅（armed）……随后其任务死亡（接收端 drop）。
        let paused_rx = conn.subscribe_reliable(EventRequestPaused::IDENTIFIER, None);
        drop(paused_rx);

        register_attach_event(&conn, &page_attach_event("S3"));

        let error = conn
            .handle_attached(&page_attach_event("S3"))
            .await
            .expect_err("armed firewall vanished: handle_attached must fail closed");
        assert!(
            matches!(error, TransportError::Protocol(_)),
            "expected a protocol error surfacing the vanished firewall, got: {error:?}"
        );

        conn.shutdown().await;
        let requests = server.await.expect("fake server joins");
        let methods: Vec<&str> = requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect();
        assert!(
            !methods.contains(&"Runtime.runIfWaitingForDebugger"),
            "the target must NOT be released without interception: {methods:?}"
        );
        assert!(
            !methods.contains(&"Fetch.enable"),
            "Fetch.enable without a consumer would wedge the session; it must not be issued: {methods:?}"
        );
    }

    /// MethodCall wire 形态：根 session 不带 sessionId 字段，method/id/params 正确。
    /// 这验证我们发出去的 JSON 与 CDP 期望一致（不需真 WS）。
    #[test]
    fn method_call_serializes_without_session_for_root() {
        let params = SetAutoAttachParams::builder()
            .auto_attach(true)
            .wait_for_debugger_on_start(true)
            .flatten(true)
            .build()
            .unwrap();
        let call = MethodCall {
            id: CallId::new(1),
            method: SetAutoAttachParams::IDENTIFIER.into(),
            session_id: None,
            params: serde_json::to_value(&params).unwrap(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["method"], "Target.setAutoAttach");
        assert!(v.get("sessionId").is_none(), "root call must omit sessionId");
        assert_eq!(v["params"]["autoAttach"], true);
        assert_eq!(v["params"]["waitForDebuggerOnStart"], true);
        assert_eq!(v["params"]["flatten"], true);
    }

    /// MethodCall wire 形态：子 session 带 sessionId 字段。
    #[test]
    fn method_call_serializes_with_session_for_child() {
        let call = MethodCall {
            id: CallId::new(5),
            method: RunIfWaitingForDebuggerParams::IDENTIFIER.into(),
            session_id: Some("S1".to_string()),
            params: serde_json::to_value(RunIfWaitingForDebuggerParams::default()).unwrap(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
        assert_eq!(v["sessionId"], "S1");
        assert_eq!(v["method"], "Runtime.runIfWaitingForDebugger");
    }

    #[tokio::test]
    async fn stale_or_mismatched_attach_work_never_registers_or_closes_targets() {
        let (url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&url).await.unwrap();
        let event = page_attach_event("retired");
        register_attach_event(&conn, &event);
        conn.registry().fail_session("retired", false);
        assert_eq!(conn.handle_attached(&event).await, Err(TransportError::SessionClosed));
        assert!(!conn.registry().has_session("retired"));
        let live = page_attach_event("live");
        register_attach_event(&conn, &live);
        let mismatch = service_worker_attach_event("live", "different-target");
        assert_eq!(conn.handle_attached(&mismatch).await, Err(TransportError::SessionClosed));
        assert!(!conn.registry().is_connection_closed());
        conn.shutdown().await;
        assert!(server.await.unwrap().is_empty(), "no close, detach, or resume for unproven targets");
    }

    #[tokio::test]
    async fn registered_service_worker_is_resumed_without_target_ownership() {
        let (url, server) = recording_fake_ws_server().await;
        let conn = Connection::connect(&url).await.unwrap();
        let event = service_worker_attach_event("worker", "worker-target");
        register_attach_event(&conn, &event);
        conn.handle_attached(&event).await.unwrap();
        assert!(conn.registry().has_session("worker"));
        conn.shutdown().await;
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["method"], "Runtime.runIfWaitingForDebugger");
        assert_eq!(requests[0]["sessionId"], "worker");
    }

}
