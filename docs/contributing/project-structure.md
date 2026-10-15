# Project Structure

[简体中文](project-structure.zh.md)

This page locates source owners. Detailed contracts live in
[architecture](../architecture/overview.md); workspace membership is declared in
[Cargo.toml](../../Cargo.toml) and [package.json](../../package.json).

| Path | Responsibility |
| --- | --- |
| [apps/desktop](../../apps/desktop) | Tauri shell, native integrations, embedded backend and packaging configuration. |
| [apps/web](../../apps/web) | Standalone `nomifun-web` host serving API, WebSocket and SPA on one port. |
| [crates/backend](../../crates/backend) | Application and feature crates; see [backend ownership](../architecture/backend-crates.md). |
| [crates/agent](../../crates/agent) | Provider-neutral types, provider clients, configuration and browser/computer automation primitives. |
| [crates/shared](../../crates/shared) | Utilities genuinely shared across backend and agent boundaries. |
| [ui/src/common](../../ui/src/common) | Cross-host API clients, adapters, configuration and shared types. |
| [ui/src/platform](../../ui/src/platform) | Host bridge substrate. |
| [ui/src/renderer](../../ui/src/renderer) | React pages, components, hooks, services and styles; see [frontend](../architecture/frontend.md). |
| [scripts](../../scripts) | Local checks, development runners and release helpers; `bun run help` lists entry points. |
| [packaging/linux](../../packaging/linux) | Linux service and deployment files. |
| [docs](../README.md) | Maintained user, operator, contributor and architecture documentation. |

Both app hosts link `nomifun-app` in process. Its standalone `nomicore`
binary remains useful for diagnostics, stdio MCP bridges and Remote operations;
the app hosts do not spawn it.

Backend use of agent types normally passes through `nomifun-ai-agent` and
its `nomi_config` / `nomi_types` re-exports. Document and feature-gate bridge
exceptions; do not add direct `nomi-*` dependencies casually. Shared process
ownership belongs in `nomi-process-runtime`; see the
[process execution contract](../architecture/backend-crates.md).

## Build outputs

| Command | Output |
| --- | --- |
| `bun run build:ui` | `ui/dist/` |
| `cargo build -p nomifun-web` | `target/<profile>/nomifun-web` |
| `cargo build -p nomifun-app --bin nomicore` | `target/<profile>/nomicore` |
| `bun run build` | Target-specific desktop bundles under `target/` |
| `docker compose build` | Local Web host image |

Build output, local datasets and dependency trees stay untracked.
See [building and packaging](building-and-packaging.md).