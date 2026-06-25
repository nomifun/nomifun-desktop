# FAQ

Honest, short answers to questions that come up over and over. For deeper explanations follow the links.

## What is the difference between NomiFun and nomifun?

**NomiFun** is the open-source project and the user-facing product name: the desktop app, WebUI surface, codebase, workspace, GitHub repository, and brand all use this spelling.

In this codebase the lowercase form `nomifun` shows up only as a literal technical identifier — package names (`nomifun-app`, `nomifun-web`, …), the desktop bundle id `com.nomifun.desktop`, environment variables prefixed `NOMIFUN_`, repository directories. Anywhere it is shown to a human as the app or project brand, use "NomiFun".

## Is there a hosted version?

No. NomiFun is a self-host project. There is no SaaS instance, no managed login at a `nomifun.com` URL, no central account system to sign up for. The two ways to use it are:

- Install the desktop app and run it locally — `nomifun-desktop`.
- Deploy `nomifun-web` on a server you control. See [Web Server Deployment](../guides/web-server-deployment.md).

You can expose your desktop install temporarily to other devices (your phone, your laptop) using [WebUI Remote Access](../guides/webui-remote-access.md), but that is a per-instance feature, not a hosted service.

## Does the desktop app require login?

No. The desktop WebView is trusted through a per-boot local trust token injected
by the Tauri shell. There is no login screen for the desktop window, but the
embedded backend is not a blanket no-auth localhost server.

The web host is the opposite: it requires login by default. Mixing the two is
intentional — the desktop shell can trust its own WebView, while a
network-reachable host needs an actual auth boundary.

## Where is my data stored?

In the **data directory**. Its location depends on which host you are running:

- **Desktop**: defaults to the **per-user application-data dir** — `%LOCALAPPDATA%\NomiFun\Nomi` on Windows, `~/Library/Application Support/NomiFun/Nomi` on macOS, `$XDG_DATA_HOME/NomiFun/Nomi` (usually `~/.local/share/NomiFun/Nomi`) on Linux. Set `NOMIFUN_DATA_DIR=<absolute path>` and the dir becomes `$NOMIFUN_DATA_DIR/Nomi` (unchanged override semantics). Older builds stored data under `<system temp>/nomifun-data/Nomi`; if such an install exists it is relocated to the new location automatically on launch, and the old dir is kept as a backup.
- **Web (`nomifun-web`)**: whatever you pass to `--data-dir` (or `NOMIFUN_DATA_DIR`), taken literally — no `/Nomi` suffix. With neither set it defaults to the **same per-user dir as the desktop app**, so a dev `bun run serve:web` and the installed app see one shared state.
- **Docker**: the named volume defined in the compose file (`nomifun-data` mounted at `/data`).

The data directory contains the SQLite database (`nomifun-backend.db*`), per-agent state, the Bun cache, log files, and any embedded extension data. Back it up like a database. Because every host defaults to the same directory, the backend guards it with an exclusive `server.lock` — a second backend instance on the same data dir fails fast instead of corrupting state.

