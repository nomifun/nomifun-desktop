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
- Agent clean cut clears Agent data while preserving non-Agent configuration.
  Destructive-release startup rebuilds readable incompatible database lineages
  through the dataset coordinator without converting historical data. Database
  I/O failures and damage to a supported schema fail closed; backup and restore
  never obtain startup rebuild authority. More migration rows do not prove a
  newer application version across destructive baseline changes.
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

## Active Agent lifecycle reliability work

- For this lifecycle/recovery/control/resource-retirement implementation,
  first read the [active implementation plan](docs/handoffs/agent-lifecycle-reliability.zh.md).
  Its U01-U13 requirements and acceptance matrix are mandatory for this work;
  the plan records targets, not proof that code has already been delivered.
- Preserve existing capabilities and user concurrency settings. Do not add
  platform brokers, duplicate execution ledgers, fixed machine quotas, generic
  task TTLs, pressure-based task killing, mandatory worktrees or resource
  declarations. Supporting every pause/force-stop mode is not Agent admission.
- Every Agent instance needs host-side user stop and visible cleanup progress.
  Keep single-task, delegation ownership, DAG dependency and whole-Execution
  control distinct; do not implement local stop through aggregate cancellation.
  User Stop must preempt stalled controls; ordinary control FIFO must not delay
  its durable cancellation fence or signal to the exact owner.
- Cache code must not start, resume or retry tasks. Runtime retirement must
  preserve legitimate writebacks, exact process owners and necessary live state;
  neither an idle status nor a checkpoint proves safe release.
- Completed-task heavy resources must retire automatically without users closing
  old Sessions. Actual app quit must close app-owned heavy resources, with native
  parent-death coverage for crashes; preserve user-owned browsers/shared services
  and persistent data. Do not confuse one Turn ending with a user workspace
  becoming disposable, or use Session history as a permanent runtime reference.
- Continue execution from the single progress section in the active plan.
  Retire this scoped entry and superseded handoff instructions when the stable
  contracts have been integrated into current architecture documentation.
