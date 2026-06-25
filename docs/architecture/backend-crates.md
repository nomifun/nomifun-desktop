# Backend Crates

The 29 `nomifun-*` crates under [`crates/backend/`](../../crates/backend/) form
the HTTP/WS server. Together they compile into the `nomifun-app` library crate
and, via `nomifun-app/src/main.rs`, the **`nomicore`** binary. The two app hosts
(`nomifun-desktop` and `nomifun-web`) link `nomifun-app` directly and call
`run_embedded_server` or compose `create_router` themselves.

The grouping below mirrors how the crates depend on each other in the workspace
manifest ([`Cargo.toml`](../../Cargo.toml)). It is not a strict layered DAG —
some feature crates depend on each other — but it gives a cognitive map that
lines up with how a request travels through the server.

## Agent-layer dependency rule

The normal product seam is
[`nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent/). Feature crates
that need agent concepts should consume them through
`nomifun_ai_agent::{nomi_config, nomi_types, RequirementSink}` when possible.

There are deliberate, feature-gated direct-dependency exceptions:

- [`nomifun-app`](../../crates/backend/nomifun-app/) depends on optional
  `nomi-computer`, `nomi-browser`, `nomi-config`, `nomi-tools`, and
  `nomi-types` for the `mcp-computer-stdio` and `mcp-browser-stdio` bridge
  subcommands.
- [`nomifun-gateway`](../../crates/backend/nomifun-gateway/) depends on optional
  `nomi-browser`, `nomi-computer`, `nomi-config`, `nomi-tools`, and
  `nomi-types` for the Desktop Gateway browser/computer registries.

Do not add another direct `nomi-*` dependency without documenting why it cannot
go through the normal seam or one of those bridge surfaces.

## Core, data, realtime, runtime

| Crate | Responsibility |
| --- | --- |
| [`nomifun-common`](../../crates/backend/nomifun-common/) | `AppError`, error chain, enums (`AgentType`, `ConversationStatus`, `MessageType`, `McpServerStatus`, ...), id generation (`generate_prefixed_id` for entity IDs, `generate_id` for tokens), AES-GCM `encrypt_string` / `decrypt_string`, `TimestampMs`, pagination helpers, `constants::DEFAULT_HOST/DEFAULT_PORT/BODY_LIMIT/CSRF_*`. |
| [`nomifun-api-types`](../../crates/backend/nomifun-api-types/) | Every HTTP request / response DTO, the `WebSocketMessage` envelope, ACP / Nomi / OpenClaw / Remote build-extras. The frontend's TypeScript types mirror this crate. |
| [`nomifun-db`](../../crates/backend/nomifun-db/) | SQLite via `sqlx`, embedded migrations, repository traits and Sqlite implementations for users, conversations, MCP, requirements, cron, ACP sessions, assistants, terminal sessions, companion tokens, webhooks, and more. Owns the `Database` handle and `init_database`. |
| [`nomifun-realtime`](../../crates/backend/nomifun-realtime/) | `WebSocketManager`, `BroadcastEventBus`, `/ws` upgrade handler with token validation, message router trait, heartbeat timing, per-connection buffer constants. |
| [`nomifun-runtime`](../../crates/backend/nomifun-runtime/) | Bundled runtime support for Bun, PATH enhancement for child processes, cross-platform process-tree kill, and a spawn `Builder` with the merged PATH. |
| [`nomifun-assets`](../../crates/backend/nomifun-assets/) | Embedded static assets (`include_dir!`) shipped with the server. |

## Authentication and session

| Crate | Responsibility |
| --- | --- |
| [`nomifun-auth`](../../crates/backend/nomifun-auth/) | JWT HS256 (`JwtService`), bcrypt password hashing, login / logout / refresh / change-password / setup routes, `auth_middleware`, **CSRF double-submit cookie** middleware (cookie `nomifun-csrf-token`, header `x-csrf-token`), security-headers middleware, **rate limiting** (auth / api / authenticated-action variants), QR-code login token store, `validate_username` / `validate_password`. Exposes `CurrentUser` for handlers. |

## The agent seam

| Crate | Responsibility |
| --- | --- |
| [`nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent/) | **The single bridge to `crates/agent/`.** Builds the agent factory (ACP / Nomi / OpenClaw / Nanobot / Remote variants), holds the `AgentRegistry` and `WorkerTaskManagerImpl`, persists ACP sessions, broadcasts `AgentStreamEvent`, exposes `agent_routes` (model info, capabilities, slash commands, ...) and `remote_agent_routes`. Re-exports `nomi_config`, `nomi_types`, and `RequirementSink` for the rest of the backend. |

## Feature crates (the bulk of the product)

