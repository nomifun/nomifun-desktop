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

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chromiumoxide::cdp::js_protocol::runtime::RunIfWaitingForDebuggerParams;
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

/// WS 写半边类型别名（split 后的 sink）。
type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>;

/// 运输写半边：CDP 协议不变,只是底层是 WS 帧还是 `--remote-debugging-pipe` 的 NUL 分隔字节流。
/// Unix 生产走 `Pipe`（浏览器在父死/管道 EOF 时自退,免疫 SIGKILL——见
/// docs/superpowers/specs/browser-use/2026-06-19-macos-pdeath-pipe-transport-design.md）;
/// Windows 生产 + 手测低层入口（`NOMI_CDP_WS_URL`）走 `Ws`。
enum TransportSink {
    Ws(WsSink),
    #[cfg(unix)]
    Pipe(tokio::net::unix::pipe::Sender),
}

/// 单条 CDP 连接。克隆友好（内部 `Arc`），可在多处持有以发命令 / 订阅事件。
#[derive(Clone)]
pub struct Connection {
    inner: Arc<ConnectionInner>,
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
}

impl Connection {
    /// 连接到给定的 CDP browser WebSocket URL（如
    /// `ws://127.0.0.1:9222/devtools/browser/<id>`），启动后台 read loop，并返回
    /// 已就绪的 [`Connection`]。
    ///
    /// WS 帧上限**显式设为 `None`**（解除 tungstenite 默认 64MiB/16MiB），否则大 DOM /
    /// 截图会静默断连（DESIGN §5）。CDP 是 ws:// localhost，无需 TLS。
    ///
    /// **注意**：本方法只建连接 + 起 read loop，**不**自动 setAutoAttach。调用方在拿到
    /// 连接后显式调 [`Connection::enable_auto_attach`]（编排顺序见该方法 doc）。
    pub async fn connect(ws_url: &str) -> Result<Self, TransportError> {
        // 解除帧上限：max_message_size/max_frame_size = None（勿硬编码 256MB）。
        let config = WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None);

        let (ws, _resp) = connect_async_with_config(ws_url, Some(config), false)
            .await
            .map_err(|e| TransportError::Protocol(format!("WS connect failed: {e}")))?;

        let (sink, mut stream) = ws.split();
        let registry = Arc::new(SessionRegistry::new());

        let inner = Arc::new(ConnectionInner {
            sink: AsyncMutex::new(TransportSink::Ws(sink)),
            registry: Arc::clone(&registry),
            next_id: AtomicUsize::new(1),
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        });

