# API Overview

[简体中文](api-overview.zh.md)

The desktop shell and Web host embed the same `nomifun-app` HTTP/WebSocket
backend. This page locates contracts; route declarations and DTOs remain the
reference for exact methods and fields.

## Base URL

| Host | Address |
| --- | --- |
| Desktop | `http://127.0.0.1:<picked-port>`; the shell injects the port and per-boot local trust secret |
| `nomifun-web` | `http://127.0.0.1:8787` by default, with SPA/API on one port |
| Standalone `nomicore` | `http://127.0.0.1:25808` by default |

The Web renderer uses same-origin `/api` and `/ws`; desktop adapters resolve
the embedded backend URL.

## Authentication model

- **Authenticated Web mode:** login sets an HttpOnly `nomifun-session` cookie
  and returns a JWT. Requests may use the cookie or a Bearer token.
  State-changing cookie-authenticated operations use the double-submit
  `nomifun-csrf-token` cookie and `x-csrf-token` header. Public
  setup/login routes have their own exemptions and rate limits.
- **Desktop local trust:** `AuthPolicy::TrustLocalToken` accepts the shell's
  per-boot trust secret. Other local/network clients still need normal
  authentication. Host-UI mutations can require both installation ownership
  and local product trust.
- **Explicit no-auth development:** `nomicore --local` or
  `nomifun-web --insecure-no-auth` disables authentication and CSRF.
  Keep this mode on loopback or an isolated trusted network.
- **Remote integrations:** `/mcp` and `/api/remote/*` use installation
  access tokens. See [public capabilities](../guides/remote-capability-api.md).

Owner authentication alone does not grant access to every native desktop
operation. Read the route's local-trust and ownership checks.

## Route ownership

Start with [router assembly](../../crates/backend/nomifun-app/src/router/routes.rs)
and [API types](../../crates/backend/nomifun-api-types/src).
The maintained [backend map](../architecture/backend-crates.md) describes
feature ownership.

| Surface | Route/source entry |
| --- | --- |
| Health | `/health`; [health router](../../crates/backend/nomifun-app/src/router/health.rs) |
| Login, setup, session auth | `/login`, `/logout`, `/api/auth/*`, `/api/ws-token`; [auth routes](../../crates/backend/nomifun-auth/src/routes.rs) |
| Agent Sessions and Remote | `/api/agent-sessions/*`, `/api/remote/*`; [Session router](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs) |
| Agent authoring and capability catalog | [control-plane routes](../../crates/backend/nomifun-agent-control-plane/src/routes.rs) |
| Native Browser and attached Chrome | [Browser Workspace router](../../crates/backend/nomifun-app/src/router/browser_workspace.rs) |
| Applications and MCP sources | [app hosting](../../crates/backend/nomifun-app/src/router/app_hosting.rs), [source observations](../../crates/backend/nomifun-app/src/router/app_mcp_sources.rs) |
| Creation canvases/assets/tasks | [workshop routes](../../crates/backend/nomifun-workshop/src/routes.rs), [creation routes](../../crates/backend/nomifun-creation/src/routes.rs) |
| Terminal sessions | `/api/terminals/*`; [terminal routes](../../crates/backend/nomifun-terminal/src/routes.rs) |
| MCP and Skills | [MCP routes](../../crates/backend/nomifun-mcp/src/routes.rs), [skill routes](../../crates/backend/nomifun-skill-library/src/skill_routes.rs) |
| Files and Office preview | [file routes](../../crates/backend/nomifun-file/src/routes.rs), [Office routes](../../crates/backend/nomifun-office/src/routes.rs) |
| Settings/providers/system | [system routes](../../crates/backend/nomifun-system/src/routes.rs) |
| Canonical Remote MCP | `/mcp`; [public adapter](../../crates/backend/nomifun-public/src/canonical.rs) |
| Realtime | `/ws`; [handler](../../crates/backend/nomifun-realtime/src/handler.rs) |

Do not copy complete endpoint lists into feature docs. Link the owning router
and explain only the user/integration contract needed there.

Session operations address the canonical AgentSession identity; conversation
messages are projections of that same Session. See
[Agent Session](../architecture/agent-session.md). Application development uses
ordinary Sessions; see [application hosting](../architecture/app-hosting.md).

Native Browser management is Session-owned and locally trusted. It does not
create an Agent grant, expose raw debug endpoints, or accept human mutations
while the Agent owns the run. See
[Browser platform](../architecture/browser-platform.md).

## WebSocket event model

`/ws` carries JSON application events, including Agent streams, terminals,
Browser lifecycle and scheduled/requirement changes. It is not a native Browser
page viewer or input transport.

The Web client obtains a token from `GET /api/ws-token` and presents it on
upgrade, normally using `Sec-WebSocket-Protocol`. Desktop uses its local-trust
path. Invalid/expired authentication closes with policy code `1008`;
normal closure uses `1000`. Heartbeat defaults are 30s/60s.

Messages use `type` and `payload`; ignore unknown event types.
Read [communication](../architecture/communication.md) for transport details.

## Responses and limits

Most handlers use `ApiResponse<T>`:

```json
{ "success": true, "data": {} }
```

Errors carry an appropriate HTTP status and error body. Auth responses use
their own typed envelopes; consult DTOs before implementing a client.

The common body limit is 10 MiB; selected routes override it, such as the
30 MiB file upload. Limits and cookie/header constants are maintained in
[configuration](configuration.md) and the executable
[constants](../../crates/backend/nomifun-common/src/constants.rs).

For setup, authentication and connection failures, see
[troubleshooting](troubleshooting.md).