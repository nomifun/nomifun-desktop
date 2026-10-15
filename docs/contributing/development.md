# Development

[简体中文](development.zh.md)

For installing or operating NomiFun, use
[installation](../getting-started/installation.md) or
[Web server deployment](../guides/web-server-deployment.md).

## Prerequisites

- Stable Rust supporting edition 2024 and resolver 3.
- Bun >= 1.3.13, Git, CMake >= 3.16 and a native C/C++ toolchain.
- Windows: MSVC build tools and WebView2. macOS: Xcode Command Line Tools.
  Linux: follow the maintained
  [system package list](../getting-started/installation.md#prerequisites).

Clean native builds may download third-party build dependencies, including ONNX
Runtime. Preserve dependency caches for offline rebuilds. Platform build details
belong in [building and packaging](building-and-packaging.md).

## Install

```bash
git clone <repo-url> nomifun-desktop
cd nomifun-desktop
bun install
```

Rust workspace membership is in [Cargo.toml](../../Cargo.toml); the Bun
workspace contains `ui/`. Root scripts are declared in
[package.json](../../package.json); run `bun run help` to discover commands.

## Development loops

| Command | Behavior |
| --- | --- |
| `bun run dev:ui` | Vite on port 5173 without a backend; API calls may fail. |
| `bun run dev:web` | Vite plus API-only `nomifun-web` on port 8787 with authentication disabled for local iteration. |
| `bun run serve:web` | Builds the SPA and starts the authenticated Web host from source. |
| `bun run dev` | Tauri, Vite and an embedded backend under desktop local trust. |

Keep the no-auth development host on loopback or an isolated trusted network.
Desktop requests use a per-boot local trust secret; the desktop host embeds
the backend in process.

Development commands select the dev build channel. The desktop dev runner
derives a schema-specific sibling data directory
`NomiFun-dev-schema-<fingerprint>` and preserves prior directories.
`serve:web` and release builds use the stable channel.
To isolate an instance, set an explicit absolute `NOMIFUN_DATA_DIR`;
its value is used literally. Two backends cannot hold the same data directory.

See [configuration](../reference/configuration.md) for data/work directory
precedence, channel defaults and logging. For example, in a POSIX shell:

```bash
NOMIFUN_DATA_DIR=/tmp/nomifun-dev bun run serve:web
```

## Verification

Choose checks that directly cover the changed files and behavior.
Documentation-only changes do not require compilation. Broad checks are useful
for cross-cutting changes and releases.

| Change | Check |
| --- | --- |
| Documentation | `bun run check:docs` and `git diff --check` |
| Script registry/help | `bun run help --check` |
| Renderer types | `bun run typecheck` |
| Renderer/UI rules | `bun run check:desktop-ui-boundary` plus focused UI tests |
| Locale JSON/types | `bun run gen:i18n`, then `bun run check:i18n` |
| Styles/presets | `bun run check:theme`, `bun run check:dead-css`; see [styling](styling.md) |
| Rust compile path | `cargo check -p <crate>` |
| Rust behavior | `bun run test:crate <crate> [cargo args]` or focused `cargo test -p <crate>` |
| Canonical Agent storage/content | Read [Agent Session](../architecture/agent-session.md), run `bun run check:agent-session-boundary` and affected history/reset/consumer tests |
| Schema/IDs/reset/restore | Follow [data standards](data-and-identifier-standards.md) and affected repository/schema tests |
| Broad regression | `bun run check`, `cargo check --workspace`, and relevant Rust/UI tests |

`bun run test:core`, `bun run test:desktop` and `bun run test` provide
broader Rust regression levels. `bun run test:ui` runs UI tests; use a file
filter for a local fix. Record commands actually run and explain unavailable
platform checks.

## Diagnostics

`nomicore` remains available independently of the application hosts:

```bash
cargo run -p nomifun-app --bin nomicore -- doctor
cargo run -p nomifun-app --bin nomicore -- tool-doctor
```

`doctor` reports detected terminal Agent CLIs. `tool-doctor` reads the
component/notice catalogue; `--verify` explicitly extracts and probes tools.
For Remote operations, stdio bridges and backup/restore commands, read
[CLI configuration](../reference/configuration.md#nomicore-backend-flags)
and the executable `--help`.

## Further reading

- [Project structure](project-structure.md)
- [Backend ownership](../architecture/backend-crates.md)
- [Frontend](../architecture/frontend.md)
- [Building and packaging](building-and-packaging.md)
- [Troubleshooting](../reference/troubleshooting.md)