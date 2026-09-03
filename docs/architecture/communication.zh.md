# 通信

NomiFun 的各个进程 —— SPA、嵌入式后端、终端子进程与 MCP 服务器 —— 通过五条彼此独立的通道相互对话。它们的职责互不重叠，挑选合适通道的规则在客户端的适配层（`ui/src/common/adapter/`）以及服务端的路由与服务 crate 中得到了固化。

## 五条通道

| 通道 | 方向 | 承载 | 位置 |
| --- | --- | --- | --- |
| HTTP REST | UI ↔ 后端 | 所有请求/响应操作：CRUD、命令调用、文件操作 | `http://127.0.0.1:<port>/api/*` |
| WebSocket | 后端 → UI（终端输入 / 心跳时反向） | 流式 agent token、终端输出、广播事件、会话产物 | `/ws` |
| Tauri IPC | 仅 UI → 桌面外壳 | 浏览器没有等价物的操作系统外壳特性 | `@tauri-apps/api` 与 Tauri 插件 |
| PTY（stdio） | 后端 ↔ 终端子进程 | 终端 session 的字节流，包含第三方 agent CLI | 伪终端 |
| MCP（stdio 或 HTTP） | 后端 ↔ MCP 服务器，agent CLI ↔ MCP 服务器 | 工具调用、资源读取、提示词 | 派生进程或本地 HTTP |

## HTTP REST

SPA 的适配层（[`httpBridge.ts`](../../ui/src/common/adapter/httpBridge.ts)）把每个操作包装成一个有类型的调用：

```ts
// Approximate shape — see httpBridge.ts for the real definitions.
const conversation = httpGet<Conversation, { id: string }>(p => `/api/conversations/${p.id}`);
const sendMessage  = httpPost<SendMessageResponse, SendMessageRequest>(p => `/api/conversations/${p.id}/messages`);
```

线上格式依赖的若干常量：

- 请求体上限 —— `nomifun_common::constants::BODY_LIMIT`。
- CSRF cookie 名 —— `nomifun-csrf-token`。
- CSRF header 名 —— `x-csrf-token`。
- 默认端口（`nomifun-web`） —— `8787`。
- 默认 host —— `127.0.0.1`。

### CSRF 双提交

Web 宿主默认以认证模式运行后端。两个 cookie 在认证中扮演角色：

| Cookie | 由谁设置 | 由谁读取 | HttpOnly |
| --- | --- | --- | --- |
| 会话 JWT | 登录时由 `nomifun-auth` 设置 | 每次认证请求由 `auth_middleware` 读取 | 是 |
| CSRF token（`nomifun-csrf-token`） | 由 `csrf_middleware` 设置（首次缺失时签发） | 浏览器的 `document.cookie`，再由 SPA 回显到 `x-csrf-token` | 否 —— SPA 必须能读到 |

CSRF 中间件（[`crates/backend/nomifun-auth/src/csrf.rs`](../../crates/backend/nomifun-auth/src/csrf.rs)）守护 POST / PUT / PATCH / DELETE 请求；安全方法绕过校验。三个豁免路径 —— `/login`、`/api/auth/qr-login`、`/api/auth/setup` —— 会跳过检查，因为它们正用于引导会话本身。桌面外壳使用 `TrustLocalToken`：WebView 呈递本地信任 secret，远程/其他本机客户端仍需正常认证或走 WebUI 登录。`--local` 仅是独立 `nomicore`/开发 Web host 的无鉴权模式。

### 响应包装

成功的 JSON 响应包装为 `{ success: true, data: ... }`；错误为 `{ success: false, error: <message>, code: <machine code>, details: ... }`。SPA 的 `httpRequest` 自动解包 `data` 字段，并对非 2xx 响应抛出携带 `status` / `code` / `backendMessage` / `details` 的 `BackendHttpError`，使调用方无需解析消息文本即可在 `code` 上分支。

## WebSocket —— `/ws`

一条 WebSocket 承载后端与 SPA 之间的所有流式负载：

| 事件类别 | 何时发送 | 来源 crate |
| --- | --- | --- |
| `message.stream` | 模型按块发出 token 时 | `nomifun-conversation::stream_relay` |
| `conversation.artifact` | 工具产生了产物（文件 / 图像 / 预览） | `nomifun-conversation::routes_aux` |
| `terminal.output` | PTY 产生输出 | `nomifun-terminal` |
| `ssh.status` | 某会话的 SSH 链路状态发生真实变化 | `nomifun-ssh::events` |
| 审批请求 / 响应 | 工具调用需要用户批准 | `nomifun-conversation`（经接缝） |
| `agentExecution.changed` / `agentExecution.leadThinking` | 持久化 Agent 协作状态与发起 Agent 的瞬时思考流 | `nomifun-agent-execution` |
| `auth-expired` / 关闭 1008 | 会话 JWT 中途失效 | `nomifun-realtime` |
| 心跳（`ping` / `pong`） | 连接保活 | `nomifun-realtime` |

