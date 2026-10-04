# Agent engine

NomiFun installs one official `nomifun.nomi` Runtime factory. Chat, Coding, planning, tools, compaction and pause recovery share the execution loop in [nomifun-agent-runtime](../../crates/backend/nomifun-agent-runtime/src/lib.rs).

See [Agent Session architecture](agent-session.md) for the current session and content contract. Superseded engine, Wrapper and CLI implementation plans remain only in Git history.

## Execution path

```text
UI or domain command
  → canonical Session owner and Store
  → canonical Turn admission
  → official_runtime and EngineSessionHost
  → nomifun-agent-runtime
  → engine_journal
  → canonical events and Message projection
  → realtime and renderer
```

[official_runtime.rs](../../crates/backend/nomifun-app/src/router/official_runtime.rs) installs the factory. [unified_runtime_host.rs](../../crates/backend/nomifun-app/src/router/unified_runtime_host.rs) composes model, tool, resource, journal and cancellation ports. [nomifun-ai-agent](../../crates/backend/nomifun-ai-agent/src/lib.rs) manages product runtime handles and streaming.

## Primitives and ownership

Agent crates supply shared provider, message, compaction, MCP, Skill, Browser and Computer primitives. Platform domains execute Module Actions and interpret resource and effect results. Runtime does not own another Session store, effect ledger or product authority.

Immutable Agent revisions and Snapshots freeze the selected execution closure. Tool descriptions may be expanded on demand; the Kernel and frozen binding continue to govern authorization.

## History and recovery

Model context reads canonical events and content. UI projections cannot repair missing Runtime journals. Closed read-only history and executable checkpoints have distinct validation rules documented in the Session architecture.

Third-party terminal CLIs are PTY child processes. Remote `/mcp` and `/api/remote/*` are canonical Session ingress. Neither installs a second product Runtime.
