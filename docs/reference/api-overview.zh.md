# API 概览

[English](api-overview.md)

桌面壳与 Web 宿主内嵌同一个 `nomifun-app` HTTP/WebSocket 后端。
本文用于定位契约，准确 method 和字段以路由声明、DTO 为准。

## Base URL

| 宿主 | 地址 |
| --- | --- |
| Desktop | `http://127.0.0.1:<picked-port>`；桌面壳注入端口和每次启动生成的本地信任密钥 |
| `nomifun-web` | 默认 `http://127.0.0.1:8787`，同端口提供 SPA/API |
| 独立 `nomicore` | 默认 `http://127.0.0.1:25808` |

Web 渲染器使用同源 `/api` 和 `/ws`，桌面 adapter 解析内嵌后端地址。

## 鉴权模型

- **Web 默认鉴权模式：** 登录设置 HttpOnly 的 `nomifun-session` cookie，
  并返回 JWT。请求可使用 cookie 或 Bearer token。
  通过 cookie 鉴权的写操作使用 `nomifun-csrf-token` cookie 和
  `x-csrf-token` header 双提交；公开 setup/login 路由有单独豁免和限流。
- **桌面本地信任：** `AuthPolicy::TrustLocalToken` 接受桌面壳每次启动的密钥。
  其他本机/网络客户端仍需正常鉴权。会展示宿主 UI 的变更可能同时要求安装 owner
  和本地产品信任。
- **明确关闭鉴权的开发模式：** `nomicore --local` 或
  `nomifun-web --insecure-no-auth` 关闭鉴权与 CSRF，只用于 loopback 或隔离可信网络。
- **外部集成：** `/mcp` 和 `/api/remote/*` 使用安装级 access token，
  详见[公开能力](../guides/remote-capability-api.zh.md)。

Owner 鉴权不自动授予所有原生桌面操作；需阅读路由的本地信任及所有权检查。

## 路由职责

从[路由组装](../../crates/backend/nomifun-app/src/router/routes.rs)和
[API 类型](../../crates/backend/nomifun-api-types/src)开始；
[后端地图](../architecture/backend-crates.zh.md)描述各功能职责。

| 能力 | 路由/源码入口 |
| --- | --- |
| 健康检查 | `/health`；[health router](../../crates/backend/nomifun-app/src/router/health.rs) |
| 登录、首次设置、会话鉴权 | `/login`、`/logout`、`/api/auth/*`、`/api/ws-token`；[鉴权路由](../../crates/backend/nomifun-auth/src/routes.rs) |
| Agent Session 与 Remote | `/api/agent-sessions/*`、`/api/remote/*`；[Session router](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs) |
| Agent 编排与能力目录 | [控制平面路由](../../crates/backend/nomifun-agent-control-plane/src/routes.rs) |
| 原生 Browser 与附加 Chrome | [Browser Workspace router](../../crates/backend/nomifun-app/src/router/browser_workspace.rs) |
| 应用和 MCP 来源 | [应用托管](../../crates/backend/nomifun-app/src/router/app_hosting.rs)、[来源观测](../../crates/backend/nomifun-app/src/router/app_mcp_sources.rs) |
| 创作画布/资产/任务 | [workshop routes](../../crates/backend/nomifun-workshop/src/routes.rs)、[creation routes](../../crates/backend/nomifun-creation/src/routes.rs) |
| 终端会话 | `/api/terminals/*`；[终端路由](../../crates/backend/nomifun-terminal/src/routes.rs) |
| MCP 与 Skills | [MCP 路由](../../crates/backend/nomifun-mcp/src/routes.rs)、[技能路由](../../crates/backend/nomifun-skill-library/src/skill_routes.rs) |
| 文件与 Office 预览 | [文件路由](../../crates/backend/nomifun-file/src/routes.rs)、[Office 路由](../../crates/backend/nomifun-office/src/routes.rs) |
| 设置/提供商/系统 | [系统路由](../../crates/backend/nomifun-system/src/routes.rs) |
| Canonical Remote MCP | `/mcp`；[公共 adapter](../../crates/backend/nomifun-public/src/canonical.rs) |
| 实时通信 | `/ws`；[handler](../../crates/backend/nomifun-realtime/src/handler.rs) |

功能文档不复制完整端点清单，应链接所属 router，仅解释必要的用户/集成契约。

Session 操作使用 canonical AgentSession 身份，会话消息是同一 Session 的投影，
详见 [Agent Session](../architecture/agent-session.zh.md)。
应用开发使用普通 Session，详见[应用托管](../architecture/app-hosting.zh.md)。

原生 Browser 管理由 Session 持有并要求本地信任，不创建 Agent 授权、不暴露原始
调试端点，也不在 Agent 持有运行时接受人工变更。
详见[浏览器平台](../architecture/browser-platform.zh.md)。

## WebSocket 事件模型

`/ws` 传递 JSON 应用事件，包括 Agent 流、终端、Browser 生命周期和
计划任务/需求变更，不作为原生 Browser 网页查看或输入通道。

Web 客户端通过 `GET /api/ws-token` 获取 token，通常经
`Sec-WebSocket-Protocol` 提交到 upgrade；桌面使用本地信任路径。
无效或过期鉴权以策略码 `1008` 关闭，正常关闭为 `1000`。
心跳默认 30s/60s。

消息使用 `type` 和 `payload`，忽略未知事件类型。
传输细节见[通信](../architecture/communication.zh.md)。

## 响应与限制

多数 handler 使用 `ApiResponse<T>`：

```json
{ "success": true, "data": {} }
```

失败返回对应 HTTP 状态及错误体。鉴权响应有独立类型，客户端应先核对 DTO。

公共请求体上限为 10 MiB，个别路由覆盖，如文件上传为 30 MiB。
限制及 cookie/header 常量见[配置](configuration.zh.md)和可执行
[常量定义](../../crates/backend/nomifun-common/src/constants.rs)。

首次设置、鉴权及连接失败见[疑难排查](troubleshooting.zh.md)。