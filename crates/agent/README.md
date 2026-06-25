# crates/agent

AI agent engine crates. Package names use the `nomi-*` prefix.

Current crates:

| Crate | Role |
| --- | --- |
| `nomi-types` | Provider-neutral data types. |
| `nomi-protocol` | Host/agent command and event protocol. |
| `nomi-compact` | Conversation compaction and context shaping. |
| `nomi-config` | Provider, auth, hook, and runtime configuration. |
| `nomi-providers` | LLM provider clients and streaming logic. |
| `nomi-tools` | Built-in tool registry. |
| `nomi-mcp` | MCP client, config, transports, and tool proxying. |
| `nomi-skills` | Skill discovery, loading, and execution support. |
| `nomi-memory` | Long-term project/user memory. |
| `nomi-agent` | Core session engine and orchestration. |
| `nomi-cli` | Standalone `nomi` CLI. |
| `nomi-computer` | Desktop computer-use tool implementation. |
| `nomi-a11y` | Accessibility helpers used by computer-use flows. |
| `nomi-browser-engine` | Self-hosted browser/CDP automation engine. |
| `nomi-browser` | Browser-use tool layer. |

## Boundary

- `crates/agent` must not depend on `nomifun-*` backend crates.
- Backend access to the agent layer should pass through
  `crates/backend/nomifun-ai-agent`.
- Shared utilities that genuinely belong on both sides live under
  `crates/shared`.

The old extraction checklist in `docs/specs/agent-extraction-checklist.md` is a
historical aid. Re-check it against the current crate list before using it as an
execution plan.
