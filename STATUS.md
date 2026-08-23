# Current Technical Status

Updated: 2026-08-23.

This file is a compact current-state snapshot. Historical P0-P5 migration notes
were removed from the active status because they described the 2026-06-08
transition plan, not the product shape in this repository now.

## Current Architecture

- One Cargo workspace:
  - `crates/agent/*`: 15 `nomi-*` crates.
  - `crates/backend/*`: 34 `nomifun-*` crates.
  - `crates/shared/*`: 3 cross-layer crates (`nomi-process-runtime`,
    `nomi-redact`, `nomifun-net`).
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
- `/browser`
- `/presets`
- `/skills`
- `/requirements`, `/requirements/extensions`, `/requirements/sources`
- `/scheduled` and `/scheduled/:cron_job_id`
- `/nomi` and `/companion`
- `/customer-service` and `/customer-service/:cs_agent_id`
- `/knowledge` and `/knowledge/:id`
- Creative Studio focused shell: `/workshop`, the canonical Canvas library at
  `/workshop/canvases`, Canvas editors at `/workshop/canvas/:canvasId` and
  `/workshop/director/:canvasId`, independent workbenches at `/workshop/image`
  and `/workshop/video`, plus `/workshop/prompts`, `/workshop/assets`, and
  `/workshop/workflows`.
  `/workshop/projects` is a deprecated compatibility redirect to
  `/workshop/canvases`; it is not a Creative Studio product object.
  `/workshop/audio` is retired; see
  [`docs/guides/creative-studio.md`](docs/guides/creative-studio.md).
- `/settings/system` and `/settings/execution-engines`, plus system
  sub-sections routed through the system settings page
- `/settings/ssh-hosts` — the SSH remote-host book (instance owner only)

Several legacy paths still exist only as redirects. Do not document them as
primary navigation.

Creative Studio has no Project domain. Canvas tasks use the
`CanvasNode { canvasId, nodeId }` owner; standalone Image/Video tasks use only
`StandaloneWorkbench { workbenchKind }`; WorkflowStep remains unchanged.
Standalone history and retirement are scoped only by `workbench_kind`, and
legacy standalone `project_id` values are inert provenance. Image and Video
routes have no Canvas selector or prerequisite and are usable with zero
Canvases. The canonical Canvas HTTP API is
`/api/creative-studio/canvases`; `/api/creative-studio/projects` remains a
deprecated alias. Gateway Canvas capabilities are
`nomi_creative_studio_list_canvases` and
`nomi_creative_studio_get_canvas`, with the old project-named capabilities
retained only as deprecated aliases. The UI/API contract version is 22.

Creative Studio writes version-2 Canvas archives while retaining a version-1
reader. Image and Video preserve versioned per-workbench session drafts in
browser session storage without `projectId` or `canvasId` keys.

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

The active docs are `README.md`, `STATUS.md`, and the non-archive sections under
`docs/`. Dated design specs, audits, and Superpowers implementation plans are
historical records. They can explain why code exists, but they must not be used
as current product or operator instructions without re-checking the source.
