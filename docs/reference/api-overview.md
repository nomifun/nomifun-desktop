# API Overview

NomiFun's backend (`nomifun-app`, binary `nomicore`) exposes a single axum HTTP server. The SPA, the desktop shell, and any external integration all talk to it the same way: JSON over HTTP for command/query, WebSocket for streaming events.

This page is an **orientation**, not an exhaustive endpoint reference. The full surface lives in the route modules under `crates/backend/`; the source is the canonical reference. Group base paths and the routers that own each one are listed below — start there.

## Base URL

| Host | Default base URL | Notes |
|---|---|---|
| `nomifun-desktop` | `http://127.0.0.1:<picked-port>` | Picks a free localhost port at startup. The renderer learns the port over IPC and uses `/api` and `/ws` against it. |
| `nomifun-web` | `http://<host>:<port>` (`http://127.0.0.1:8787` by default) | Same backend, served alongside the SPA on one port. |
| `nomicore` standalone | `http://127.0.0.1:25808` | Backend run on its own — useful for debugging. |

The SPA uses **relative paths** (`/api/...`, `/ws`). There is no separate API server to point clients at — the SPA and the API are co-located.

## Authentication model

NomiFun runs under one of three auth policies, decided at startup:

### Authenticated mode (default for `nomifun-web`)

- Login via `POST /login` returns a session JWT in both a cookie (`nomifun-session`, `HttpOnly`) and the JSON body. Subsequent requests authenticate via the cookie or an `Authorization: Bearer …` header.
- State-changing requests must additionally include the CSRF header `x-csrf-token` matching the `nomifun-csrf-token` cookie (Double Submit Cookie pattern). Safe methods (`GET`, `HEAD`, `OPTIONS`) bypass CSRF; the login/setup/qr-login endpoints are exempt because they have no session yet.
- WebSocket upgrades carry the same JWT — typically via `Sec-WebSocket-Protocol`, fetched from `GET /api/ws-token`. The `/ws` route is exempt from CSRF (no cookie-based double-submit on a WebSocket upgrade) but is otherwise authenticated.
- Rate limiters apply per-client to login attempts, general API traffic, and authenticated state-changing actions.

### Desktop local-trust mode (`nomifun-desktop`)

- The embedded backend uses `AuthPolicy::TrustLocalToken`.
- The desktop WebView receives a per-boot secret (`window.__nomiLocalTrust`) and
  presents it on HTTP/WebSocket requests.
- Other clients, even on the same machine, are not trusted unless they have a
  normal authenticated session. This is what makes WebUI remote access safe to
  expose behind login.

### No-auth local mode (`--local` on `nomicore`, or `--insecure-no-auth` on the web host)

- Authentication and CSRF are turned off entirely. Every request acts as the
  installation owner recorded in the database.
- A permissive CORS layer is added so the desktop WebView (and tooling) can call the API freely.
- Local-only routes such as `/api/webui/*` become reachable.

The trust boundary in local mode is the network — only ever expose it on loopback or a fully trusted private network. The web host loudly logs a warning if `--insecure-no-auth` is combined with a non-loopback bind.

## Body size and limits

- The default request body limit is **10 MiB** (`BODY_LIMIT` in `nomifun-common`). Routes that legitimately need more (file upload, ZIP creation, …) install their own larger limit — `/api/fs/upload` accepts up to 30 MiB.
- Remote images downloaded on behalf of the user are capped at 5 MiB and follow at most 5 redirects.

## Route groups

Each group is owned by a specific crate. The base path is the actual URL prefix mounted into the app router; auth applies in authenticated mode and desktop local-trust mode.