`ssh.status` 是某个会话 SSH 链路的 owner-scoped 投影:`{ sshHostId, conversationId, state, attempt, nextRetryInMs, hostFingerprint, detail, retryable, reaped, changedAt }`,`state` 取 `idle` / `connecting` / `connected` / `degraded` / `reconnecting` / `dropped` / `closed` 之一。它走用户总线而非 per-turn 的 agent 流,因为链路恰恰会在会话空闲、没有任何 turn 可承载消息时掉线与重连;`SshConnectionPool` 只在状态**真实变化**时发布,所以一条健康的空闲链路不占用总线。`GET /api/ssh-hosts/statuses` 从同一个 watch 值提供同样的载荷,供重连后补齐快照。

升级由 `nomifun_realtime::ws_upgrade_handler`（[`crates/backend/nomifun-realtime/src/handler.rs`](../../crates/backend/nomifun-realtime/src/handler.rs)）处理，它校验通过 cookie 或 `Sec-WebSocket-Protocol` header 携带的 JWT（header 的取值会被原样回显以使握手正确完成）。认证失败时它会发送 `auth-expired` 消息并以 1008 关闭；SPA 同时监听这两个信号（参见 [`browser.ts`](../../ui/src/common/adapter/browser.ts)），并在任一路径上重定向到 `/login`。

`httpBridge.ts` 中的 SPA WebSocket 逻辑是单例的：每个页面生命周期一个连接、指数退避重连（封顶 30s）、按 JSON 形状（`{ name, data }`）解复用并把事件分发到通过 `wsEmitter(name)` 注册的监听器。两个事件名与 HTTP 路径列表被显式维护，用以**抑制 agent 流式或 PTY 活跃时的嘈杂控制台日志**：

```ts
const NOISY_WS_EVENTS    = new Set(['terminal.output', 'message.stream', 'conversation.artifact']);
const NOISY_HTTP_FRAGMENTS = ['/input', '/resize'];
```

心跳常量定义在 `nomifun_realtime::types::{HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, PER_CONNECTION_BUFFER}`。

## Tauri IPC —— 仅操作系统外壳

Tauri 外壳采用**反向 IPC**：是 SPA 调用操作系统外壳，绝不反过来。[`apps/desktop/src/main.rs`](../../apps/desktop/src/main.rs) 中注册的 Tauri 命令包括：

```rust
.invoke_handler(tauri::generate_handler![
    install_update,
    sync_companion_windows,
    webui_get_status,
    webui_start,
    webui_stop,
    set_keep_awake,
    set_tray_labels
])
```

其余一切都通过 Tauri 已发布的 JS API —— `@tauri-apps/api` 与 `tauri-plugin-*` crate。SPA 的 `tauriShell.ts` 用 `isTauri()` 守护每个操作，使同一份代码路径在浏览器中变为空操作：

| 操作 | 插件 |
| --- | --- |
| 窗口最小化 / 最大化 / 关闭、isMaximized 监听 | `@tauri-apps/api/window` |
| 打开原生对话框 | `tauri-plugin-dialog` |
| 发送通知 | `tauri-plugin-notification` |
| 进程重启 | `tauri-plugin-process` |
| 开机自启 | `tauri-plugin-autostart` |
| 深链接 `open-url` 事件 | `tauri-plugin-deep-link` |
| 单实例锁 | `tauri-plugin-single-instance` |
| 自更新检查（唯一的 Rust 命令） | `tauri-plugin-updater` |
| OS 路径查询（`home`、`downloads`、`desktop`） | `@tauri-apps/api/path` |

少数操作没有 Tauri 等价物，已在浏览器中被有意**桩化**（Chrome DevTools Protocol、GPU 恢复、渲染进程日志通道、关闭至托盘）。这些操作在 `tauriShell.ts` 中标记为 `DEGRADE_STUB`，留给未来的 Tauri 移植。

## 会话回合 —— 进程内的 nomi 引擎

会话只有一个引擎：内置的 `nomi`。接缝 crate `nomifun-ai-agent` 持有 Agent 工厂和 `AgentRuntimeRegistry`；后者按 Conversation 缓存唯一的进程内 runtime handle。一个回合完整地在后端进程内执行，没有需要握手、也没有需要移交会话的子 agent CLI。

