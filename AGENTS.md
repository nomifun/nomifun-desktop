# Repository Guidelines

## Supported UI targets

- `nomifun-desktop` supports the Tauri desktop shell and desktop-class WebUI
  browsers only. The shared renderer's minimum supported viewport is 880x600,
  matching the desktop window contract.
- Do not add phone or tablet layouts, mobile drawers/action sheets, touch-only
  fallbacks, safe-area rules, or viewport breakpoints below 880px. Do not use
  device, user-agent, or touch detection to select renderer layouts. Use
  container queries for narrow panes inside a supported desktop layout when
  necessary.
- Do not run or add phone emulation, mobile viewport snapshots, or mobile-only
  UI acceptance cases. Keep the separate Mobile product and remote-client wire
  compatibility out of renderer layout decisions.
- Run `bun run check:desktop-ui-boundary` after renderer or UI-rule changes.
  Changing this target contract requires an explicit product decision.

## Page headings

- New pages with a visible page title must use the shared `PageHeader` (or
  `HubPageShell`) and its `app-page-title` style. Keep the shared title tokens
  (`--page-title-size`, `--page-title-line-height`), weight and color consistent;
  do not introduce page-specific title typography.
- Place page-level actions such as create/import and search in the header's
  `actions` area, aligned to the right on the same row as the title when
  applicable. Keep content filters in the page body.

## Efficient validation

- Choose the smallest checks that directly cover the files and behavior being
  changed. Documentation-only edits do not require a full build or test run.
- Use targeted UI, Rust, packaging, or platform checks for changes in those
  areas. Run broad commands such as `bun run check` or the full test suite for
  cross-cutting changes, releases, or when a narrower check cannot establish
  confidence.
- If a relevant command is unavailable or platform-specific, report what was
  not run and why. It is not a repository-wide commit blocker for unrelated
  changes.

## Canonical Agent Session

- AgentSession, Turn, Event, Payload and Effect are the sole Agent fact chain.
  Conversation APIs and Message projections use the same Session identity.
- Runtime context must read typed canonical events and payloads. Do not restore
  projection-to-context fallback, old transcript readers, dual writes, lossy
  field mirrors, legacy draft import, or a second checkpoint/binding ledger.
- Retire replaced code, schema fields, DTOs, fixtures, tests and design documents
  in the same change. Use Git history for superseded designs; do not retain
  archived plans as current development inputs or generator requirements.
- Preserve explicit current-generation Agent/model transitions and native
  checkpoint safety. These are not authorization to import retired Agent data.
- Agent clean cut clears Agent data while preserving non-Agent configuration.
  Destructive-release startup rebuilds readable incompatible database lineages
  through the dataset coordinator without converting historical data. Database
  I/O failures and damage to a supported schema fail closed; backup and restore
  never obtain startup rebuild authority. More migration rows do not prove a
  newer application version across destructive baseline changes.
- Scheduling, delegation, controls and resource retirement follow
  `docs/architecture/agent-execution.zh.md`; tested scope and remaining platform
  evidence are recorded in `docs/validation/agent-lifecycle.zh.md`.
- Read `docs/architecture/agent-session.zh.md` before changing Session storage
  or content. Run `bun run check:agent-session-boundary` and the affected
  canonical history, reset and consumer tests.
- Repository scripts and GitHub Actions may be used when they are useful and
  maintainable; no workflow-presence audit is required for ordinary changes.

## Git workflow

- Use the contributor's configured Git identity. The repository does not
  install or require custom attribution hooks.
- Preserve unrelated work and inspect staged files before committing.
- Do not force-push, rewrite shared history, or expose credentials without
  explicit authorization.

## Active Computer Use rebuild

- For Computer Use replacement work, read the sole
  [active implementation plan](docs/handoffs/computer-use-rebuild.zh.md) and
  [authorized start prompt](docs/handoffs/computer-use-rebuild.start-prompt.zh.md).
  Continue from section 12 under the user's 2026-10-20 revision: complete
  Windows S1/S2/S3 first; run other platform builds and native validation in
  parallel. Missing machines, Wayland/DPI environments or model credentials
  remain pending and do not block local integration. Release validation applies
  to the platforms being released; never claim unverified support.
- Use the vendored Cua Driver and NomiFun private worker, with only necessary
  registered defect patches. Delete the replaced implementation as one complete
  integration; do not migrate or read retired Computer contracts/data, add
  compatibility paths, or alter canonical Agent Session storage. Preserve
  unrelated concurrent main-branch work and the actual HEAD contract version.
- Current-user desktop testing is authorized for dedicated test windows,
  sessions and temporary data. Leave real files, credentials, production data
  and signed-in browsers untouched. Prefer exact test-owned processes for fault
  cases; isolated accounts, displays or VMs are optional tools, not prerequisites.
  Keep automated, native and unexecuted evidence distinct, use targeted checks,
  and retire this scoped entry at S5 after documenting stable contracts.
