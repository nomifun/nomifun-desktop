# Backend Crates

The 54 `nomifun-*` crates under [`crates/backend/`](../../crates/backend/) form
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
`nomifun_ai_agent::{nomi_config, nomi_types}` when possible.

There are deliberate, feature-gated direct-dependency exceptions:

- Browser and Computer implementations are owned by the canonical
  `AgentPlatform` Role host in `nomifun-app`; the Platform Gateway does not
  depend on or register concrete desktop-control tools.

Do not add another direct `nomi-*` dependency without documenting why it cannot
go through the normal seam or one of those bridge surfaces.

## Core, data, realtime, runtime

### v3 data and identifier contract

Contributor changes must follow
[Data and Identifier Standards](../contributing/data-and-identifier-standards.md).
All backend crates follow the v3 data contract:

- every NomiFun product table has `id INTEGER PRIMARY KEY AUTOINCREMENT`,
  except the canonical Agent Store tables and the Unified Plugin tables, which
  use TEXT/composite keys as frozen in the baseline;
- stable cross-dataset entities use named bare canonical UUIDv7 fields such as
  `user_id`, `conversation_id`, and `message_id`;
- internal-only rows keep the table `id` as a repository implementation detail
  and use owner UUIDv7, sequence, natural, or composite keys for relations;
- one relationship has one reference field, with no `*_row_id` dual fields;
- repositories and services maintain indexed logical references; product DDL
  outside the Agent Store and Unified Plugin exception groups contains no
  physical `FOREIGN KEY`, `REFERENCES`, `ON DELETE`, or `ON UPDATE` clauses,
  beyond the registered guard triggers shipped in the baseline;
- v3 startup resets/quarantines an incompatible managed dataset as a whole
  instead of migrating historical rows.

The technical `id` is dataset-local and must not be treated as an API or
inter-table identity. Stable UUIDv7 fields are strings; external protocol
identifiers remain opaque.

| Crate | Responsibility |
| --- | --- |
| [`nomifun-common`](../../crates/backend/nomifun-common/) | `AppError`, error chain, enums (`AgentType`, `ConversationStatus`, `MessageType`, `McpServerStatus`, ...), bare UUIDv7 generation/validation for stable business IDs, dataset-reset helpers, AES-GCM `encrypt_string` / `decrypt_string`, `TimestampMs`, pagination helpers, `constants::DEFAULT_HOST/DEFAULT_PORT/BODY_LIMIT/CSRF_*`. |
| [`nomifun-api-types`](../../crates/backend/nomifun-api-types/) | Every HTTP request / response DTO, the `WebSocketMessage` envelope, and the Nomi build-extras. The frontend's TypeScript types mirror this crate. |
| [`nomifun-db`](../../crates/backend/nomifun-db/) | v3 SQLite baseline via `sqlx`, schema-contract and logical-reference registries, the canonical Agent Store schema, plus repository traits and Sqlite implementations for users, conversations, MCP, requirements, cron, agent presets, terminal sessions, the installation access token, webhooks, and more. Owns the `Database` handle and v3 baseline initialization. |
| [`nomifun-realtime`](../../crates/backend/nomifun-realtime/) | `WebSocketManager`, `BroadcastEventBus`, `/ws` upgrade handler with token validation, message router trait, heartbeat timing, per-connection buffer constants. |
| [`nomifun-runtime`](../../crates/backend/nomifun-runtime/) | Bundled Bun extraction, cache management, command discovery, and startup-time `PATH` enhancement. Child-process ownership lives in the shared `nomi-process-runtime` crate. |
| [`nomifun-assets`](../../crates/backend/nomifun-assets/) | Embedded static assets (`include_dir!`) shipped with the server. |

## Authentication and session

| Crate | Responsibility |
| --- | --- |
| [`nomifun-auth`](../../crates/backend/nomifun-auth/) | JWT HS256 (`JwtService`), bcrypt password hashing, login / logout / refresh / change-password / setup routes, `auth_middleware`, **CSRF double-submit cookie** middleware (cookie `nomifun-csrf-token`, header `x-csrf-token`), security-headers middleware, **rate limiting** (auth / api / authenticated-action variants), QR-code login token store, `validate_username` / `validate_password`. Exposes `CurrentUser` for handlers. |

## The agent seam

