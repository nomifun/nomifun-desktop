# Communication

NomiFun has several transport surfaces. They deliberately serve different
callers and security models.

## Channels

| Channel | Direction | Carries | Source |
| --- | --- | --- | --- |
| HTTP REST | UI/browser/client -> backend | CRUD, commands, setup, file operations, terminal input | `nomifun-app` route tree |
| WebSocket `/ws` | backend <-> UI | Agent stream events, terminal output, broadcast events, heartbeats | `nomifun-realtime` |
| Tauri IPC | SPA -> desktop shell | Desktop-only OS features | `apps/desktop/src/main.rs` + Tauri plugins |
| PTY stdio | backend <-> child process | Terminal session bytes, including third-party agent CLIs | `nomifun-terminal` |
| MCP stdio/HTTP | agent/backend/client <-> MCP server | Tools/resources/prompts | `nomi-mcp`, `nomifun-mcp`, `nomifun-public`, bridge subcommands |
| Canonical Remote ingress | external agents/scripts -> backend | Explicit AgentSession `open/turn/observe/cancel` | `/mcp`, `/api/remote/*` |

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
conversation, terminal, cron/requirement, channel, companion, SSH link, and
other events into the WebSocket manager.

`ssh.status` is the owner-scoped projection of one conversation's SSH link:
`{ sshHostId, conversationId, state, attempt, nextRetryInMs, hostFingerprint,
detail, retryable, reaped, changedAt }`, where `state` is one of `idle`,
`connecting`, `connected`, `degraded`, `reconnecting`, `dropped`, `closed`. It
rides the user bus rather than the per-turn agent stream because a link drops
and reconnects while a session sits idle, when no turn is open to carry the
news. `SshConnectionPool` publishes it only on a real state change, so a healthy
idle link costs the bus nothing, and `GET /api/ssh-hosts/statuses` serves the
same payload from the same watch value for reconnect resync.

Persistent Agent collaboration has exactly two realtime projections:
`agentExecution.changed { execution_id, sequence, change_kind }` invalidates
committed state, while `agentExecution.leadThinking` carries transient lead
thinking. Clients deduplicate the first by sequence and refill detail/events
over HTTP; no parallel execution-event family exists.

## Tauri IPC

Rust commands currently registered by the desktop shell include:

- `install_update`
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
- `terminal-hook`
- `doctor`
- `remote open`
- `remote turn`
- `remote observe`
- `remote cancel`

MCP injection differs by session and by caller:

- user MCP rows and OAuth-backed HTTP servers come from `nomifun-mcp`,
- requirement and knowledge servers are scoped internal MCP servers,
- platform Gateway tools are transported through `nomifun-gateway`,
- browser/computer bridges are feature-gated,
- public `/mcp` is the installation-token authenticated canonical Remote
  AgentSession front from `nomifun-public`.

Internal stdio bridges do not trust caller-supplied user ids or persisted
Conversation flags. The host derives an exact server-side scope and gives a
child process only a scoped, expiring, signed capability claim. Claim roots stay
inside the parent process; they are not serialized into runtime DTOs, database
rows, or child configuration. Public capability fronts use their own
installation-token boundary and do not inherit the internal host claim.

## Canonical Remote Ingress

The Fresh-v4 host mounts two projections of the same installation-token
authenticated Remote contract:

- `/mcp`: Streamable-HTTP MCP with exactly `open`, `turn`, `observe`, `cancel`;
- `/api/remote/*`: REST forms of the same four operations.

Each installation has one token. A caller acts as the installation owner with
no implicit companion, profile, persona, knowledge binding, or active thread.
`open` uses a local owner-created `RemoteBinding` and returns an explicit
UUIDv7 `agent_session_id`; every later operation carries that ID. The MCP
transport session remains connection lifecycle only.

## Quick Lookup

| Operation | Transport |
| --- | --- |
| Login/setup | HTTP `/api/auth/*` |
| Conversation send | HTTP `/api/conversations/*` plus streamed `/ws` events |
| Persistent Agent collaboration | HTTP `/api/agent-executions/*`; invalidation/thinking over `/ws` |
| Terminal input | HTTP terminal route; output over `/ws` |
| Desktop keep-awake | Tauri command |
| Remote AgentSession operations | MCP `/mcp` or REST `/api/remote/*` |
| Conversation turn | in-process `nomi` engine; tokens streamed over `/ws` |
| Terminal (incl. third-party agent CLIs) | child process stdio over a PTY managed by `nomifun-terminal` |
| Internal knowledge search for a session | `mcp-knowledge-stdio` bridge |

See [`agent-execution.zh.md`](agent-execution.zh.md) for the collaboration
aggregate, state transitions, event ordering, and three model-facing tools.
