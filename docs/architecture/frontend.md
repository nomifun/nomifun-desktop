# Frontend

The frontend is a single React 19 SPA in [`ui/`](../../ui/). The Tauri desktop
shell and the `nomifun-web` host load the same Vite build from `ui/dist`; the
renderer talks to the backend through HTTP and WebSocket, with a small Tauri
adapter only for desktop shell operations.

## Stack

| Concern | Current choice |
| --- | --- |
| Framework | React 19 + TypeScript |
| Bundler | Vite 6 |
| Routing | `react-router-dom` v7 with `HashRouter` |
| UI | Arco Design + custom CSS theme layers + UnoCSS |
| Data | SWR plus React contexts for app-shaped state |
| i18n | `i18next` / `react-i18next`; current app locales are `zh-CN` and `en-US` |
| Terminal | `xterm.js` with fit/web-links/webgl addons |
| Markdown | `react-markdown`, GFM, KaTeX, Mermaid |

## Source Layout

```text
ui/src/
├── common/       bridge/API/types/util code shared across hosts
├── platform/     small substrate for storage/logger/theme/runtime bridge
└── renderer/     React app: pages, layout, hooks, services, styles
```

The renderer imports the composite bridge from
`ui/src/common/adapter/ipcBridge.ts`. Most product operations are HTTP calls.
Tauri-specific operations are guarded behind `isTauri()` and implemented in the
adapter layer rather than scattered through pages.

## Backend URL And Trust

Desktop:

- `apps/desktop/src/main.rs` injects `window.__backendPort`.
- It also injects a per-boot `window.__nomiLocalTrust` secret.
- The init script patches `fetch` and `XMLHttpRequest` so requests to the
  embedded loopback backend include `x-nomi-local-trust`.

Web:

- No port is injected.
- The bridge uses same-origin `/api` and `/ws`.
- Authenticated web mode uses the session cookie plus CSRF double-submit header.

## Current Route Map

The source of truth is
[`ui/src/renderer/components/layout/Router.tsx`](../../ui/src/renderer/components/layout/Router.tsx).

| Route | Surface |
| --- | --- |
| `/login` | Login / first-run setup. |
| `/companion` | Desktop companion window route; outside the normal protected app layout. |
| `/guid` | Session start surface. |
| `/conversation/:id` | Conversation runtime. |
| `/terminal-new` | Terminal creation. |
| `/terminal/:id` | Terminal runtime. |
| `/models` | Model and agent management. |
| `/presets` | Reusable preset library. |
| `/skills` | Skills capability library. |
| `/mcp` | MCP server management. |
| `/open-capabilities` | Remote/public capability exposure. |
| `/scheduled`, `/scheduled/:job_id` | Scheduled tasks. |
| `/requirements`, `/requirements/extensions`, `/requirements/sources` | Requirements Platform, AutoWork, notification/source extensions. |
| `/nomi` | Companion configuration. |
| `/knowledge`, `/knowledge/:id` | Knowledge base list/detail. |
| `/settings/system` and related settings subroutes | System settings page and sub-sections. |

Legacy settings paths such as `/settings/model`, `/settings/agent`,
`/settings/capabilities`, `/settings/skills-hub`, `/settings/tools`,
`/settings/webui` and `/settings/webhook` are
redirects. Do not document them as primary navigation.

Agent collaboration has no standalone route or separate page. Its
AgentExecution projection is rendered inside the owning Conversation, so
navigation does not introduce another product object.

## State And Data

- SWR owns most remote list/detail state.
- `AuthProvider`, theme, feedback, preview, and conversation-history contexts
  own app-shaped state.
- `configService` initializes before i18n/theme consumers so early render reads
  backend-backed preferences.
- Realtime events arrive through a singleton WebSocket and are demuxed by event
  name.

## Desktop-Specific UX

Desktop shell behavior is implemented by Tauri commands and plugins:

- updater check,
- companion window reconciliation,
- WebUI LAN listener status/start/stop,
- keep-awake toggle,
- tray label localization,
- deep-link forwarding,
- tray close behavior.

Browser builds no-op or degrade desktop-only affordances in the adapter layer.
