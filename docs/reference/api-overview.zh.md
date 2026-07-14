# API 概览

NomiFun 的后端（`nomifun-app`，二进制 `nomicore`）对外暴露的是单一的
axum HTTP 服务。SPA、桌面外壳，以及任何外部集成，与它沟通的方式都一样：
HTTP 上的 JSON 用于命令/查询，WebSocket 用于流式事件。

本页是一份**导览**，不是穷尽式的端点参考。完整的接口面位于
`crates/backend/` 下的各路由模块；源码即权威参考。下方列出了各分组的
基础路径与对应的路由 owner——请从那里开始查阅。

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

- 鉴权与 CSRF 完全关闭。每个请求都以 `system_default_user` 身份执行。
- 加入一层宽松的 CORS，使桌面 WebView（以及工具）可以自由调用 API。
- 仅本地可达的路由（如 `/api/auth/internal/*` 与 `/api/webui/*`）变为
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
| 鉴权 —— 仅本地 admin/internal | `/api/webui/*`、`/api/auth/internal/*` | 仅本地模式 | 同上 |
| 会话 | `/api/conversations/*`、`/api/messages/search` | 已鉴权 | [`nomifun-conversation/src/routes.rs`](../../crates/backend/nomifun-conversation/src/routes.rs)、[`routes_aux.rs`](../../crates/backend/nomifun-conversation/src/routes_aux.rs) |
| 智能体（本地 CLI 智能体） | `/api/agents/*` | 已鉴权 | [`nomifun-ai-agent/src/routes/agent.rs`](../../crates/backend/nomifun-ai-agent/src/routes/agent.rs) |
| 远程智能体 | `/api/remote-agents/*` | 已鉴权 | [`nomifun-ai-agent/src/routes/remote.rs`](../../crates/backend/nomifun-ai-agent/src/routes/remote.rs) |
| 设定 | `/api/presets/*` | 已鉴权 | [`nomifun-preset/src/routes.rs`](../../crates/backend/nomifun-preset/src/routes.rs) |
| 设定标签 | `/api/preset-tags/*` | 已鉴权 | 同上 |
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
| 伙伴 | `/api/companion/*` | 已鉴权 | [`nomifun-companion/src/routes.rs`](../../crates/backend/nomifun-companion/src/routes.rs) |
| WebUI/public 能力 companion token | `/api/webui/companions/{id}/access-token` | 已鉴权 / 本地 WebUI admin 流 | [`router/companion_token_routes.rs`](../../crates/backend/nomifun-app/src/router/companion_token_routes.rs) |
| Browser-use secrets | `/api/browser-secrets/*` | 已鉴权 | [`nomifun-secret/src/routes.rs`](../../crates/backend/nomifun-secret/src/routes.rs) |
| 文件系统 | `/api/fs/*` | 已鉴权 | [`nomifun-file/src/routes.rs`](../../crates/backend/nomifun-file/src/routes.rs) |
| Office 预览 | `/api/word-preview/*`、`/api/excel-preview/*`、`/api/ppt-preview/*`、`/api/document/convert`、`/api/preview-history/*`、`/api/star-office/detect` | 已鉴权 | [`nomifun-office/src/routes.rs`](../../crates/backend/nomifun-office/src/routes.rs) |
| Office iframe 代理 | `/api/ppt-proxy/*`、`/api/office-watch-proxy/*` | 公共（提供 iframe 内容；不鉴权） | 同上 |
| 设置 + 提供商 + 系统信息 | `/api/settings`、`/api/providers/*`、`/api/system/*` | 已鉴权 | [`nomifun-system/src/routes.rs`](../../crates/backend/nomifun-system/src/routes.rs) |
| 全局模型故障转移队列 | `/api/agent/model-failover` | 已鉴权 | [`router/model_failover.rs`](../../crates/backend/nomifun-app/src/router/model_failover.rs) |
| 连接探测（Bedrock 等） | `/api/bedrock/test-connection` | 已鉴权 | [`nomifun-system/src/bedrock_probe/routes.rs`](../../crates/backend/nomifun-system/src/bedrock_probe/routes.rs) |
| Shell 辅助 + STT | `/api/shell/*`、`/api/stt` | 已鉴权 | [`nomifun-shell/src/routes.rs`](../../crates/backend/nomifun-shell/src/routes.rs) |
| 公共资源（logo） | `/api/assets/logos/*` | 公共 | [`nomifun-assets/src/routes.rs`](../../crates/backend/nomifun-assets/src/routes.rs) |
| Public MCP front door | `/mcp/*` | companion-token / 已配置 public auth | [`nomifun-public/src/router.rs`](../../crates/backend/nomifun-public/src/router.rs) |
| Public MCP agent front door | `/mcp-agent/*` | companion-token / 已配置 public auth | [`nomifun-public/src/router.rs`](../../crates/backend/nomifun-public/src/router.rs) |
| Remote capability REST API | `/v1/*` | companion-token | [`nomifun-public/src/rest.rs`](../../crates/backend/nomifun-public/src/rest.rs) |
| 实时 WebSocket | `/ws` | 已鉴权（token 通过 `Sec-WebSocket-Protocol` 或查询串传递） | [`nomifun-realtime/src/handler.rs`](../../crates/backend/nomifun-realtime/src/handler.rs) |

如需各路由具体支持的方法，请阅读对应的 `routes.rs` 文件——每个 router
都在源文件内联声明自身的路由。

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

## WebSocket 事件模型

`/ws` 是用于流式更新的单一双向通道：智能体 token 流、终端输出，
以及需求、计划任务和协作任务的状态变化。

- 鉴权：通过 `GET /api/ws-token` 获得的 JWT，放在 WebSocket 的
  `Sec-WebSocket-Protocol` 请求头中（或 `Authorization`）。token 无效或
  过期 → 服务端发出 `auth-expired` 事件并以 `1008` 关闭。完全没有
  token → 以 `1008` 关闭，原因为 `"no token provided"`。
- 升级成功后，每条消息都是带 `type` 与 `payload` 的 JSON 对象。当域内
  事件发生时（新的智能体 token、一个终端字节、需求状态切换），由
  服务端推送；客户端通常无需回送任何内容。服务端把单一的
  `BroadcastEventBus` 多路复用给所有已连接客户端。
- 心跳：每 30 秒 ping 一次，60 秒超时（`HEARTBEAT_INTERVAL_MS` /
  `HEARTBEAT_TIMEOUT_MS`）。
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
- [疑难排查](./troubleshooting.zh.md) —— 常见的 API 与 WebSocket 故障
  形态。
- [Web 服务部署](../guides/web-server-deployment.md) —— 在 TLS 之后把
  API 暴露到网络上。