进程内的流量如下：

```
SPA ──HTTP/WS──▶ nomifun-conversation ──▶ nomifun-ai-agent::AgentService
                                                      │
                                                      ▼
                                          nomi-agent engine turn
                                            （providers / tools / MCP）
                                                      │
                                                      ▼
                                          stream tokens / tool calls
                                                      │
                              broadcast through nomifun-realtime to /ws
```

`nomi-protocol` crate 定义了宿主/agent 的命令、事件与工具审批状态机；`nomifun-ai-agent::protocol::events::AgentStreamEvent` 把这些事件翻译成 SPA 能理解的 `WebSocketMessage`。

第三方 agent CLI（Claude Code、Codex、Gemini CLI）不走这条通道：它们作为普通子进程运行在 `nomifun-terminal` 的 PTY session 里，后端持有伪终端而不解析它们的协议。见 [`../guides/terminal.zh.md`](../guides/terminal.zh.md)。

## MCP —— Model Context Protocol

MCP 服务器对外暴露引擎可调用的工具与资源。当前 `nomifun-app`
二进制提供多个 stdio 桥子命令，而不是旧的单一 `mcp-bridge`：

- `mcp-requirement-stdio`
- `mcp-knowledge-stdio`
- `mcp-gateway-stdio`
- `mcp-open-stdio`

同一二进制还提供 `terminal-hook`、`doctor`，以及
`remote open/turn/observe/cancel` 等运维 / 调用子命令。

不同运行时的 MCP 注入来源不同：

- 用户配置的 MCP 行与 OAuth HTTP MCP 服务器由 `nomifun-mcp` 管理；
- Requirement 与 Knowledge 服务器是有作用域的内部 MCP 服务器；
- 平台 Gateway 工具通过 `nomifun-gateway` 传输；
- Browser / Computer 桥按 feature gate 启用；
- 公开 `/mcp` 由 `nomifun-public` 提供，是安装令牌认证的 canonical Remote
  AgentSession 入口。

内部 stdio bridge 不信任调用方提交的 user id 或 Conversation 持久标记。宿主在
服务端派生精确作用域，只向子进程签发带作用域、有效期和签名的能力声明。签名根
始终留在父进程内，不序列化进运行时 DTO、数据库行或子进程配置。公开能力入口使用
独立的安装令牌边界，不继承内部宿主能力声明。

针对 HTTP MCP 服务器的 OAuth 流程由 `nomifun-mcp::oauth_service` 处理（PKCE、回调 URI、token 存储）。加密后的 token 通过 AES-GCM 落入 SQLite 的 `oauth_tokens` 仓库（参见 `nomifun-common::crypto::{encrypt_string, decrypt_string}`）。

## Canonical Remote 入口

Fresh-v4 host 对同一个安装令牌认证的 Remote contract 提供两种投影：

- `/mcp`：Streamable-HTTP MCP，精确包含 `open`、`turn`、`observe`、`cancel`；
- `/api/remote/*`：同四操作的 REST 形式。

每个安装只有一枚有效 Token。调用方以安装 owner 身份行动，不隐式绑定伙伴，
也不继承伙伴 profile、模型 / 人格、知识绑定或活动会话。`open` 使用本地 owner
创建的 `RemoteBinding`，返回显式 UUIDv7 `agent_session_id`；后续操作始终携带
该 ID。MCP transport session 只负责连接生命周期。

## 快速查表：事件 / 传输

| 事件或操作 | 传输 |
| --- | --- |
| 登录 / 设置 | HTTP `/api/auth/*` |
| 发送会话消息 | HTTP `/api/conversations/*`，流式事件走 `/ws` |
| 持久化 Agent 协作 | HTTP `/api/agent-executions/*`，失效通知与瞬时思考流走 `/ws` |
| 终端输入 / 输出 | 输入走 HTTP 终端路由，输出走 `/ws` |
| 桌面 keep-awake | Tauri command |
| Remote AgentSession 操作 | MCP `/mcp` 或 REST `/api/remote/*` |
| 会话回合 | 进程内 `nomi` 引擎，token 流走 `/ws` |
| 终端（含第三方 agent CLI） | `nomifun-terminal` 管理的 PTY 子进程 stdio |
| session 内部知识搜索 | `mcp-knowledge-stdio` bridge |

交叉参考：统一协作聚合与状态机见 [`agent-execution.zh.md`](agent-execution.zh.md)；数据与持久化层见 [`data-and-storage.md`](data-and-storage.zh.md)；引擎内部细节见 [`agent-engine.md`](agent-engine.zh.md)；SPA 适配层见 [`frontend.md`](frontend.zh.md)。
