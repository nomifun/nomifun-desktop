# Communication

NomiFun has several transport surfaces. They deliberately serve different
callers and security models.

## Channels

| Channel | Direction | Carries | Source |
| --- | --- | --- | --- |
| HTTP REST | UI/browser/client -> backend | CRUD, commands, setup, file operations, terminal input | `nomifun-app` route tree |
| WebSocket `/ws` | backend <-> UI | Agent stream events, terminal output, broadcast events, heartbeats | `nomifun-realtime` |
| Tauri IPC | SPA -> desktop shell | Desktop-only OS features | `apps/desktop/src/main.rs` + Tauri plugins |
| ACP/agent stdio | backend <-> child CLI | External CLI-agent conversation traffic | `nomifun-ai-agent` |
| MCP stdio/HTTP | agent/backend/client <-> MCP server | Tools/resources/prompts | `nomi-mcp`, `nomifun-mcp`, `nomifun-public`, bridge subcommands |
| Public Remote fronts | external agents/scripts -> backend | MCP tools or REST capability calls | `/mcp`, `/mcp-agent`, `/v1` |

## Auth Modes

The backend resolves trust through `nomifun-auth` and the `AppServices`
configuration:

- **Required**: normal web mode. Login cookie is required for `/api/*`; CSRF
  protects state-changing cookie-authenticated requests.
- **NoAuth**: explicit insecure mode, used only through flags such as
  `--insecure-no-auth` for trusted loopback/private use.
- **TrustLocalToken**: desktop shell mode. The webview gets a per-boot secret
  and sends it as `x-nomi-local-trust`; middleware resolves that request to the
  trusted local user. This is not the same as the old blanket `--local` story.

WebSocket auth accepts the normal authenticated browser path and the local-trust
path used by the desktop shell.

## HTTP And WebSocket

The SPA bridge in `ui/src/common/adapter/httpBridge.ts` selects:

- same-origin URLs for `nomifun-web`,
- `http://127.0.0.1:<window.__backendPort>` for the desktop webview.

`/ws` is a singleton connection per page lifetime. The backend event bus fans
conversation, terminal, cron/requirement, channel, companion, and other events
into the WebSocket manager.

## Tauri IPC

Rust commands currently registered by the desktop shell include:

- `check_for_updates`
- `sync_companion_windows`
- `webui_get_status`
- `webui_start`
- `webui_stop`
- `set_keep_awake`
- `set_tray_labels`

The renderer also uses Tauri JS APIs/plugins for window, dialog, notification,
process, autostart, deep-link, updater, and path operations where appropriate.

## MCP And Agent Bridges

The current `nomicore` CLI subcommands include:

- `mcp-requirement-stdio`
- `mcp-knowledge-stdio`
- `mcp-gateway-stdio`
- `mcp-open-stdio`
- `mcp-computer-stdio`
- `mcp-browser-stdio`
- `terminal-hook`
- `doctor`
- `tools`
- `call`
- `agent`

Older docs that mention `mcp-bridge`, `mcp-guide-stdio`, or `mcp-team-stdio`
are historical and predate the current bridge set.

MCP injection differs by runtime:

- user MCP rows and OAuth-backed HTTP servers come from `nomifun-mcp`,
- requirement and knowledge servers are scoped internal MCP servers,
- Desktop Gateway tools are exposed through `nomifun-gateway`,
- browser/computer bridges are feature-gated,
- public `/mcp` and `/mcp-agent` are companion-token authenticated fronts from
  `nomifun-public`.

## Public Capability Fronts

The full app router mounts three companion-token authenticated surfaces outside
the normal `/api` browser-auth tree:

- `/mcp`: general MCP profile for a companion identity,
- `/mcp-agent`: curated agent profile,
- `/v1`: REST capability adapter, with optional agent profile selection.

Tokens are per companion. A caller acts as that companion and inherits the
associated profile, model/persona choices, and scoped capabilities.

## Quick Lookup

| Operation | Transport |
| --- | --- |
| Login/setup | HTTP `/api/auth/*` |
| Conversation send | HTTP `/api/conversations/*` plus streamed `/ws` events |
| Terminal input | HTTP terminal route; output over `/ws` |
| Desktop keep-awake | Tauri command |
| Remote MCP tool call | `/mcp` or `/mcp-agent` |
| Remote REST capability call | `/v1` |
| Agent CLI conversation | child process stdio managed by `nomifun-ai-agent` |
| Internal knowledge search for ACP session | `mcp-knowledge-stdio` bridge |
