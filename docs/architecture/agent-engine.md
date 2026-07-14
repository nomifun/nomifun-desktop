# Agent Engine

The agent engine lives under [`crates/agent/`](../../crates/agent/) and is
consumed by the backend primarily through
[`nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent/). This page is an
implementation map for the current workspace, not an extraction plan.

## Crate Map

| Crate | Responsibility |
| --- | --- |
| `nomi-types` | Provider-neutral messages, tool types, compaction types, file state, skill types, plus the Agent task, tool-policy, and one-invocation primitives shared by local and persistent collaboration. |
| `nomi-protocol` | Host/agent command and event protocol plus approval state. |
| `nomi-compact` | Context compaction and message-window shaping. |
| `nomi-config` | Runtime/provider/profile/auth configuration. |
| `nomi-providers` | Anthropic, OpenAI-compatible, Bedrock, Vertex, and shared streaming/retry/provider logic. |
| `nomi-tools` | Built-in tools and tool registry primitives. |
| `nomi-mcp` | MCP client, manager, transports, and tool proxying. |
| `nomi-skills` | Skill discovery, frontmatter, loading, and skill-index support. |
| `nomi-memory` | Memory storage and retrieval primitives. |
| `nomi-agent` | Core engine loop, sessions, compaction glue, confirmations, output sinks, skill tool, requirement tools, and the crate-private embedded AgentExecution projection. |
| `nomi-cli` | Standalone `nomi` CLI consumer of the engine. |
| `nomi-computer` | Desktop computer-use tool implementation. |
| `nomi-a11y` | Accessibility helpers for computer-use flows. |
| `nomi-browser-engine` | Self-hosted browser/CDP automation engine. |
| `nomi-browser` | Browser-use tool facade. |

`nomi_delegate` has one request and receipt contract in `nomi-types`:
`ParallelDelegationRequest`, `AgentExecutionReceipt`, and
`AgentExecutionStatus`. A platform deployment persists the aggregate and may
return an active status while the scheduler continues asynchronously. An
embedded CLI deployment runs the same Agent invocations in the current Turn and
returns a terminal projection (`completed`, `completed_with_failures`, or
`failed`) with typed results. This deployment choice is private host
composition, not a user setting, model argument, product mode, or second state
machine. Fork-mode skills reuse the same `AgentInvocationRunner` primitive.

For multi-Agent embedded work, the host maintains a private progress ledger and
injects only a bounded, JSON-encoded sibling assignment/status snapshot through
`ContextContributor`. The block is explicitly marked as untrusted data and
cannot grant authority. There is no model-visible task-board tool. Workspace
placement is derived from the effective inherited tool scope and the same
read/mutation effect catalog used to build the child registry. Zero or one
mutation-capable sibling keeps direct writes; with two or more, only writers use
private worktrees from one stable, self-contained source snapshot while readers
continue to share the source workspace. A non-Git fallback is explicit in each
affected result. Parent raw-shell hooks are intentionally not inherited: they
were an authority bypass for read-only and synthesis Agents. Any future child
hook support must run through the same process capability and effect boundary.

The agent crates do not depend on `nomifun-*` backend crates. Backend-to-agent
integration normally flows through `nomifun-ai-agent`; feature-gated bridge
surfaces in `nomifun-app` and `nomifun-gateway` directly depend on browser and
computer-use crates to expose those capabilities as stdio/public tools.

## Runtime Families

NomiFun supports several runtime families:

- **Nomi engine**: in-tree engine from `nomi-agent`, with providers, built-in
  tools, skills, MCP, memory, browser, and computer-use support.
- **ACP-style CLI agents**: Claude Code, Codex, Gemini CLI, Qwen/OpenCode-style
  integrations, and related CLIs managed by `nomifun-ai-agent`.
- **Remote/Open capability surfaces**: external agents connect through
  companion-token authenticated `/mcp`, `/mcp-agent`, or `/v1` fronts.

The implementation source of truth for factory behavior is:

- `crates/backend/nomifun-ai-agent/src/factory/nomi.rs`
- `crates/backend/nomifun-ai-agent/src/factory/acp.rs`
- `crates/backend/nomifun-ai-agent/src/factory/acp_assembler.rs`

## MCP And Tool Injection

MCP/tool availability is assembled per runtime and per session. It is not a
single flat list.

Common sources include:

- user-configured MCP server rows from `nomifun-mcp`,
- requirement declaration tools when AutoWork requires them,
- scoped knowledge search when a session has mounted knowledge bases,
- platform Gateway tools when the factory derives instance-owner authority,
- Windows/open helper bridge,
- feature-gated computer-use and browser-use stdio bridges,
- runtime-native skills or first-message skill injection,
- Nomi's native tool registry.

The platform Gateway is an internal capability transport, not a Conversation
setting or persisted grant. The server derives authority from the authenticated
principal. When an Agent runs in a child process, the parent issues only a
scoped, expiring access claim plus a renewal proof bound to the same immutable
authorization. Renewal is backed by a revocable process-local lease, so a
long-lived or sleep-resumed child can refresh access without receiving the
signing root or widening scope. The root and lease registry remain
process-private and are never stored in build-extra, Conversation or database
rows; runtime teardown and process restart revoke them. Public and non-owner
contexts fail closed and receive no host capability.

When documenting tool availability, cite the factory files above rather than
assuming all agents receive the same injected servers.

## Skills

Skills are instruction/tool bundles whose materialization depends on runtime
capability:

- Nomi has a real `Skill` tool path in the engine.
- Native CLI runtimes may receive symlinked/copied skill files or lightweight
  first-message guidance when the runtime supports it.
- Custom workspace or non-native paths can be summarized in a first-message
  skill index.

Relevant source files:

- `crates/backend/nomifun-extension/src/skill_service.rs`
- `crates/backend/nomifun-ai-agent/src/capability/skill_manager/mod.rs`
- `crates/backend/nomifun-ai-agent/src/capability/first_message_injector.rs`
- `crates/agent/nomi-agent/src/skill_tool.rs`

## Session Flow

```text
UI request
  -> nomifun-conversation route/service
  -> nomifun-ai-agent AgentService / AgentRuntimeRegistry
  -> runtime family factory
  -> Nomi engine or external CLI process
  -> AgentStreamEvent
  -> nomifun-realtime /ws
  -> renderer stream handlers
```

Nomi-engine sessions run inside the process. ACP-style sessions spawn and manage
child CLIs. Public remote capability calls enter through `nomifun-public` and
the platform Gateway registry rather than the conversation HTTP route.

## Design Notes

Older specs describe the agent layer as mechanically extraction-ready and list
only 11 crates. Those files are historical. The current code still keeps a
strong boundary, but browser/computer bridge work and public gateway surfaces
mean the real rule is “primary seam plus documented feature-gated exceptions.”