For the full lifecycle and `work-dir` semantics, see [Configuration Reference](./configuration.md#data-directory-and-work-directory-semantics).

## Which agents and providers are supported?

The "Agent CLIs" NomiFun runs as ACP (Agent Client Protocol) backends include `claude`, `codex`, `gemini`, `nomi`, `codebuddy`, `qwen`, and `opencode`. Each one is a separate CLI you install on your system; NomiFun discovers them on `PATH` and the registry hydrates from there. Run `nomicore doctor` to see what your install detects.

For raw model access (e.g. provider keys, custom OpenAI-compatible endpoints), the system supports configurable providers via `/api/providers/*` and the in-app settings UI. You bring the API keys; NomiFun stores them encrypted at rest in the data directory.

There is no built-in agent that calls out to a hosted NomiFun endpoint — there is no such endpoint. Every agent / provider you configure is something you control.

## Is NomiFun really local-only?

The application logic and your data are local. The agents you connect to may not be — most CLI agents make outbound calls to their respective providers (Anthropic, OpenAI, Google, …). That is between you and the agent.

What NomiFun itself does over the network:

- Optional update checks (system info / check-update endpoint).
- Extension marketplace (`/api/hub/*`) — only if you actively use it.
- Whatever your configured agents and providers do — typically API calls to LLM providers.

There is no telemetry pipeline, no analytics SDK, no `SENTRY_DSN` integration in the binary. The backend does not phone home on its own.

## What about extensions and skills — what runs them?

Extensions (themes, assistants, channel plugins, settings tabs) are loaded by `nomifun-extension` from the data directory. Skills are bundles of prompts/instructions resolved into the agent's context per-conversation. Both are local files under your data dir; the marketplace flow simply downloads them into that directory.

The agent CLI binaries are not extensions — they are external CLIs that NomiFun launches as child processes via the ACP protocol.

## Can I run agents on a different machine from the UI?

Yes — that is exactly what `nomifun-web` is for. Deploy the web host on the machine where you want the agents (and their CLIs, and their network access) to live, and access the SPA from any browser. See [Web Server Deployment](../guides/web-server-deployment.md).

For lighter-weight remote access from a phone or another laptop without spinning up a separate server, [WebUI Remote Access](../guides/webui-remote-access.md) exposes an existing desktop install over the LAN.

## What is the licence?

**Apache-2.0**, declared in the workspace `Cargo.toml`. You can use, modify, redistribute, and bundle the code under the standard Apache-2.0 terms — including in commercial products — provided you keep the licence and notice intact.

## Are there prebuilt installers?

Not yet. Desktop bundles can be built locally, macOS Developer ID signing is
scripted through `bun run build:signed`, and updater artifacts can be generated
with `bun run build:updater`; there is not yet an official public release
channel or registry-backed installer feed. Until then, the supported install
paths are:

- **Desktop**: `bun install && bun run build:ui && cargo run -p nomifun-desktop` (or `cargo build --release -p nomifun-desktop`).
- **Server**: build from source (`cargo build --release -p nomifun-web`) or `docker compose up -d --build`.

When prebuilt installers ship, they will be linked from the project README and the [getting-started guide](../getting-started/).

## I lost my admin password

On `nomifun-web`, the in-band recovery flow is the local-only WebUI route (`POST /api/webui/reset-password`) — you can hit it from the same machine the server runs on, and it generates a fresh random password and prints it. From a remote machine you cannot recover the password through the API.

The fallback is to stop the server, edit the database directly (the `system_default_user.password_hash` column), and restart. The simplest reset is to set the hash to an empty string — the next boot then treats the install as needing first-run setup again, and the next visitor can claim the admin.

For desktop installs there is no password for the local WebView. WebUI remote
access has its own admin password because it is reachable from another browser.

## Is there a "single binary" build?

Yes for the server: `nomifun-web` is one statically-linked Rust binary plus the `ui/dist/` directory it serves. SQLite is statically linked, TLS uses rustls — there is no `libsqlite`, no `openssl` dependency at runtime. Build with `cargo build --release -p nomifun-web`.

The agent runtime (bun) is *not* embedded by default; install it system-wide or use the `NOMIFUN_EMBED_BUN=1` build flag to bundle it into the binary. See the [bun-on-PATH](../guides/web-server-deployment.md#bun-must-be-on-the-system-path) section.

The desktop shell also produces a single binary (`nomifun-desktop`), but for distribution you typically want the platform-native packaging via `bun run build` once installer signing is set up.

## See also

- [Configuration Reference](./configuration.md)
- [API Overview](./api-overview.md)
- [Troubleshooting](./troubleshooting.md)
- [Web Server Deployment](../guides/web-server-deployment.md)
- [Running NomiFun as a Desktop App](../guides/desktop-app.md)
