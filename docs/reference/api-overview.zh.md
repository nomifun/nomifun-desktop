# API 概览

NomiFun 的后端（`nomifun-app`，二进制 `nomicore`）对外暴露的是单一的
axum HTTP 服务。SPA、桌面外壳，以及任何外部集成，与它沟通的方式都一样：
HTTP 上的 JSON 用于命令/查询，WebSocket 用于流式事件。

本页是一份**导览**，不是穷尽式的端点参考。完整的接口面位于
`crates/backend/` 下的各路由模块；源码即权威参考。下方列出了各分组的
基础路径与对应的路由 owner——请从那里开始查阅。

> **Agent API 当前合同：**用户通过 `/agent` 工作台完成 Agent 设计和试用；机器资源
> 使用 `/api/agent-presets/*`、`/api/agent-preset-templates/*`、
> `/api/agent-sessions/*`、`/api/agent-bindings/*` 和 `/api/capabilities`。
> 旧 `/api/presets` 不再是 canonical API；当前重构工作树仍有 residual 时，以
> `GLOBAL-CLOSURE-TODO.zh.md` 的 AP-6/AP-7 状态为准，不把历史兼容路径当作可用合同。

## Base URL

| 宿主 | 默认 base URL | 备注 |
|---|---|---|
| `nomifun-desktop` | `http://127.0.0.1:<picked-port>` | 启动时挑选一个空闲的 localhost 端口。渲染端通过 IPC 获知端口号，并以此向 `/api` 与 `/ws` 发起调用。 |
| `nomifun-web` | `http://<host>:<port>`（默认 `http://127.0.0.1:8787`） | 同一个后端，与 SPA 一并在同一个端口上提供。 |
| `nomicore` 独立运行 | `http://127.0.0.1:25808` | 单独运行后端——便于调试。 |

SPA 使用**相对路径**（`/api/...`、`/ws`）。客户端不需要指向另一台 API
服务——SPA 与 API 同址。

## 鉴权模型

NomiFun 启动时进入三种鉴权策略之一：

### 已鉴权模式（`nomifun-web` 默认）

- 通过 `POST /login` 登录，返回一个会话 JWT，同时写入 cookie
  （`nomifun-session`，`HttpOnly`）与 JSON body。后续请求依靠该 cookie
  或 `Authorization: Bearer …` 请求头进行鉴权。
- 状态变更类请求还必须附带 CSRF 请求头 `x-csrf-token`，其值需与
  `nomifun-csrf-token` cookie 匹配（Double Submit Cookie 模式）。安全
  方法（`GET`、`HEAD`、`OPTIONS`）跳过 CSRF；登录/设置/二维码登录端点
  因尚无会话被豁免。
- WebSocket 升级携带同一份 JWT——通常通过 `Sec-WebSocket-Protocol`
  传输，可由 `GET /api/ws-token` 获取。`/ws` 路由对 CSRF 豁免（在
  WebSocket 升级中无法做基于 cookie 的双提交），但仍需鉴权。
- 限流器分别按客户端作用于登录尝试、一般 API 流量与已鉴权的状态变更
  动作。

### 桌面本地信任模式（`nomifun-desktop`）

- 嵌入式后端使用 `AuthPolicy::TrustLocalToken`。
- 桌面 WebView 会得到每次启动生成的 secret（`window.__nomiLocalTrust`），并在 HTTP/WebSocket 请求中呈递它。
- 其他客户端即使在同一台机器上，也不会因为来自 loopback 自动受信任；除非它拥有正常登录会话。这也是 WebUI 远程访问可以放在登录后的原因。

### 无鉴权本地模式（`nomicore --local`，或 Web 宿主 `--insecure-no-auth`）

- 鉴权与 CSRF 完全关闭。每个请求都以数据库中记录的安装所有者身份执行。
- 加入一层宽松的 CORS，使桌面 WebView（以及工具）可以自由调用 API。
- 仅本地可达的路由（如 `/api/webui/*`）变为
  可达。

本地模式下的信任边界是网络——只能将其暴露在 loopback 或完全受信任的
私有网络上。Web 宿主在 `--insecure-no-auth` 与非 loopback 绑定同时使用
时会大声地打印警告日志。

## 请求体大小与上限

- 请求体的默认大小上限是 **10 MiB**（`nomifun-common` 中的
  `BODY_LIMIT`）。确实需要更大的路由（文件上传、ZIP 创建等）会安装自己
  的更大限制——`/api/fs/upload` 接受最大 30 MiB。
