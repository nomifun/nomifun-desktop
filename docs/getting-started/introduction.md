# Introduction

**NomiFun** is an open-source AI workstation and coding workspace. It unifies
multiple AI runtimes, LLM providers, MCP servers, skills, terminals, knowledge
bases, scheduled work, and companion/remote capability surfaces in one local-first
application.

> Ready to run it? Start with [Installation](installation.md), then
> [Quick Start](quick-start.md). For the full documentation map, see
> [docs/README.md](../README.md).

![NomiFun guide / landing page](../images/gs-01-introduction-hero.png)

## What NomiFun Solves

Modern AI workflows are scattered across separate CLIs, terminals, browser
tabs, MCP servers, and local scripts. NomiFun pulls them into one workspace:

- **Many agents, one surface.** Use the built-in Nomi engine or external
  ACP-style CLIs such as Claude Code, Codex, Gemini CLI, Qwen, and OpenCode.
- **One workspace per conversation.** Conversations can own files, previews,
  diffs, terminals, and knowledge bindings instead of living as isolated chat
  transcripts.
- **Backend-driven automation.** Scheduled tasks, AutoWork requirements,
  terminal sessions, channel integrations, and completion notifications are
  durable backend services, not foreground browser-tab state.
- **Extensible capability layer.** MCP servers, skills, assistants, browser use,
  computer use, and public remote capability fronts can be composed per runtime.
- **Local-first deployment.** Run it as a Tauri desktop app or a self-hosted web
  server. You provide the model/API credentials and decide where the data lives.

NomiFun is not a no-code SaaS chat product. It is infrastructure for users who
are comfortable configuring agents, providers, local tools, and self-hosted
services.

## Two Hosts, One Backend

Both hosts run the same Rust backend (`nomifun-app`) in-process and load the
same React SPA (`ui/dist`).

| Mode | Binary | Auth model | Typical use |
| --- | --- | --- | --- |
| Desktop app | `nomifun-desktop` | Per-boot local trust token injected into the desktop webview | Personal workstation |
| Web server | `nomifun-web` | Login required by default; first-run setup or pre-seeded admin | Browser / LAN / server deployment |

```text
nomifun-desktop
  Tauri shell -> embedded backend on 127.0.0.1:<ephemeral> -> same SPA

nomifun-web
  axum server -> /api + /ws + static ui/dist on one port (default 8787)
```

For implementation details, see [Architecture Overview](../architecture/overview.md).

## Main Surfaces

- **Home & conversations** (`/guid`): start and continue AI sessions.
- **Terminals**: PTY-backed agent or shell sessions inside the app.
- **Models**: providers, local agent detection, global IDMM/failover settings.
- **Assistant & Skill**: assistant personas and skill management.
- **MCP**: local MCP server configuration.
- **Open Capabilities**: WebUI remote access, remote MCP, and REST capability
  exposure.
- **Requirements / AutoWork**: backend-owned queue processing and completion
  notifications.
- **Scheduled tasks**: recurring or one-shot jobs.
- **Desktop Companion** (`/nomi`): companion configuration, memory, and remote binding.
- **Knowledge**: local knowledge-base management and session bindings.

The current frontend route source is
`ui/src/renderer/components/layout/Router.tsx`.

## Project Status

NomiFun is in active development. [STATUS.md](../../STATUS.md) is the compact
current-state snapshot. Design and audit history is not kept in the repo; consult
git history for past decisions.

## Acknowledgments

NomiFun began as a fork of the open-source
[AionUi](https://github.com/iOfficeAI/AionUi) project and has since been
substantially refactored around a Tauri + Rust architecture. NomiFun is released
under the Apache-2.0 License.