| Group | Base path | Auth | Owning crate / file |
|---|---|---|---|
| Health | `/health` | public | [`router/health.rs`](../../crates/backend/nomifun-app/src/router/health.rs) |
| Auth — login / setup / status / refresh | `/login`, `/logout`, `/api/auth/*`, `/api/ws-token`, `/qr-login` | mixed (login/setup/qr-login: public; rest: authenticated) | [`nomifun-auth/src/routes.rs`](../../crates/backend/nomifun-auth/src/routes.rs) |
| Auth — local-only admin/internal | `/api/webui/*` | local mode only | same as above |
| Conversations | `/api/conversations/*`, `/api/messages/search` | authenticated | [`nomifun-conversation/src/routes.rs`](../../crates/backend/nomifun-conversation/src/routes.rs), [`routes_aux.rs`](../../crates/backend/nomifun-conversation/src/routes_aux.rs) |
| Agents | `/api/agents/*` | authenticated | [`nomifun-ai-agent/src/routes/agent.rs`](../../crates/backend/nomifun-ai-agent/src/routes/agent.rs) |
| SSH hosts | `/api/ssh-hosts/*` | instance owner only | [`nomifun-ssh/src/routes.rs`](../../crates/backend/nomifun-ssh/src/routes.rs) |
| Presets | `/api/presets/*` | authenticated | [`nomifun-preset/src/routes.rs`](../../crates/backend/nomifun-preset/src/routes.rs) |
| Preset tags | `/api/preset-tags/*` | authenticated | same as above |
| MCP servers | `/api/mcp/*` | authenticated | [`nomifun-mcp/src/routes.rs`](../../crates/backend/nomifun-mcp/src/routes.rs) |
| Skills | `/api/skills/*` | authenticated | [`nomifun-extension/src/skill_routes.rs`](../../crates/backend/nomifun-extension/src/skill_routes.rs) |
| Extensions | `/api/extensions/*` | authenticated | [`nomifun-extension/src/routes.rs`](../../crates/backend/nomifun-extension/src/routes.rs) |
| Hub (extension marketplace) | `/api/hub/*` | authenticated | [`nomifun-extension/src/hub_routes.rs`](../../crates/backend/nomifun-extension/src/hub_routes.rs) |
| Cron jobs | `/api/cron/*` | authenticated | [`nomifun-cron/src/routes.rs`](../../crates/backend/nomifun-cron/src/routes.rs) |
| Channels (IM bridges) | `/api/channel/*` | authenticated | [`nomifun-channel/src/routes.rs`](../../crates/backend/nomifun-channel/src/routes.rs) |
| Webhooks + tag settings | `/api/webhooks/*`, `/api/tags/{tag}/settings` | authenticated | [`nomifun-webhook/src/routes.rs`](../../crates/backend/nomifun-webhook/src/routes.rs) |
| Requirements (project board) | `/api/requirements/*` | authenticated | [`nomifun-requirement/src/routes.rs`](../../crates/backend/nomifun-requirement/src/routes.rs) |
| AutoWork / IDMM | `/api/idmm/*`, `/api/requirements/autowork*` | authenticated | [`nomifun-idmm/src/routes.rs`](../../crates/backend/nomifun-idmm/src/routes.rs) |
| Agent executions | `/api/agent-executions/*` | authenticated | [`nomifun-agent-execution/src/routes.rs`](../../crates/backend/nomifun-agent-execution/src/routes.rs) |
| Terminals | `/api/terminals/*` | authenticated | [`nomifun-terminal/src/routes.rs`](../../crates/backend/nomifun-terminal/src/routes.rs) |
| Knowledge bases | `/api/knowledge/*` | authenticated | [`nomifun-knowledge/src/routes.rs`](../../crates/backend/nomifun-knowledge/src/routes.rs) |
| Creative Studio management and generation | `/api/creative-studio/*` management families: projects, assets, prompts, templates/runs/drafts, tasks, agent sessions, and collections | instance owner only | [`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs), [`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs), [`nomifun-conversation/src/routes.rs`](../../crates/backend/nomifun-conversation/src/routes.rs) |
| Creative Studio media delivery | `GET /api/creative-studio/files/{asset_id}` | public read-only capability URL; no list or mutation surface | [`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs) |
| Unified Plugin Core | `/api/plugins`, `/api/plugins/{plugin_id}/*`, and `/api/plugin-drafts/*`: library, Chat/Draft authoring, shared directory/ZIP/Backup import, configuration, lifecycle, Package/Backup export, and Preview | instance owner; mutations additionally require local product trust | [`router/plugin.rs`](../../crates/backend/nomifun-app/src/router/plugin.rs) |
| Plugin Surface assets and bridge | descriptor-fenced asset and bridge routes containing the exact session generation and Artifact digest | instance owner with the matching live Surface descriptor | same as above |
| Companion | `/api/companion/*` | authenticated | [`nomifun-companion/src/routes.rs`](../../crates/backend/nomifun-companion/src/routes.rs) |
| NomiFun Desktop access token | `/api/webui/access-token` | local-trust installation-owner flow | [`router/instance_token_routes.rs`](../../crates/backend/nomifun-app/src/router/instance_token_routes.rs) |
| Conversation Browser Workspace | `/api/conversations/{conversation_id}/browser*` | installation owner with local product trust; conversation ownership is checked and delegated execution steps are rejected | [`router/browser_workspace.rs`](../../crates/backend/nomifun-app/src/router/browser_workspace.rs) |
| System-browser connection | `/api/conversations/{conversation_id}/system-browser*` | installation owner with local product trust; conversation ownership is checked; currently Windows-only | [`router/system_browser.rs`](../../crates/backend/nomifun-app/src/router/system_browser.rs) |
| Filesystem | `/api/fs/*` | authenticated | [`nomifun-file/src/routes.rs`](../../crates/backend/nomifun-file/src/routes.rs) |
| Office preview | `/api/word-preview/*`, `/api/excel-preview/*`, `/api/ppt-preview/*`, `/api/preview-history/*` | authenticated | [`nomifun-office/src/routes.rs`](../../crates/backend/nomifun-office/src/routes.rs) |
| Office iframe proxies | `/api/ppt-proxy/*`, `/api/office-watch-proxy/*` | public (serve iframe content; no auth) | same as above |
| Settings + providers + system info | `/api/settings`, `/api/providers/*`, `/api/system/*` | authenticated | [`nomifun-system/src/routes.rs`](../../crates/backend/nomifun-system/src/routes.rs) |
| Host system permissions | `GET /api/system/permissions` reads live Microphone, Accessibility, and Screen Recording state; `POST /api/system/permissions/{request,open-settings}` requests access or opens the matching pane | installation owner for reads; mutations that display host UI additionally require local product trust | [`router/computer_permissions.rs`](../../crates/backend/nomifun-app/src/router/computer_permissions.rs) |
| Global model failover queue | `/api/agent/model-failover` | authenticated | [`router/model_failover.rs`](../../crates/backend/nomifun-app/src/router/model_failover.rs) |
| Connection probes (Bedrock, …) | `/api/bedrock/test-connection` | authenticated | [`nomifun-system/src/bedrock_probe/routes.rs`](../../crates/backend/nomifun-system/src/bedrock_probe/routes.rs) |
| Shell helpers + STT | `/api/shell/*`, `/api/stt` | authenticated | [`nomifun-shell/src/routes.rs`](../../crates/backend/nomifun-shell/src/routes.rs) |
| Public assets (logos) | `/api/assets/logos/*` | public | [`nomifun-assets/src/routes.rs`](../../crates/backend/nomifun-assets/src/routes.rs) |
| Canonical Remote MCP front door | `/mcp` | installation token | [`nomifun-public/src/canonical.rs`](../../crates/backend/nomifun-public/src/canonical.rs) |
| Canonical Remote REST API | `/api/remote/open`, `/api/remote/turn`, `/api/remote/observe`, `/api/remote/cancel` | installation token | [`nomifun-app/src/router/remote_rest.rs`](../../crates/backend/nomifun-app/src/router/remote_rest.rs) |
| Realtime WebSocket | `/ws` | authenticated (token in `Sec-WebSocket-Protocol` or query) | [`nomifun-realtime/src/handler.rs`](../../crates/backend/nomifun-realtime/src/handler.rs) |

For the exact set of methods on each route, read the corresponding `routes.rs` file — every router declares its routes inline.

### Selected auth endpoints

These are the auth endpoints clients are most likely to interact with directly:

| Method + path | Purpose |
|---|---|
| `POST /login` | Username + password login. Returns `{success, user, token}` and sets the session cookie. CSRF-exempt. Rate-limited. |
| `POST /api/auth/setup` | One-time first-run admin creation on a fresh install. Atomic; concurrent callers race on a conditional UPDATE so only one wins (the others get `409 Conflict`). CSRF-exempt. |
| `POST /logout` | Blacklists the current token; clears the session cookie. |
| `GET  /api/auth/status` | Public — reports `{needs_setup, user_count, is_authenticated}`. Useful as a liveness/health probe. |
| `GET  /api/auth/user` | Returns the current `{id, username}`. |
| `POST /api/auth/change-password` | Changes the current user's password and rotates the JWT secret (invalidating every other session). |
| `POST /api/auth/refresh` | Refreshes a token that is still valid but near expiry. |
| `GET  /api/ws-token` | Returns the token to use for the WebSocket upgrade. |
| `POST /api/auth/qr-login` | Consume a one-shot QR-login token (issued via the WebUI remote-access flow). |
| `GET  /qr-login` | Static HTML page that completes a QR login redirect from a phone scanner. |

### Browser platform endpoints

The interactive Browser is a native surface owned by one conversation. Its HTTP
API supports the conversation UI; Agent observation and input use the frozen
`Browser` tool binding for the active turn rather than an HTTP management API.
Both paths address the same tabs and profile. While an Agent run is active,
human commands fail closed; after the turn ends, the user can operate the same
page. There is no user-control transfer state.

| Method + path | Purpose |
|---|---|
| `GET /api/conversations/{conversation_id}/browser` | Read the Browser Workspace snapshot without creating a runtime. |
| `POST /api/conversations/{conversation_id}/browser` | Ensure the conversation's native Browser Workspace and return its snapshot. |
| `DELETE /api/conversations/{conversation_id}/browser` | Close an idle Browser Workspace using its exact `runtime_generation`; stale or active requests are rejected. |
| `POST /api/conversations/{conversation_id}/browser/commands` | Run one typed user command against the native tabs while input is user-owned: create/activate/close/navigate/history/reload, close all pages, open Downloads or the current URL externally, cancel an owned download, answer a website permission/dialog, or clear this conversation's site data. |
| `GET /api/conversations/{conversation_id}/system-browser` | Read the separate system-browser connection and authorized-tab state. |
| `POST /api/conversations/{conversation_id}/system-browser` | Connect to an already-running Chrome instance after the user enabled Chrome remote debugging. NomiFun does not launch Chrome or import its profile. |
| `DELETE /api/conversations/{conversation_id}/system-browser` | Disconnect the exact connection without closing Chrome or its tabs. |
| `POST /api/conversations/{conversation_id}/system-browser/choices` | List the current connection's candidate tabs for explicit authorization. |
| `POST /api/conversations/{conversation_id}/system-browser/tabs` | Authorize one exact tab choice for this conversation. |

These routes are mounted only for the locally trusted desktop product and are
also protected by installation-owner authentication. They never return raw
protocol endpoints, debugging ports, profile paths, cookies, or credentials.
Mutable system-browser operations are unavailable while the Agent owns the
conversation run.

`nomi_local_websearch` has no Browser-management endpoint. It is an independently
selectable Agent tool backed by an isolated background browser and does not use
the conversation profile. Likewise, `nomi_system_browser` is an independent
Agent capability; enabling it does not enable the embedded Browser or local
search.

## WebSocket event model

`/ws` is the shared JSON realtime channel for application updates: agent token streams, terminal output, Browser inventory/lifecycle events, and requirement / scheduled-task / collaboration state changes. It remains the general application event channel; it is not a Browser page viewer or input channel.

- Authentication: a JWT obtained from `GET /api/ws-token`, sent in the WebSocket `Sec-WebSocket-Protocol` header (or `Authorization`). Invalid or expired token → server sends an `auth-expired` event and closes with code `1008`. No token at all → close with `1008`, reason `"no token provided"`.
- After a successful upgrade, every message is a JSON object with a `type` and a `payload`. Messages are pushed by the server when domain events occur (a new agent token, a terminal byte, a requirement transition); clients usually do not need to send anything back. The server multiplexes a single `BroadcastEventBus` to every connected client.
- Heartbeats: ping every 30s, timeout at 60s (`HEARTBEAT_INTERVAL` / `HEARTBEAT_TIMEOUT` in `nomifun-realtime`).
- Close codes: `1000` for a normal close, `1008` for policy violations (auth failure, invalid token).

The set of `type` values is open-ended — extensions and feature modules emit their own. Treat unknown types as forward-compatible: ignore them.

## Response envelope

Most JSON responses use the same shape (`ApiResponse<T>` from `nomifun-api-types`):

```json
{ "success": true, "data": { ... } }
```

Errors are returned with the appropriate HTTP status and a body like:

```json
{ "success": false, "error": "Invalid username or password" }
```

The login/setup/refresh handlers return slightly enriched envelopes (`LoginResponse`, `RefreshResponse`) — they include the token or user object inline.

## Source-of-truth pointers

The list above is meant to get you to the right module. From there, read the source — every router declares its routes in one place, and every handler is in the same file or the next one over. The router assembly itself is in [`crates/backend/nomifun-app/src/router/routes.rs`](../../crates/backend/nomifun-app/src/router/routes.rs); the middleware stack (CSRF, security headers, body limit, optional CORS) is also there.

## See also

- [Configuration Reference](./configuration.md) — flags, env vars, the auth secret resolution order.
- [Browser Platform Architecture](../architecture/browser-platform.md) — the native conversation Browser Workspace, independent system-browser connection, isolated background runtimes, and lifecycle guarantees.
- [Troubleshooting](./troubleshooting.md) — common API and WebSocket failure modes.
- [Web Server Deployment](../guides/web-server-deployment.md) — exposing the API over the network behind TLS.
