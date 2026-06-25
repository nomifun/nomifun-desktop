# Current Technical Status

Updated: 2026-06-24.

This file is a compact current-state snapshot. Historical P0-P5 migration notes
were removed from the active status because they described the 2026-06-08
transition plan, not the product shape in this repository now.

## Current Architecture

- One Cargo workspace:
  - `crates/agent/*`: 15 `nomi-*` crates.
  - `crates/backend/*`: 29 `nomifun-*` crates.
  - `crates/shared/*`: 2 cross-layer crates.
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
- `/assistants`
- `/mcp`
- `/open-capabilities`
- `/requirements`, `/requirements/extensions`, `/requirements/sources`
- `/scheduled` and `/scheduled/:job_id`
- `/nomi`
- `/knowledge` and `/knowledge/:id`
- `/settings/system` plus system sub-sections routed through that page

Several legacy paths still exist only as redirects. Do not document them as
primary navigation.

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
