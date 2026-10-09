# Current Technical Status

Updated: 2026-10-09.

This file is a compact current-state snapshot. Historical migration notes are
intentionally not kept here; use Git history for superseded transitions.

## Current Architecture

- One Cargo workspace:
  - `crates/agent/*`: 7 `nomi-*` crates.
  - `crates/backend/*`: 54 `nomifun-*` crates.
  - `crates/shared/*`: 4 cross-layer crates (`nomi-process-runtime`,
    `nomi-redact`, `nomi-ssh`, `nomifun-net`).
  - `apps/web` and `apps/desktop`.
- One frontend: `ui/`, a React 19 + Vite SPA.
- Two host modes:
  - Desktop: `apps/desktop`, Tauri 2 shell, embedded backend on loopback,
    local-trust header injected into `fetch` and `XMLHttpRequest`.
  - Web: `apps/web`, standalone server, authenticated by default, serves API,
    `/ws`, and `ui/dist` on one port.
- One backend composition root: `nomifun-app`, assembled through
  `AppServices`, `build_module_states`, and `create_router`.

## Active Product Surfaces

The current frontend route map lives in
`ui/src/renderer/components/layout/Router.tsx`. Active top-level surfaces are:

- `/guid` and `/conversation/:id`
- `/terminal-new` and `/terminal/:id`
- `/models`
- `/mcp`
- `/open-capabilities`
- `/skills`
- `/agent` and `/agent-sessions/:agentSessionId`
- `/requirements`, `/requirements/extensions`, `/requirements/sources`
- `/scheduled` and `/scheduled/:cron_job_id`
- `/nomi` and `/companion`
- `/customer-service` and `/customer-service/:cs_agent_id`
- `/knowledge` and `/knowledge/:id`
- `/plugins` and `/plugins/run/:id` (feature-gated; currently hidden)
- Creative Studio: the Canvas library at `/nomi/canvases`, Canvas editors at
  `/nomi/canvases/:canvasId`, plus `/asset-library`,
  `/asset-library/materials`, `/asset-library/prompts`, and
  `/asset-library/templates` (see
  `ui/src/renderer/pages/creativeStudio/app/resourceRoutes.ts`).
- `/settings/system` and `/settings/execution-engines`, plus system
  sub-sections routed through the system settings page
- `/settings/ssh-hosts` — the SSH remote-host book (instance owner only)
- `/settings/permissions`

Several legacy paths still exist only as redirects. Do not document them as
primary navigation.

Creative Studio has no Project domain. Canvas tasks use the
`CanvasNode { canvasId, nodeId }` owner; template executions use
`TemplateStep { templateId, templateRunId, templateStepId }`. The canonical
Canvas HTTP API is `/api/creative-studio/canvases`;
`/api/creative-studio/projects` remains a deprecated alias, and the old
project-named Gateway capabilities are retained only as deprecated aliases.

## Commands

Use the root script catalog:

```bash
bun run help
bun run dev
bun run dev:web
bun run build:ui
bun run check
bun run test
```

For packaging and signing, see:

- `docs/contributing/building-and-packaging.md`
- `apps/desktop/signing/README.md`
- `apps/desktop/updater/README.md`
- `packaging/linux/README.md`

## Known Documentation Policy

The active docs are `README.md`, `STATUS.md`, and the sections under `docs/`.
Dated design specs under `docs/specs/` are point-in-time contracts: they can
explain why code exists, but they must not be used as current product or
operator instructions without re-checking the source.