| Crate | Responsibility |
| --- | --- |
| [`nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent/) | **The single bridge to `crates/agent/`.** Builds the built-in `nomi` Agent runtime, while `AgentRuntimeSessions` manages process-local runtime handles. It broadcasts `AgentStreamEvent`, exposes `agent_routes` (model info, capabilities, slash commands, ...), and re-exports `nomi_config` and `nomi_types` for the rest of the backend. |

## Feature crates (the bulk of the product)

| Crate | Responsibility |
| --- | --- |
| [`nomifun-conversation`](../../crates/backend/nomifun-conversation/) | Canonical AgentSession product adapters and shared projection types: session-owner port (`CanonicalAgentSessionOwner`), creation ingress, Creative Studio canvas-agent-session binding, product-agent snapshot resolution, turn delivery, and the model-failover queue service. HTTP routes for sessions live in `nomifun-app`'s router. |
| [`nomifun-agent-execution`](../../crates/backend/nomifun-agent-execution/) | Persistent Agent collaboration: the `AgentExecutionEngine` facade owns planning, dependency scheduling, Attempts, recovery, decisions, events, and explicit Conversation links. Single- and multi-Agent work use this same aggregate; see the [unified execution architecture](agent-execution.zh.md). |
| [`nomifun-mcp`](../../crates/backend/nomifun-mcp/) | MCP server CRUD, **OAuth flow**, multi-CLI sync (`Claude`, `Codex`, `CodeBuddy`, `Gemini`, `Qwen`, `OpenCode`, `Nomi`, `Nomifun` adapters under `adapters/`), connection test, session injection of MCP capabilities (incl. built-in image-gen). |
| [`nomifun-skill-library`](../../crates/backend/nomifun-skill-library/) | Skill library product: built-in and user skills, market discovery, routes, startup materialization, and runtime skill resolution. |
| [`nomifun-channel`](../../crates/backend/nomifun-channel/) | External chat-channel adapters (Telegram, Lark, DingTalk, WeChat) — feature-gated. Maps inbound messages into the shared Agent / Conversation runtime, resolves per-bot or per-platform companion ownership, and applies channel Agent context. This is an integration boundary, not a separate Agent type or mode. |
| [`nomifun-gateway`](../../crates/backend/nomifun-gateway/) | **Platform Gateway MCP** — in-process capability registry and transport for `nomi_*` compatibility tools (conversations, cron, companion memory, requirements, and other domain services). Browser/Computer Role capabilities are owned by `AgentPlatform`. Internal child processes reach it through `nomicore mcp-gateway-stdio` with a server-derived, scoped, expiring signed claim; no Conversation or build-extra field grants access. Authenticated public fronts project only their allowed capability subset. |
| [`nomifun-cron`](../../crates/backend/nomifun-cron/) | Scheduled tasks: cron expressions, timezone repair, the cron daemon, slash-command-driven creation. |
| [`nomifun-requirement`](../../crates/backend/nomifun-requirement/) | **Persistent AutoWork runner** — backend-driven, boot-resume loop. `RequirementServiceSink` implements `nomifun_common::RequirementCreator` for the opt-in channel-to-requirement pipeline, wired by `nomifun-app`. |
| [`nomifun-idmm`](../../crates/backend/nomifun-idmm/) | Intelligent Decision-Making Mode: an opt-in canonical AgentSession supervisor for provider faults, model silence, and decision stalls (rule tier + constrained bypass model); it owns no second Session or Runtime. See [Intelligent Decision](../guides/intelligent-decision.md). |
| [`nomifun-webhook`](../../crates/backend/nomifun-webhook/) | Outbound Lark sender and `CompletionNotifier` for completed Agent work. |
| [`nomifun-agent-contracts`](../../crates/backend/nomifun-agent-contracts/) | Canonical machine contracts for the Agent platform: frozen types and deterministic artifacts only; no dependency on Conversation, app composition, or product state. |
| [`nomifun-agent-control-plane`](../../crates/backend/nomifun-agent-control-plane/) | AgentPreset, capability catalog, binding, and revision control plane. |
| [`nomifun-agent-kernel`](../../crates/backend/nomifun-agent-kernel/) | Thin Agent capability kernel and trusted in-process plugin host. |
| [`nomifun-agent-runtime`](../../crates/backend/nomifun-agent-runtime/) | Source-integrated execution loop for the unified Nomi Agent Runtime, composed through model/tool/event ports. |
| [`nomifun-agent-session`](../../crates/backend/nomifun-agent-session/) | Canonical AgentSession facts, projections, recovery, and deletion closure on the shared Agent Store root (Session, Turn, Event, Effect, Payload, Resource facts). |
| [`nomifun-agent-domain-support`](../../crates/backend/nomifun-agent-domain-support/) | Shared construction/validation helpers for the bundled domain-wave crates. |
| [`nomifun-agent-domain-wave1`](../../crates/backend/nomifun-agent-domain-wave1/) | Target-generation Web Research, Knowledge, and Memory module registrations. |
| [`nomifun-agent-domain-wave2`](../../crates/backend/nomifun-agent-domain-wave2/) | Bundled Wave 2 coding-extension capability registrations. |
| [`nomifun-agent-domain-wave3`](../../crates/backend/nomifun-agent-domain-wave3/) | Bundled creative and multimodal capability registrations (C7 Wave 3). |
| [`nomifun-agent-domain-wave4`](../../crates/backend/nomifun-agent-domain-wave4/) | Bundled identity/channel/device contribution registrations (C7 Wave 4). |
| [`nomifun-agent-domain-wave5`](../../crates/backend/nomifun-agent-domain-wave5/) | Bundled automation, supervision, and Remote domain registrations (C7 Wave 5). |
| [`nomifun-engine-core`](../../crates/backend/nomifun-engine-core/) | Ports shared by source-integrated execution engines; no strategy, Session database, plugin loader, or ambient authority. |
| [`nomifun-chat-model-broker`](../../crates/backend/nomifun-chat-model-broker/) | Provider-neutral Agent chat broker and stateless local Responses bridge; the only model retry/failover authority. |
| [`nomifun-voice`](../../crates/backend/nomifun-voice/) | Application voice owners; canonical work is injected through a port. |
| [`nomifun-voice-core`](../../crates/backend/nomifun-voice-core/) | Provider-independent voice media/session mechanics. |
| [`nomifun-voice-contracts`](../../crates/backend/nomifun-voice-contracts/) | Optional voice contracts; references Agent work identifiers without redefining them. |
| [`nomifun-companion`](../../crates/backend/nomifun-companion/) | Desktop companion state, figure/image assets, memory/persona data, companion public image serving, and robot/device binding integration. |
| [`nomifun-knowledge`](../../crates/backend/nomifun-knowledge/) | Knowledge bases, source ingestion, bound-base mount state, and scoped read-only knowledge MCP server. |
| [`nomifun-workshop`](../../crates/backend/nomifun-workshop/) | Canonical Creation domain: versioned project documents, assets, prompts, templates/runs, strict one-shot template drafts, Canvas ZIP archives, and most owner-only `/api/creative-studio/*` routes. Project documents, template state, and asset metadata live in SQLite; binary originals/thumbnails live under `{data_dir}/workshop/assets/`. The only public surface is read-only `GET /api/creative-studio/files/{asset_id}` for browser media elements. |
| [`nomifun-plugin-platform`](../../crates/backend/nomifun-plugin-platform/) | Unified Plugin Core: one Manifest/Package/Plugin identity, content-addressed Artifacts, Drafts, generation DataRoots, Action/Binding registry, isolated Service processes, Surface Bridge, Package/Backup transfer, rollback, and the single `/api/plugins` + `/api/plugin-drafts` HTTP surface. |
| [`nomifun-plugin-development`](../../crates/backend/nomifun-plugin-development/) | Optional Agent module for developing one Unified Plugin through ordinary conversation tools; Plugin instances and runtime remain owned by Plugin Core. |
| [`nomifun-js-runtime`](../../crates/backend/nomifun-js-runtime/) | One immutable, process-wide JavaScript runtime authority for the Unified Plugin Core. |
| [`nomifun-robot`](../../crates/backend/nomifun-robot/) | Robot gateway: LAN-attached physical robots (Xiaozhi firmware) acting as the physical embodiment of a desktop companion. |
| [`nomifun-creation`](../../crates/backend/nomifun-creation/) | Media-generation engine behind Creation canvas nodes and template steps. Owns the canonical owner-only `/api/creative-studio/tasks*` queue (`queued → running → succeeded/failed/canceled`), exact provider/model/task/input identity, per-provider and global concurrency, cancellation, and boot reconciliation. Delegates model execution to `nomifun-model-invoke` and hands produced bytes to the Workshop `AssetSink`. |
| [`nomifun-customer-service`](../../crates/backend/nomifun-customer-service/) | Standalone customer-service domain for serving strangers over IM channels. Shares no concepts with the companion/conversation system: dialogues are the domain's own aggregate and replies come from a disposable one-shot engine session with a fixed read-only tool registry. |
| [`nomifun-public`](../../crates/backend/nomifun-public/) | Installation-token authenticated canonical Remote MCP adapter at `/mcp`, exposing only `open/turn/observe/cancel` over `AgentPlatform`/`AgentSession`. |
## Infrastructure features

| Crate | Responsibility |
| --- | --- |
| [`nomifun-terminal`](../../crates/backend/nomifun-terminal/) | Terminal sessions backed by `portable-pty`, resize, input/output streaming over WS. |
| [`nomifun-ssh`](../../crates/backend/nomifun-ssh/) | SSH remote sessions: the encrypted, owner-scoped host book (`ssh_hosts`), the connection pool/provider, `/api/ssh-hosts` routes, and the `SshBackend` sink that gives an SSH-bound conversation's agent a remote tool family. Transport lives in the isolated shared `nomi-ssh` crate (`russh`/`russh-sftp`). |
| [`nomifun-browser-platform`](../../crates/backend/nomifun-browser-platform/) | Typed contracts for the conversation-owned native Browser Workspace: run/input ownership, tab and runtime generations, snapshots, uploads/downloads, profile cleanup, and the attached-Chrome provider grant model. The desktop host supplies the native WebView; isolated search/render and an attached personal browser remain distinct consumers. |
| [`nomifun-browser-macos`](../../crates/backend/nomifun-browser-macos/) | macOS 14+ system WKWebView adapter for the Browser Workspace; Windows retains its independent WebView2 host. |
| [`nomifun-model-invoke`](../../crates/backend/nomifun-model-invoke/) | Unified multimodal model invocation layer: typed task requests/results, declarative auth schemes, shared HTTP transport, the protocol-adapter seam + registry, and catalog resolution. Consumed by `nomifun-shell` STT/TTS, `nomifun-creation`, and other model-calling features. |
| [`nomifun-shell`](../../crates/backend/nomifun-shell/) | OS shell helpers: open files in the system, speech-to-text against Deepgram or OpenAI, clipboard / paste integration. |
| [`nomifun-file`](../../crates/backend/nomifun-file/) | Sandboxed filesystem under the conversation work dir (`browse`, `path_safety`, `watch_service`, `snapshot_service`), zip helpers. |
| [`nomifun-office`](../../crates/backend/nomifun-office/) | LibreOffice convert/preview pipeline (Office documents → preview). |
| [`nomifun-system`](../../crates/backend/nomifun-system/) | LLM provider / model lookup, app-level settings, sysinfo, app version-check / self-updater scaffold. |

## The composition root: `nomifun-app`

[`nomifun-app`](../../crates/backend/nomifun-app/) is what the two host binaries
link. It is structured as:

| Module | Role |
| --- | --- |
| `cli.rs` | Top-level `nomicore` clap parser: `--host/--port/--data-dir/--work-dir/--app-version/--local/--log-dir/--log-level` plus subcommands `mcp-requirement-stdio`, `mcp-knowledge-stdio`, `mcp-gateway-stdio`, `mcp-open-stdio`, `terminal-hook`, `doctor`, `remote` (`open`/`turn`/`observe`/`cancel`), `backup`, and `restore`. The web host calls `Cli::parse_from(["nomifun-web"])` to get a defaulted instance, then overrides what it owns. |
| `bootstrap/` | Layered initialization: `tracing_init` (file + console layers), `work_dir` resolution, `builtin_skills` materialization, `environment::{init_environment,init_data_layer}`, `admin::ensure_admin_credentials` for first-run pre-seed in authenticated mode. |
| `services.rs` | The `AppServices` god-bag: every feature-crate service wired together with the right repositories. Built once via `AppServices::from_config(database, &config)`. |
| `router/` | `create_router(&services)` and the typed `routes`, `state`, `health`, `trace` helpers; `build_module_states` / `build_skill_state` / `build_ws_state`. |
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
