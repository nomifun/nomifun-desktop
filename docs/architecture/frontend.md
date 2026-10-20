# Frontend

The frontend is a single React 19 SPA in [`ui/`](../../ui/). The Tauri desktop
shell and the `nomifun-web` host load the same Vite build from `ui/dist`; the
renderer talks to the backend through HTTP and WebSocket, with a small Tauri
adapter only for desktop shell operations.

The shared renderer targets desktop-class surfaces only: the Tauri window and
desktop WebUI browsers, with a minimum supported viewport of 880x600. Phone and
tablet layouts, touch-only fallbacks, safe-area handling, and sub-880px viewport
breakpoints are outside this repository's UI contract. Narrow content panes
within a supported desktop window should adapt with container-aware layout.

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
| `/agent` | Agent settings and Agent Presets. |
| `/agent-sessions/:agentSessionId` | Single Agent session page. |
| `/skills` | Skills capability library. |
| `/mcp` | MCP server management. |
| `/connections` | Connections & Devices: phone pairing and relay access, other-computer browser access, and external MCP/REST tools (`task` = `phone`, `browser`, or `tools`). |
| `/scheduled`, `/scheduled/:cron_job_id` | Scheduled tasks. |
| `/requirements`, `/requirements/extensions`, `/requirements/sources` | Requirements Platform, AutoWork, notification/source extensions. |
| `/nomi` | Companion configuration. |
| `/customer-service`, `/customer-service/:cs_agent_id` | Customer-service roster/detail. |
| `/knowledge`, `/knowledge/:id` | Knowledge base list/detail. |
| `/nomi/canvases` | Canonical Canvas library. |
| `/nomi/canvases/:canvasId` | Canvas editor. |
| `/asset-library` | Redirects to `/asset-library/materials`. |
| `/asset-library/materials`, `/asset-library/prompts`, `/asset-library/templates` | Asset library: materials, prompt catalog, private Template Studio. |
| `/apps` | Application hosting list, current run settings, import, commands and system-browser links. |
| `/settings/system`, `/settings/execution-engines`, `/settings/ssh-hosts`, `/settings/permissions`, `/settings/about` | Settings surfaces. |

Legacy settings paths such as `/settings/model`, `/settings/capabilities`,
`/settings/skills-hub`, `/settings/tools`, `/settings/display`,
`/settings/webui`, `/settings/computer-use`, `/settings/voice-input` and
`/settings/webhook` are redirects, as are `/open-capabilities`, `/requirements/kanban`,
`/requirements/new`, `/requirements/tag-sessions`, `/autowork` and `/other`.
Do not document them as primary navigation.

Creation reuses the normal app titlebar and swaps the primary rail to a
Settings-style product navigation surface, with **Back to Workbench** pinned at
the bottom. During an app session, the main rail entry resumes the last complete
Creation location that passes the product's exact route matcher; invalid
or unknown stored locations fall back to `/nomi/canvases`. The product rail
has no separate home item because prompt-led creation lives in the Canvas
Assistant. Its route constants and exact-match rules live in
[`pages/creativeStudio/app/resourceRoutes.ts`](../../ui/src/renderer/pages/creativeStudio/app/resourceRoutes.ts).
Stored `/workshop/*` resume locations are rewritten by
`migratedCreativeRoute` there; the legacy paths are not mounted as routes.

Creation has Canvases only:
the canonical HTTP resource is `/api/creative-studio/canvases`, while
`/api/creative-studio/projects` is a deprecated alias. The Gateway's current
Canvas capabilities are `nomi_creative_studio_list_canvases` and
`nomi_creative_studio_get_canvas`; old project-named capabilities are deprecated
aliases.

Canvas exports use an archive v3 writer and retain v1/v2 readers.

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

## Theme and internationalization

`useTheme` selects light/dark mode; `useColorScheme` selects the brand scheme, defaulting to `default`. Prefer semantic tokens in components. `AppThemeRuntime` and `applyCustomCss` inject preset CSS and apply `!important`. Light/dark token sets are complete and symmetric, selectors cover `:root`/`body`, and Arco RGB scales use comma-separated triples. Themes exclude broad selectors, layout properties and external resources. Popover alpha is at least .86; feedback/tooltips use solid colors, and `.message-item` outer wrappers receive no decoration. See the [styling contract](../contributing/styling.md).

i18next uses static `zh-CN`/`en-US` bundles and shared normalization from [common/config/i18n.ts](../../ui/src/common/config/i18n.ts). Backend settings own language preferences; localStorage is an initial-render hint. The current `changeLanguage`/preference helper synchronizes the main window, companion and other surfaces rather than updating independent instances. [localeKeyParity.ts](../../ui/src/renderer/services/i18n/localeKeyParity.ts) validates plural keys against each locale's categories, without demanding identical plural suffixes across languages.

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


## Computer Use interface

The `nomifun.computer` module is owned by `nomifun-app`’s `computer_host`, using the five host modules in `nomifun-computer` and an independent Cua worker. Gateway does not register desktop tools. The UI derives observation/interaction/application/clipboard groups, titles and foreground marks from frozen metadata. Status is `/api/computer/status`; OS access uses `/api/system/permissions`. UI/API version is 44. Retired Computer sessions ask for a new conversation while keeping history. See [Computer Use](computer-use.md) for architecture and outstanding platform acceptance.
