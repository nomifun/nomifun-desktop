# crates/agent

AI agent engine crates. Package names use the `nomi-*` prefix.

Current crates:

| Crate | Role |
| --- | --- |
| `nomi-types` | Provider-neutral data types and native tool effect categories. |
| `nomi-compact` | Conversation compaction and context shaping. |
| `nomi-config` | Provider, auth, hook, and runtime configuration. |
| `nomi-providers` | LLM provider clients and streaming logic. |
| `nomi-tools` | Built-in tool registry. |
| `nomi-mcp` | MCP client, config, transports, and tool proxying. |
| `nomi-computer` | Desktop computer-use tool implementation. |
| `nomi-a11y` | Accessibility helpers used by computer-use flows. |
| `nomi-browser-engine` | Self-hosted browser/CDP automation engine. |

## Boundary

- `crates/agent` must not depend on `nomifun-*` backend crates.
- Backend access to the agent layer should pass through
  `crates/backend/nomifun-ai-agent`.
- Shared utilities that genuinely belong on both sides live under
  `crates/shared`.