- 代用户下载的远程图片上限为 5 MiB，最多跟随 5 次重定向。

## 路由分组

每个分组归属一个特定的 crate。下表中的基础路径就是挂载到 app router 中
的实际 URL 前缀；鉴权在已鉴权模式和桌面本地信任模式下生效。

| 分组 | 基础路径 | 鉴权 | 归属 crate / 文件 |
|---|---|---|---|
| 健康检查 | `/health` | 公共 | [`router/health.rs`](../../crates/backend/nomifun-app/src/router/health.rs) |
| 鉴权 —— 登录 / 设置 / 状态 / 刷新 | `/login`、`/logout`、`/api/auth/*`、`/api/ws-token`、`/qr-login` | 混合（登录/设置/qr-login：公共；其余：已鉴权） | [`nomifun-auth/src/routes.rs`](../../crates/backend/nomifun-auth/src/routes.rs) |
| 鉴权 —— 仅本地 admin/internal | `/api/webui/*` | 仅本地模式 | 同上 |
| 会话 | `/api/conversations/*`、`/api/messages/search` | 已鉴权 | [`nomifun-conversation/src/routes.rs`](../../crates/backend/nomifun-conversation/src/routes.rs)、[`routes_aux.rs`](../../crates/backend/nomifun-conversation/src/routes_aux.rs) |
| Agent 工作台控制平面 | `/api/agent-presets/*`、`/api/agent-preset-templates/*`、`/api/capabilities`、`/api/mcp-tool-mappings`、`/api/agent-bindings/*` | 已鉴权 / owner-scoped | [`nomifun-agent-control-plane/src/routes.rs`](../../crates/backend/nomifun-agent-control-plane/src/routes.rs)、[`nomifun-app/src/router/agent_platform.rs`](../../crates/backend/nomifun-app/src/router/agent_platform.rs) |
| Agent Session | `/api/agent-sessions/*` | 已鉴权 / owner-scoped | [`nomifun-agent-platform/src/platform.rs`](../../crates/backend/nomifun-agent-platform/src/platform.rs)、[`nomifun-app/src/router/agent_platform.rs`](../../crates/backend/nomifun-app/src/router/agent_platform.rs) |
| 旧 Agent/模型信息查询 | `/api/agents/*` | 已鉴权 | [`nomifun-ai-agent/src/routes/agent.rs`](../../crates/backend/nomifun-ai-agent/src/routes/agent.rs)；不承担 AgentPreset authoring |
| SSH 主机 | `/api/ssh-hosts/*` | 仅实例主人 | [`nomifun-ssh/src/routes.rs`](../../crates/backend/nomifun-ssh/src/routes.rs) |
| MCP 服务 | `/api/mcp/*` | 已鉴权 | [`nomifun-mcp/src/routes.rs`](../../crates/backend/nomifun-mcp/src/routes.rs) |
| 技能 | `/api/skills/*` | 已鉴权 | [`nomifun-extension/src/skill_routes.rs`](../../crates/backend/nomifun-extension/src/skill_routes.rs) |
| 扩展 | `/api/extensions/*` | 已鉴权 | [`nomifun-extension/src/routes.rs`](../../crates/backend/nomifun-extension/src/routes.rs) |
| Hub（扩展市场） | `/api/hub/*` | 已鉴权 | [`nomifun-extension/src/hub_routes.rs`](../../crates/backend/nomifun-extension/src/hub_routes.rs) |
| 计划任务 | `/api/cron/*` | 已鉴权 | [`nomifun-cron/src/routes.rs`](../../crates/backend/nomifun-cron/src/routes.rs) |
| 频道（IM 桥） | `/api/channel/*` | 已鉴权 | [`nomifun-channel/src/routes.rs`](../../crates/backend/nomifun-channel/src/routes.rs) |
| Webhook + 标签设置 | `/api/webhooks/*`、`/api/tags/{tag}/settings` | 已鉴权 | [`nomifun-webhook/src/routes.rs`](../../crates/backend/nomifun-webhook/src/routes.rs) |
| 需求（项目看板） | `/api/requirements/*` | 已鉴权 | [`nomifun-requirement/src/routes.rs`](../../crates/backend/nomifun-requirement/src/routes.rs) |
| AutoWork / IDMM | `/api/idmm/*`、`/api/requirements/autowork*` | 已鉴权 | [`nomifun-idmm/src/routes.rs`](../../crates/backend/nomifun-idmm/src/routes.rs) |
| Agent Execution | `/api/agent-executions/*` | 已鉴权 | [`nomifun-agent-execution/src/routes.rs`](../../crates/backend/nomifun-agent-execution/src/routes.rs) |
| 终端 | `/api/terminals/*` | 已鉴权 | [`nomifun-terminal/src/routes.rs`](../../crates/backend/nomifun-terminal/src/routes.rs) |
| 知识库 | `/api/knowledge/*` | 已鉴权 | [`nomifun-knowledge/src/routes.rs`](../../crates/backend/nomifun-knowledge/src/routes.rs) |
| 创意工坊管理与生成 | `/api/creative-studio/*` 管理分组：项目、素材、提示词、模板/运行/草稿、任务、Agent session 与集合 | 仅实例 owner | [`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs)、[`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs)、[`nomifun-conversation/src/routes.rs`](../../crates/backend/nomifun-conversation/src/routes.rs) |
| 创意工坊媒体交付 | `GET /api/creative-studio/files/{asset_id}` | 公开的只读 capability URL；不提供列表或写操作 | [`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs) |
| Plugin 平台与 Product 管理 | `/api/plugins/*`：workspace、draft、authoring、project、mount、operation 和 installation-scoped Plugin 状态 | 仅实例 owner；写操作还要求本地产品信任 | [`router/plugin_product`](../../crates/backend/nomifun-app/src/router/plugin_product/) |
| Plugin Product 运行域 | `/api/plugins/runtimes/*`：project、Workshop、源码、构建/测试/发布/回滚、启停、发布模式、Service 生命周期、分享/备份/导入、回收站/恢复/永久删除和 Surface 开关 | 仅实例 owner；写操作还要求本地产品信任 | [`router/plugin_runtime.rs`](../../crates/backend/nomifun-app/src/router/plugin_runtime.rs) |
| Plugin Surface 资源与 bridge | 由 Surface open 返回、受 Capability 约束的资源和 bridge 路由；调用方不得自行拼接公开资源 URL | 持有有效且作用域匹配的 Surface capability | 同上 |
| 伙伴 | `/api/companion/*` | 已鉴权 | [`nomifun-companion/src/routes.rs`](../../crates/backend/nomifun-companion/src/routes.rs) |
| NomiFun Desktop 访问令牌 | `/api/webui/access-token` | 本地信任 / 安装 owner 流 | [`router/instance_token_routes.rs`](../../crates/backend/nomifun-app/src/router/instance_token_routes.rs) |
| 会话 Browser Workspace | `/api/conversations/{conversation_id}/browser*` | 安装 owner + 本地产品信任；校验 conversation 所有权，并拒绝委派执行 step | [`router/browser_workspace.rs`](../../crates/backend/nomifun-app/src/router/browser_workspace.rs) |
| 系统浏览器连接 | `/api/conversations/{conversation_id}/system-browser*` | 安装 owner + 本地产品信任；校验 conversation 所有权；当前仅支持 Windows | [`router/system_browser.rs`](../../crates/backend/nomifun-app/src/router/system_browser.rs) |
| 文件系统 | `/api/fs/*` | 已鉴权 | [`nomifun-file/src/routes.rs`](../../crates/backend/nomifun-file/src/routes.rs) |
| Office 预览 | `/api/word-preview/*`、`/api/excel-preview/*`、`/api/ppt-preview/*`、`/api/preview-history/*` | 已鉴权 | [`nomifun-office/src/routes.rs`](../../crates/backend/nomifun-office/src/routes.rs) |
| Office iframe 代理 | `/api/ppt-proxy/*`、`/api/office-watch-proxy/*` | 公共（提供 iframe 内容；不鉴权） | 同上 |
| 设置 + 提供商 + 系统信息 | `/api/settings`、`/api/providers/*`、`/api/system/*` | 已鉴权 | [`nomifun-system/src/routes.rs`](../../crates/backend/nomifun-system/src/routes.rs) |
| 宿主系统权限 | `GET /api/system/permissions` 读取麦克风、辅助功能与屏幕录制实时状态；`POST /api/system/permissions/{request,open-settings}` 请求授权或打开对应设置 | 读取仅限安装 owner；会弹出本机 UI 的写操作还要求本地产品信任 | [`router/computer_permissions.rs`](../../crates/backend/nomifun-app/src/router/computer_permissions.rs) |
| 全局模型故障转移队列 | `/api/agent/model-failover` | 已鉴权 | [`router/model_failover.rs`](../../crates/backend/nomifun-app/src/router/model_failover.rs) |
| 连接探测（Bedrock 等） | `/api/bedrock/test-connection` | 已鉴权 | [`nomifun-system/src/bedrock_probe/routes.rs`](../../crates/backend/nomifun-system/src/bedrock_probe/routes.rs) |
| Shell 辅助 + STT | `/api/shell/*`、`/api/stt` | 已鉴权 | [`nomifun-shell/src/routes.rs`](../../crates/backend/nomifun-shell/src/routes.rs) |
| 公共资源（logo） | `/api/assets/logos/*` | 公共 | [`nomifun-assets/src/routes.rs`](../../crates/backend/nomifun-assets/src/routes.rs) |
| Canonical Remote MCP front door | `/mcp` | 安装令牌 | [`nomifun-public/src/canonical.rs`](../../crates/backend/nomifun-public/src/canonical.rs) |
| Canonical Remote REST API | `/api/remote/open`、`/api/remote/turn`、`/api/remote/observe`、`/api/remote/cancel` | 安装令牌 | [`nomifun-app/src/router/remote_rest.rs`](../../crates/backend/nomifun-app/src/router/remote_rest.rs) |
| 实时 WebSocket | `/ws` | 已鉴权（token 通过 `Sec-WebSocket-Protocol` 或查询串传递） | [`nomifun-realtime/src/handler.rs`](../../crates/backend/nomifun-realtime/src/handler.rs) |

如需各路由具体支持的方法，请阅读对应的 `routes.rs` 文件——每个 router
都在源文件内联声明自身的路由。

### Agent 工作台控制平面

Agent 工作台的请求顺序由服务端控制，而不是由客户端拼接 Snapshot：

```text
官方 seed / 用户 Draft
  → /api/agent-presets/... Preview
  → canonical Compiler
  → immutable Revision + ContributionLock[]
  → ResolvedSnapshot
  → /api/agent-sessions
```

客户端不得提交 Snapshot digest、Mount ID、内部 Revision ID、完整 Binding 或裸
canonical JSON 来驱动执行。`agent_snapshot` 是 Conversation、Cron、Agent
Execution participant 和 Template participant 的统一持久化执行投影名称；061/062
migration 的当前状态和尚未清零的旧引用见 AP ledger。

### 选取的鉴权端点

下面这些是客户端最常直接交互的鉴权端点：

| 方法 + 路径 | 用途 |
|---|---|
| `POST /login` | 用户名 + 密码登录。返回 `{success, user, token}` 并设置会话 cookie。CSRF 豁免。带限流。 |
| `POST /api/auth/setup` | 全新安装上的一次性首位管理员创建。原子操作；并发调用通过条件 UPDATE 竞争，只有一个会赢（其余得到 `409 Conflict`）。CSRF 豁免。 |
| `POST /logout` | 将当前 token 加入黑名单；清除会话 cookie。 |
| `GET  /api/auth/status` | 公共——返回 `{needs_setup, user_count, is_authenticated}`。可作为 liveness/health 探针。 |
| `GET  /api/auth/user` | 返回当前 `{id, username}`。 |
| `POST /api/auth/change-password` | 修改当前用户密码并轮换 JWT 密钥（使其他会话全部失效）。 |
| `POST /api/auth/refresh` | 刷新仍然有效但接近过期的 token。 |
| `GET  /api/ws-token` | 返回用于 WebSocket 升级的 token。 |
| `POST /api/auth/qr-login` | 消费一次性的二维码登录 token（由 WebUI 远程访问流程下发）。 |
| `GET  /qr-login` | 静态 HTML 页面，用于完成来自手机扫码的二维码登录跳转。 |

### 浏览器平台端点

交互式 Browser 是由单个 conversation 持有的原生 Surface。HTTP API 服务于
会话 UI；Agent 的观察和输入则通过当前 turn 冻结的 `Browser` Tool binding，
而不是 HTTP 管理 API。两条路径操作的是同一组标签页和同一个 Profile。Agent
run 活跃时，用户命令 fail closed；本轮结束后，用户可以直接操作同一页面。
系统没有用户控制权转交状态。

| 方法 + 路径 | 用途 |
|---|---|
| `GET /api/conversations/{conversation_id}/browser` | 读取 Browser Workspace snapshot，不创建 runtime。 |
| `POST /api/conversations/{conversation_id}/browser` | 确保该会话的原生 Browser Workspace 存在，并返回 snapshot。 |
| `DELETE /api/conversations/{conversation_id}/browser` | 使用精确 `runtime_generation` 关闭 idle Browser Workspace；陈旧请求或活跃 run 会被拒绝。 |
| `POST /api/conversations/{conversation_id}/browser/commands` | 在输入权属于用户时，对原生标签页执行一个类型化命令：创建/激活/关闭/导航/历史/刷新、关闭全部网页、打开下载目录、在系统浏览器打开当前 URL、取消受管下载、处理网站权限/对话框，或清除此会话的站点数据。 |
| `GET /api/conversations/{conversation_id}/system-browser` | 读取独立的系统浏览器连接与已授权标签页状态。 |
| `POST /api/conversations/{conversation_id}/system-browser` | 在用户已经开启 Chrome 远程调试后，连接正在运行的 Chrome；NomiFun 不启动 Chrome，也不导入其 Profile。 |
| `DELETE /api/conversations/{conversation_id}/system-browser` | 断开精确连接，不关闭 Chrome 或其中的标签页。 |
| `POST /api/conversations/{conversation_id}/system-browser/choices` | 列出当前连接中可供用户明确授权的标签页候选项。 |
| `POST /api/conversations/{conversation_id}/system-browser/tabs` | 为当前 conversation 授权一个精确标签页候选项。 |

这些路由只挂载在本地受信任的桌面产品中，并同时要求安装 owner 鉴权。响应不会
包含原始协议 endpoint、调试端口、Profile 路径、Cookie 或凭据。Agent 持有
conversation run 时，系统浏览器的可变操作不可用。

`nomi_local_websearch` 没有 Browser 管理端点。它是独立可选的 Agent Tool，由
隔离后台浏览器实现，不使用 conversation Profile。`nomi_system_browser` 同样是
独立 Agent 能力；启用它不会顺带启用内嵌 Browser 或本地搜索。

## WebSocket 事件模型

`/ws` 是应用更新共用的 JSON 实时通道：智能体 token 流、终端输出、
Browser inventory/生命周期事件，以及需求、计划任务和协作任务的状态
变化。它仍是通用应用事件通道，不是 Browser 页面 Viewer 或输入通道。

- 鉴权：通过 `GET /api/ws-token` 获得的 JWT，放在 WebSocket 的
  `Sec-WebSocket-Protocol` 请求头中（或 `Authorization`）。token 无效或
  过期 → 服务端发出 `auth-expired` 事件并以 `1008` 关闭。完全没有
  token → 以 `1008` 关闭，原因为 `"no token provided"`。
- 升级成功后，每条消息都是带 `type` 与 `payload` 的 JSON 对象。当域内
  事件发生时（新的智能体 token、一个终端字节、需求状态切换），由
  服务端推送；客户端通常无需回送任何内容。服务端把单一的
  `BroadcastEventBus` 多路复用给所有已连接客户端。
- 心跳：每 30 秒 ping 一次，60 秒超时（`HEARTBEAT_INTERVAL` /
  `HEARTBEAT_TIMEOUT`）。
- 关闭码：`1000` 表示正常关闭；`1008` 表示策略违规（鉴权失败、token
  无效）。

`type` 取值集合是开放的——扩展与功能模块会发出各自的类型。请把未知
类型当作向前兼容的：忽略它们即可。

## 响应包络

绝大多数 JSON 响应使用同一种形状（来自 `nomifun-api-types` 的
`ApiResponse<T>`）：

```json
{ "success": true, "data": { ... } }
```

错误使用恰当的 HTTP 状态码返回，body 形如：

```json
{ "success": false, "error": "Invalid username or password" }
```

登录/设置/刷新这几个 handler 会返回略微富化的包络
（`LoginResponse`、`RefreshResponse`）——它们会把 token 或 user 对象
内联在响应中。

## 真值来源指引

上面的列表只是为了把你引导到对的模块。到达后请阅读源码——每个 router
在一处声明全部路由，每个 handler 都在同一个文件或紧挨着的下一个文件
里。Router 装配本身位于
[`crates/backend/nomifun-app/src/router/routes.rs`](../../crates/backend/nomifun-app/src/router/routes.rs)；
中间件栈（CSRF、安全响应头、请求体上限、可选的 CORS）也在那里。

## 另见

- [配置参考](./configuration.zh.md) —— 参数、环境变量、鉴权密钥解析顺序。
- [浏览器平台架构](../architecture/browser-platform.zh.md) —— 会话内原生 Browser Workspace、独立系统浏览器连接、隔离后台 runtime 与生命周期保证。
- [疑难排查](./troubleshooting.zh.md) —— 常见的 API 与 WebSocket 故障
  形态。
- [Web 服务部署](../guides/web-server-deployment.md) —— 在 TLS 之后把
  API 暴露到网络上。