| Crate | Responsibility |
| --- | --- |
| [`nomifun-conversation`](../../crates/backend/nomifun-conversation/) | Conversation and message CRUD, send-message route, **streaming relay** that fans backend agent tokens onto `/ws`, ACP error recovery, response middleware (e.g. `/cron` slash-command detection, `<think>` stripping), skill resolver / snapshot, runtime-state persistence. |
| [`nomifun-mcp`](../../crates/backend/nomifun-mcp/) | MCP server CRUD, **OAuth flow**, multi-CLI sync (`Claude`, `Codex`, `CodeBuddy`, `Gemini`, `Qwen`, `OpenCode`, `Nomi`, `Nomifun` adapters under `adapters/`), connection test, session injection of MCP capabilities (incl. built-in image-gen). |
| [`nomifun-extension`](../../crates/backend/nomifun-extension/) | Extension and skill hub: manifests, dependency graph, classifier, install / enable / disable, packs that bundle skills + MCP servers + assistants. |
| [`nomifun-team`](../../crates/backend/nomifun-team/) | Multi-agent teams: scheduler, mailbox, task board, crash detection, event loop, the team-MCP server (`mcp/`), the Guide MCP `nomi_create_team` tool, prompts. |
| [`nomifun-channel`](../../crates/backend/nomifun-channel/) | External chat-channel adapters (Telegram, Lark, DingTalk, WeChat) — feature-gated. New conversations default to **master-agent mode**: companion persona + the Desktop Gateway tools (opt-out per platform via `assistant.{platform}.masterAgent`). |
| [`nomifun-gateway`](../../crates/backend/nomifun-gateway/) | **Desktop Gateway MCP** — in-process HTTP tool server exposing the whole desktop (conversations, cron, companion memory, requirements, and feature-gated browser/computer tools) as `nomi_*` tools to internal and external agent surfaces. Reached internally via the `nomicore mcp-gateway-stdio` bridge. |
| [`nomifun-cron`](../../crates/backend/nomifun-cron/) | Scheduled tasks: cron expressions, timezone repair, the cron daemon, slash-command-driven creation. |
| [`nomifun-requirement`](../../crates/backend/nomifun-requirement/) | **AutoWork orchestrator** — backend-driven, boot-resume, persistent loop. Speaks to the agent layer through `RequirementSink`. |
| [`nomifun-idmm`](../../crates/backend/nomifun-idmm/) | Intelligent Decision-Making Mode: a per-session supervisor that keeps agent / terminal sessions alive through provider faults and decision stalls (rule tier + sidecar model). See [Intelligent Decision](../guides/intelligent-decision.md). |
| [`nomifun-webhook`](../../crates/backend/nomifun-webhook/) | Outbound Lark sender, `CompletionNotifier` for finished agent runs. |
| [`nomifun-assistant`](../../crates/backend/nomifun-assistant/) | Assistant (preset prompt + skill set + MCP set) CRUD, override resolution, import/export. |
| [`nomifun-companion`](../../crates/backend/nomifun-companion/) | Desktop companion state, figure/image assets, memory/persona data, companion public image serving, and companion-bound token integration. |
| [`nomifun-knowledge`](../../crates/backend/nomifun-knowledge/) | Knowledge bases, source ingestion, bound-base mount state, and scoped read-only knowledge MCP server. |
| [`nomifun-public`](../../crates/backend/nomifun-public/) | Companion-token authenticated public front doors: `/mcp`, `/mcp-agent`, and `/v1`. |
| [`nomifun-secret`](../../crates/backend/nomifun-secret/) | Per-companion browser-use secret storage and credential lookup. |

## Infrastructure features

| Crate | Responsibility |
| --- | --- |
| [`nomifun-terminal`](../../crates/backend/nomifun-terminal/) | Terminal sessions backed by `portable-pty`, resize, input/output streaming over WS. |
| [`nomifun-shell`](../../crates/backend/nomifun-shell/) | OS shell helpers: open files in the system, speech-to-text against Deepgram or OpenAI, clipboard / paste integration. |
| [`nomifun-file`](../../crates/backend/nomifun-file/) | Sandboxed filesystem under the conversation work dir (`browse`, `path_safety`, `watch_service`, `snapshot_service`), zip helpers. |
| [`nomifun-office`](../../crates/backend/nomifun-office/) | LibreOffice convert/preview pipeline (Office documents → preview). |
| [`nomifun-system`](../../crates/backend/nomifun-system/) | LLM provider / model lookup, app-level settings, sysinfo, app version-check / self-updater scaffold. |

## The composition root: `nomifun-app`

[`nomifun-app`](../../crates/backend/nomifun-app/) is what the two host binaries
link. It is structured as:

| Module | Role |
| --- | --- |
| `cli.rs` | Top-level `nomicore` clap parser: `--host/--port/--data-dir/--work-dir/--app-version/--local/--log-dir/--log-level` plus subcommands `mcp-requirement-stdio`, `mcp-knowledge-stdio`, `mcp-gateway-stdio`, `mcp-open-stdio`, `mcp-computer-stdio`, `mcp-browser-stdio`, `terminal-hook`, `doctor`, `tools`, `call`, and `agent`. The web host calls `Cli::parse_from(["nomifun-web"])` to get a defaulted instance, then overrides what it owns. |
| `bootstrap/` | Layered initialization: `tracing_init` (file + console layers), `work_dir` resolution, `builtin_skills` materialization, `environment::{init_environment,init_data_layer}`, `admin::ensure_admin_credentials` for first-run pre-seed in authenticated mode. |
| `services.rs` | The `AppServices` god-bag: every feature-crate service wired together with the right repositories. Built once via `AppServices::from_config(database, &config)`. |
| `router/` | `create_router(&services)` and the typed `routes`, `state`, `health`, `trace` helpers; `build_assistant_state` / `build_conversation_state` / `build_extension_states` / `build_module_states` / `build_ws_state`. |
| `commands/` | CLI subcommand bodies for the server, current stdio MCP bridges, terminal lifecycle hook, diagnostics, and public capability client commands. |
| `lib.rs` | Public façade: `run_embedded_server`, `AppServices`, `create_router`, `bootstrap` re-exports. This is the only API the host binaries import. |

## Checking direct agent dependencies

If you want to inspect direct `nomi-*` dependencies, scan every backend crate
manifest:

```sh
# from the repo root, on a Unix shell
rg -l 'nomi-[a-z-]+\\s*=' crates/backend/*/Cargo.toml
```

Expect the primary seam (`nomifun-ai-agent`) plus the feature-gated bridge
exceptions described above.
