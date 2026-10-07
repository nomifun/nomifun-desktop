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
- Agent clean cut clears Agent data while preserving non-Agent configuration;
  unknown or partial database lineage must fail closed without conversion.
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