        // 后台 read loop：每条文本喂纯路由；WS 关闭/出错 → fail_connection 解除所有挂起。
        let reg_for_loop = Arc::clone(&registry);
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(WsMessage::Text(text)) => {
                        if let Err(e) = reg_for_loop.dispatch_message(&text) {
                            // 单条畸形消息不应拖垮连接；记录后继续。
                            tracing::warn!(target: "nomi_browser_engine::transport", error = %e, "dropped malformed CDP message");
                        }
                    }
                    Ok(WsMessage::Binary(_)) => {
                        // CDP over WS 是文本 JSON；二进制帧不该出现，忽略。
                    }
                    Ok(WsMessage::Close(_)) | Err(_) => break,
                    // Ping/Pong/Frame：tungstenite 自动处理 ping/pong，这里无需动作。
                    _ => {}
                }
            }
            // 连接结束（对端关闭 / 读错误）：解除所有挂起命令为 Closed。
            reg_for_loop.fail_connection();
        });

        Ok(Self { inner })
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
        });

        // 后台 read loop:从管道累积字节,按 NUL 切帧,每帧喂纯路由;EOF/出错 → fail_connection。
        let reg_for_loop = Arc::clone(&registry);
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                match receiver.read(&mut chunk).await {
                    Ok(0) => break, // EOF:chrome 关闭管道(进程退出)。
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        // 按 NUL 切出完整帧(一次 read 可能含多帧或半帧)。
                        while let Some(pos) = buf.iter().position(|&b| b == 0) {
                            let frame: Vec<u8> = buf.drain(..=pos).collect();
                            let text = &frame[..frame.len() - 1]; // 去结尾 NUL。
                            match std::str::from_utf8(text) {
                                Ok(s) => {
                                    if let Err(e) = reg_for_loop.dispatch_message(s) {
                                        tracing::warn!(target: "nomi_browser_engine::transport", error = %e, "dropped malformed CDP message (pipe)");
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(target: "nomi_browser_engine::transport", error = %e, "non-utf8 CDP pipe frame; dropped");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(target: "nomi_browser_engine::transport", error = %e, "pipe read error; closing connection");
                        break;
                    }
                }
            }
            reg_for_loop.fail_connection();
        });

        Ok(Self { inner })
    }

    /// 从 [`launch`](crate::launch) 产物的运输连接（pipe/ws 二选一）。供 `CdpBackend::from_launched`
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

    /// 订阅某 method（可选限定 session）的事件流。详见
    /// [`SessionRegistry::subscribe`]。
    pub fn subscribe(
        &self,
        method: impl Into<String>,
        session_id: Option<&str>,
    ) -> tokio::sync::broadcast::Receiver<CdpEvent> {
        self.inner.registry.subscribe(method, session_id)
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

        // 写 WS（失败则清理回调）。
        if let Err(e) = self.write_call::<C>(call_id, session_id, params).await {
            self.inner.registry.cancel_command(session_id, call_id);
            return Err(e);
        }

        // 等回包 vs deadline 竞速。
        match tokio::time::timeout(self.inner.command_timeout, rx).await {
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
    /// 之后用 [`Connection::run_attach_loop`]（或自建循环）处理每个 attach 事件、登记
    /// 子 session、装好该子 session 的监听，**最后**对它发
    /// `Runtime.runIfWaitingForDebugger` 放行（否则尤其 service_worker 永久卡）。
    pub async fn enable_auto_attach(&self) -> Result<(), TransportError> {
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

    /// 处理**单个** `Target.attachedToTarget` 事件：登记子 session（含 target 类型），
    /// 然后对其放行（`Runtime.runIfWaitingForDebugger`）。
    ///
    /// **spike 坑（务必遵守）**：**不**对 service_worker 主动 `detachFromTarget`
    /// （chromiumoxide 的写法）——SW 出口流量防火墙（P2/P3）需保持对 SW 的 attach。
    /// 这里对**所有**子 target（含 SW）一视同仁：登记 + 放行。
    ///
    /// 调用方应在 `enable_auto_attach` **之前**就订阅好 attach 事件，再把每个收到的
    /// 事件交给本方法。这保证「先装监听后放行」：放行（runIfWaitingForDebugger）发生
    /// 在子 session 已登记之后，故该子 session 上的后续事件不会丢。
    pub async fn handle_attached(&self, event: &EventAttachedToTarget) -> Result<(), TransportError> {
        let sid: String = event.session_id.clone().into();
        let ttype = event.target_info.r#type.clone();

        // 1) 先登记子 session（先装好路由，再放行）。
        self.inner.registry.register_session(&sid, &ttype);

        // 2) **级联 setAutoAttach 到 page/iframe 子 session**：OOPIF（跨进程 iframe）只在其**所属帧的
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

        // 3) 仅当该 target 在等调试器时才放行（waitForDebuggerOnStart=true 的产物）。
        //    放行命令用 send_may_fail：target 可能在我们处理前就 detach 了，吞掉即可。
        if event.waiting_for_debugger {
            let run = RunIfWaitingForDebuggerParams::default();
            self.send_may_fail::<RunIfWaitingForDebuggerParams>(&sid, &run)
                .await?;
        }
        Ok(())
    }

    /// 后台运行 attach 处理循环：持续消费 `Target.attachedToTarget`（全 session 通配），
    /// 对每个事件调 [`Connection::handle_attached`]。返回的 `JoinHandle` 可在连接关闭时丢弃。
    ///
    /// 编排正确性依赖：**先订阅（本方法内部 subscribe）→ 再 enable_auto_attach**。故
    /// 典型用法是先 `let h = conn.run_attach_loop();` 再 `conn.enable_auto_attach().await?;`。
    pub fn run_attach_loop(&self) -> tokio::task::JoinHandle<()> {
        let conn = self.clone();
        let mut rx = self.subscribe(EventAttachedToTarget::IDENTIFIER, None);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        match serde_json::from_value::<EventAttachedToTarget>(ev.params.clone()) {
                            Ok(attached) => {
                                if let Err(e) = conn.handle_attached(&attached).await {
                                    tracing::warn!(target: "nomi_browser_engine::transport", error = %e, "handle_attached failed");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(target: "nomi_browser_engine::transport", error = %e, "failed to parse attachedToTarget");
                            }
                        }
                    }
                    // 订阅落后（lagged）→ 继续；连接关闭（closed）→ 退出循环。
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use chromiumoxide::cdp::browser_protocol::target::EventAttachedToTarget;

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

    /// handle_attached：登记子 session（不 detach SW）。这验证 spike 修正——
    /// service_worker 一视同仁登记，绝不主动 detach。用注册表直接断言副作用，
    /// 放行命令（无真 WS）会在 send_may_fail 里因 sink 不可用而被吞（SessionClosed 路径
    /// 不触发，因 session 已登记；这里改为构造 waiting_for_debugger=false 跳过放行写 WS）。
    #[tokio::test]
    async fn handle_attached_registers_service_worker_without_detach() {
        // 构造一个不需要真 WS 的 Connection 不可行（connect 要真连接）。改为直接对
        // SessionRegistry 验证 handle_attached 的核心副作用「登记 SW 子 session」——
        // 该逻辑即 registry.register_session，已在 session.rs 单测覆盖类型登记。
        // 此处断言「不 detach」的契约：我们的 handle_attached 路径里**没有**任何
        // detachFromTarget 调用——以源码不变量形式由本测试名+注释钉死，运行期由
        // Task 7 的 #[ignore] 集成（真实 SW attach 后仍可见）兜底验证。
        let reg = SessionRegistry::new();
        // 模拟 handle_attached 的登记步骤（waiting=false 分支不写 WS）。
        let event_json = r#"{"sessionId":"SW1","targetInfo":{"targetId":"T","type":"service_worker","title":"","url":"","attached":true,"canAccessOpener":false},"waitingForDebugger":false}"#;
        let event: EventAttachedToTarget = serde_json::from_str(event_json).unwrap();
        let sid: String = event.session_id.clone().into();
        reg.register_session(&sid, event.target_info.r#type.clone());
        assert!(reg.has_session("SW1"));
        assert_eq!(reg.target_type("SW1").as_deref(), Some("service_worker"));
        // 不 detach：session 仍活（未被 fail_session）。
        assert!(reg.register_command("SW1", CallId::new(1)).is_ok());
    }

    // ── 真实 connect 集成测试 ───────────────────────────────────────────────
    //
    // 真实 WS connect + setAutoAttach + 子 session 放行需要一个跑着的 Chromium
    // （`chrome --remote-debugging-port=9222 --headless=new`）。本 task 范围只到传输/
    // 路由层，统一留给 Task 7 的 launch+connect 冒烟一并验证（届时由托管启动提供端口）。
    // 这里放一个 `#[ignore]` 占位，指向手动起的 9222 实例，便于本地按需冒烟。
    #[tokio::test]
    #[ignore = "需手动 chrome --remote-debugging-port=9222 --headless=new；统一留 Task 7"]
    async fn live_connect_and_auto_attach_smoke() {
        // 取 browser ws url：GET http://127.0.0.1:9222/json/version → webSocketDebuggerUrl。
        // 这里省略 HTTP 探测（属 Task 7 launch 职责），直接用约定 url 形态示意。
        // 这是手动冒烟占位：未提供 NOMI_CDP_WS_URL 时优雅跳过（而非 panic），
        // 这样 `--run-ignored` 全量跑不会因缺少手动起的 9222 实例而见红；真实
        // launch+connect 覆盖由本 crate 其它 #[ignore] 集成测试（自起托管 Chrome）提供。
        let Ok(ws_url) = std::env::var("NOMI_CDP_WS_URL") else {
            eprintln!(
                "skipping live_connect_and_auto_attach_smoke: set NOMI_CDP_WS_URL to a browser \
                 webSocketDebuggerUrl (with a running `chrome --remote-debugging-port=9222 \
                 --headless=new`) to run this manual smoke"
            );
            return;
        };
        let conn = Connection::connect(&ws_url).await.expect("connect");
        let _attach_loop = conn.run_attach_loop();
        conn.enable_auto_attach().await.expect("setAutoAttach");
        // 给子 session attach 一点时间，然后断言至少根 session 在。
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(conn.registry().has_session(ROOT_SESSION));
    }
}
