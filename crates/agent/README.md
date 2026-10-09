# crates/agent

Agent provider, configuration, and desktop automation crates. Package names use
the `nomi-*` prefix.

Current crates:

| Crate | Role |
| --- | --- |
| `nomi-types` | Provider-neutral model, message, and tool data types. |
| `nomi-compact` | Compaction-level configuration type used by `nomi-config`. |
| `nomi-config` | Provider, auth, hook, and runtime configuration. |
| `nomi-providers` | LLM provider clients and streaming logic. |
| `nomi-computer` | Desktop computer-use tool implementation. |
| `nomi-a11y` | Accessibility helpers used by computer-use flows. |
| `nomi-browser-engine` | Self-hosted browser/CDP automation engine. |

The model/tool loop lives in `nomifun-agent-runtime`. `nomifun-engine-core`
projects admitted tool plans into `nomifun-agent-kernel`; the application host
supplies capability adapters. MCP connections and invocation are owned by
`nomifun-mcp`.

## Boundary

- `crates/agent` must not depend on `nomifun-*` backend crates.
- Backend access to the agent layer should pass through
  `crates/backend/nomifun-ai-agent`.
- Shared utilities that genuinely belong on both sides live under
  `crates/shared`.
