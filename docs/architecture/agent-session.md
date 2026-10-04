# Agent Session logs and content architecture

AgentSession is the sole fact chain for sessions, turns, content, effects and cancellation. User-facing Conversation APIs project the same `AgentSessionId`. Chat and Coding run through one in-process `nomifun.nomi` Runtime.

This is the current development entry point. Rust types, the Session event registry and the canonical schema own executable contracts. Superseded designs and implementation instructions remain only in Git history.

## Facts and derived data

| Data | Responsibility |
| --- | --- |
| Sessions and binding transitions | Identity, immutable Agent Revision and Snapshot, resources and explicit Agent switches |
| Turns | Accepted input, admission, idempotency, lifecycle and terminal receipts |
| Events and payloads | Ordered semantic facts and their content; the source for context and presentation |
| Effects | One ledger; domain owners interpret effect results |
| Message projections | UI queries; never supply missing Runtime records or infer completion |
| Native checkpoints | Recovery bound to exact build, binding and cursor; not another transcript |
| AgentExecution and domains | Their own business facts, referencing the same canonical Session and Turn |

## Runtime context

`engine_history.rs` reads typed canonical events and resolved payloads. It does not read Message projections to determine model roles. Native closed turns replay their structured journal. Missing Runtime records fail closed rather than falling back to chat text.

An accepted input that fails or is cancelled before Runtime startup remains valid context together with its explicit terminal fact. It does not acquire fabricated Runtime, tool or completion evidence.

Creation prompts, Cron notices and Execution summaries enter context through formal message events. Text before an explicit Agent transition is data only; it does not carry old grants, handles or completion gates. Context-clear events and the fixed accepted-root cursor bound the window.

Current-generation history, immutable Snapshots, model changes and explicit Agent transitions remain supported product semantics. They are verified against identity, sequence, binding and content integrity. Executable recovery requires the native checkpoint's exact build and binding. Closed read-only event history may cross an explicitly validated transition without gaining authority.

Session reasoning uses one native `reasoning_effort` field defined by the shared contract. No lossy mirrors or fallback columns are maintained.

## Consumers and deletion

UI consumes current stream and Message projections directly. It has no marker-dependent local terminal-processing state machine or prose-based reclassification of old errors. Channel binds through its current owner; orphan rows without an authority binding produce a conflict instead of being automatically rebound.

Cron, Companion, Requirements, AutoWork, IDMM and AgentExecution retain their own business configuration and supervision state. Session input, admission, cancellation and completion use canonical Turn receipts.

The refactor uses an Agent-only clean cut in the same database. Retired Agent sessions, messages, logs, Execution, presets and bindings are neither imported nor read. Non-Agent configuration follows its explicit owner and cutover contract. Repository cleanup must not operate on a developer's or user's live data directory.

Replacement includes physical deletion of the old reader, writer, DTO, schema fields, fixtures, tests and documents. Published migration lineage needs its own deliberate boundary; checksum rewriting and permanent dual writes are not substitutes for deletion. Unused generic checkpoint APIs, historical deletion manifests and generator requirements for obsolete documents are removed.

## Sources and validation

- [Session owner](../../crates/backend/nomifun-conversation/src/canonical_session_owner.rs) and [Store](../../crates/backend/nomifun-agent-session/src/store.rs)
- [Official Runtime](../../crates/backend/nomifun-app/src/router/official_runtime.rs) and [Runtime implementation](../../crates/backend/nomifun-agent-runtime/src/lib.rs)
- [Journal](../../crates/backend/nomifun-app/src/router/engine_journal.rs), [canonical history](../../crates/backend/nomifun-app/src/router/engine_history.rs) and [native replay](../../crates/backend/nomifun-app/src/router/unified_runtime_history.rs)
- [Contracts](../../crates/backend/nomifun-agent-contracts/src/lib.rs) and [schema](../../crates/backend/nomifun-db/migrations/001_canonical_baseline.sql)

Run targeted Rust/UI tests, contract generator check, `bun run check:agent-session-boundary` and `bun run check:uarc-boundary`. Renderer changes also require `bun run check:desktop-ui-boundary`; the minimum supported desktop viewport is 880×600.

Cover new and sequential turns, rejected admission followed by a new turn, Agent switches, context clear, cancellation, restart, pause recovery, unknown effects and deletion. Missing structured history must not be repaired by a projection fallback.
