# crates/backend

Backend crates. Package names use the `nomifun-*` prefix. Together these crates
provide the HTTP/WS server, data layer, auth, conversations, MCP/skills,
knowledge, requirements/AutoWork, terminal sessions, companions, public
capability gateway, and app composition.

The current backend group contains 29 crates. The most important entry points
are:

| Crate | Role |
| --- | --- |
| `nomifun-app` | Composition root, CLI, bootstrap, service graph, router assembly, and embedded-server helpers. |
| `nomifun-db` | SQLite, migrations, repository traits, and repository implementations. |
| `nomifun-api-types` | Shared HTTP/WS request and response types. |
| `nomifun-auth` | JWT, local trust, CSRF, auth routes, QR login, and security middleware. |
| `nomifun-conversation` | Conversation/message service and agent stream relay. |
| `nomifun-ai-agent` | Single bridge into `crates/agent`; agent factory, registry, ACP/session management, worker tasks. |
| `nomifun-mcp` | MCP server config, OAuth, adapters, sync, and connection tests. |
| `nomifun-extension` | Extension, skill, assistant contribution, and hub plumbing. |
| `nomifun-requirement` | Requirements Platform and AutoWork orchestration. |
| `nomifun-terminal` | PTY-backed terminal sessions. |
| `nomifun-knowledge` | Knowledge bases and scoped knowledge MCP server. |
| `nomifun-companion` | Desktop companions and companion memory/persona state. |
| `nomifun-gateway` | Desktop Gateway MCP tools exposed to internal and external agents. |
| `nomifun-public` | Companion-token authenticated `/mcp`, `/mcp-agent`, and `/v1` public front doors. |

See `docs/architecture/backend-crates.md` for the maintained map.

## Agent Boundary

Only `nomifun-ai-agent` should depend directly on `nomi-*` crates. Other backend
crates consume agent-facing types through its re-exports, for example
`nomifun_ai_agent::{nomi_config, nomi_types, RequirementSink}`.

This keeps the agent layer isolated enough to reason about, but the older
`nomifun-agent-rs` extraction language in historical specs is not a current
roadmap commitment.
