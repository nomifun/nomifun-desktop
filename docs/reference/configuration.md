# Configuration Reference

[简体中文](configuration.zh.md)

Common operator flags and environment variables, their defaults and owning source.
Use the executable's `--help` and linked modules for the complete current surface.

NomiFun ships **one** Rust backend (`nomifun-app`, binary `nomicore`) and two hosts that embed it:

- `nomifun-desktop` — the Tauri desktop shell. Boots the backend under `AuthPolicy::TrustLocalToken` on a chosen loopback port and injects a per-boot trust secret into its own WebView.
- `nomifun-web` — the standalone web/server host. Boots the same backend in **authenticated** mode by default and serves the SPA on the same port.

Both hosts share the same configuration surface for the backend; the per-host CLIs only override the bits each one owns.

## `nomifun-web` flags and environment variables

Source: [`apps/web/src/main.rs`](../../apps/web/src/main.rs).

| Flag | Env var | Default | Purpose |
|---|---|---|---|
| `--host` | `NOMIFUN_WEB_HOST` | `127.0.0.1` | IP to bind on. `0.0.0.0` accepts LAN/VPN/public traffic; pre-seed or complete first-run setup before broad exposure. Hostnames are not parsed; bad input fails fast at startup. |
| `--port` | `NOMIFUN_WEB_PORT` | `8787` | TCP port. Serves the API, the WebSocket at `/ws`, and the SPA from one socket. |
| `--data-dir` | `NOMIFUN_DATA_DIR` | per-user app-data dir | Backend data directory (SQLite database, agent state, logs, managed runtime cache). Defaults to the active channel's per-user location shared by every host; stable uses `NomiFun`, while dev uses the `NomiFun-dev` sibling. Override with the flag or `NOMIFUN_DATA_DIR` (taken literally, no suffix); use an absolute path in production. |
| `--dist` | `NOMIFUN_WEB_DIST` | `../../ui/dist` | Directory containing the built SPA. Set this explicitly when deploying outside the repo. |
| `--api-only` | — | `false` | Serve the backend without a SPA fallback; used by the Vite-proxied development loop. |
| `--admin-user` | `NOMIFUN_ADMIN_USERNAME` | `admin` | Username used when pre-seeding the first admin. Ignored once an admin exists. |
| `--admin-password` | `NOMIFUN_ADMIN_PASSWORD` | — | Pre-seeds the first admin password at boot, skipping interactive setup. Ignored once an admin exists. |
| `--insecure-no-auth` | `NOMIFUN_WEB_INSECURE_NO_AUTH` | `false` | Disables authentication entirely. Only use on loopback or an isolated trusted private network. Desktop uses local trust instead. |

Boolean envs accept `1`, `true`, `yes`, `on` (case-insensitive).

## `nomicore` (backend) flags

Source: [`crates/backend/nomifun-app/src/cli.rs`](../../crates/backend/nomifun-app/src/cli.rs).

These are the flags exposed by the standalone `nomicore` binary. The two hosts construct a defaulted `Cli` and override only what they own — so the same flags apply when the backend is run on its own.

| Flag | Default | Purpose |
|---|---|---|
| `--host` | `127.0.0.1` (`DEFAULT_HOST`) | Host address to listen on. |
| `--port` | `25808` (`DEFAULT_PORT`) | Port to listen on. |
| `--data-dir` | per-user app-data dir | Database + file storage root. Bound to the `NOMIFUN_DATA_DIR` env (literal value) via clap; with neither set it resolves `default_data_dir()` — the per-user location shared by hosts on the active build channel. |
| `--work-dir` | (none) | Working directory for conversation workspaces. Falls back to the work-root binding persisted in `dir-config.json`, then to the `NOMIFUN_WORK_DIR` env, then to the data dir itself. |
| `--app-version` | crate version | Host application version reported to the extension engine for compatibility checks. |
| `--local` | `false` | No-auth local mode for standalone `nomicore`. `nomifun-web --insecure-no-auth` maps to the same policy. The desktop shell does not use this flag; it uses `TrustLocalToken` instead. |
| `--log-dir` | `<data-dir>/logs` | Directory for rolling daily log files. |
| `--log-level` | `info` | Log level filter. Supports per-target overrides — e.g. `info,nomifun_mcp=trace`. |

`nomicore` subcommands (used internally by the agent CLI bridge and for diagnostics; the separate `nomifun-web` host does not accept them):

| Subcommand | Purpose |
|---|---|
| `mcp-requirement-stdio` | MCP stdio server for AutoWork requirement declaration tools. |
| `mcp-knowledge-stdio` | MCP stdio server for per-session knowledge search. |
| `mcp-gateway-stdio` | Internal stdio transport for platform Gateway tools; accepts only a host-issued scoped, expiring signed claim. |
| `mcp-open-stdio` | MCP stdio server exposing a reliable OS `open` tool. |
| `terminal-hook --event <kind>` | One-shot terminal lifecycle hook relay. |
| `doctor` | Self-check: hydrate the agent registry, probe every CLI on `$PATH`, print a per-agent availability table. |
| `tool-doctor [--verify]` | Read-only component/notice catalogue; `--verify` explicitly extracts and probes without initializing database or Agent data. See [external tools](../architecture/external-tools.md). |
| `remote open <binding_id>` | Open a canonical Remote AgentSession. |
| `remote turn <agent_session_id> <json-input>` | Start a turn on an explicit Remote AgentSession. |
| `remote observe <agent_session_id>` | Read Remote Session events/messages after `--after-seq`. |
| `remote cancel <agent_session_id>` | Cancel the explicit Session's active turn. |
| `backup --output <dir>` | Create a new offline backup bundle; the destination must not exist. |
| `restore --bundle <dir> --destination-data-dir <dir>` | Restore a compatible offline bundle into a new data directory. Read `--help` for scope and exclusions. |

## Shared environment variables

These are read by the backend regardless of which host embeds it.

| Env var | Read by | Effect |
|---|---|---|
| `NOMIFUN_DATA_DIR` | all hosts | Source of truth for the backend data directory when the host wants to honour it. The value is taken **literally** as the data root on every host — desktop shell, standalone web host, and the `nomicore` binary alike (the desktop shell no longer appends `/Nomi`). With it unset the dir is the per-user app-data default (see [below](#data-directory-and-work-directory-semantics)). |
| `NOMIFUN_NFAGENT_PATH` | desktop shell | Offline override pointing to an existing approved `nfagent` **absolute path**, bypassing the NomiRelay on-demand download. The file must match the current platform's approved manifest SHA-256; a mismatch is rejected. |
| `NOMIFUN_BUN_PATH` / `NOMIFUN_NODE_PATH` / `NOMIFUN_OFFICECLI_PATH` | managed runtime, debug only | Explicit absolute executables still satisfy manifest development ranges and genuine engine/capability probes; release ignores them and never falls back to PATH. |
| `NOMIFUN_EMBED_OFFICECLI` | build time | Debug `1` embeds approved OfficeCLI; release always embeds OfficeCLI. Node/Bun are managed downloads, never application payloads. |
| `NOMIFUN_WORK_DIR` | `nomicore` | Fallback for `--work-dir` (per-conversation workspace root). Ranked below the work-root binding persisted in `dir-config.json`; inherited values that name a default data-root location or a directory that no longer exists are ignored (protection against stale self-exports across auto-update restarts). |
| `JWT_SECRET` | `nomifun-app` | Secret used to sign session JWTs. See [Auth secret resolution](#auth-secret-resolution) for the resolution order. |
| `NOMIFUN_HTTPS` | `nomifun-auth::CookieConfig` | When truthy, session and CSRF cookies get the `Secure` flag and `SameSite=Strict`. Set it whenever the app is reached over HTTPS (TLS reverse proxy, etc.). Default is `false` → no `Secure` flag, `SameSite=Lax`. |
| `SHELL` | agent engine (Linux/macOS) | Shell used when the agent engine spawns child processes. On Linux servers under systemd, set this explicitly (the system account often has no `$SHELL`). |
| `NOMIFUN_URL` | `nomicore remote` | Base URL for a running instance when using canonical Remote operations. |
| `NOMIFUN_ACCESS_TOKEN` | backend startup, `nomicore remote` | Installation-scoped token for `/mcp` and `/api/remote/*`; startup values seed/rotate the stored verifier and never bind a companion. |

There is no `SENTRY_DSN` integration: the codebase does not read that environment variable.

## Backend constants

Sources: [common constants](../../crates/backend/nomifun-common/src/constants.rs),
[file service](../../crates/backend/nomifun-file/src/service.rs) and
[realtime types](../../crates/backend/nomifun-realtime/src/types.rs).
These are compile-time values, not environment variables.

| Constant | Value | Used for |
|---|---|---|
| `DEFAULT_HOST` | `127.0.0.1` | Default `--host` for `nomicore`. |
| `DEFAULT_PORT` | `25808` | Default `--port` for `nomicore`. (The web host overrides this to `8787`.) |
| `BODY_LIMIT` | `10 MiB` | Default request body limit applied to every route. Routes that need more (e.g. `/api/fs/upload`) install their own larger limit. |
| `UPLOAD_MAX_SIZE` | `30 MiB` | Cap for the file upload route (`/api/fs/upload`). |
| `MAX_REMOTE_IMAGE_SIZE` | `5 MiB` | Cap for downloading a remote image referenced in chat. |
| `COOKIE_NAME` | `nomifun-session` | Session cookie. |
| `CSRF_COOKIE_NAME` | `nomifun-csrf-token` | CSRF cookie (NOT HttpOnly — JavaScript reads it). |
| `CSRF_HEADER_NAME` | `x-csrf-token` | Header that mirrors the CSRF cookie value (Double Submit Cookie). |
| `COOKIE_MAX_AGE_DAYS` | `30` | Cookie `Max-Age`. |
| `SESSION_MAX_AGE_SECONDS` | `30d` | JWT validity window, kept identical to the browser session cookie lifetime. |
| `HEARTBEAT_INTERVAL` / `HEARTBEAT_TIMEOUT` | `30s` / `60s` | WebSocket heartbeat ping/pong. |

## Data directory and work directory semantics

- `data-dir` holds the SQLite database (`nomifun-backend.db*`), per-agent state, the managed runtime cache, log files, and any embedded extension data. Treat it like any other database — back it up and restrict permissions. Sharing it between two running backends is prevented mechanically (see the server lock below).
- All three hosts (`nomifun-desktop`, `nomifun-web`, the standalone `nomicore` binary) resolve a default through `nomifun_app::cli::default_data_dir()`. Hosts built for the same channel share it: stable uses the per-user `NomiFun` directory (`%LOCALAPPDATA%\NomiFun` on Windows, `~/Library/Application Support/NomiFun` on macOS, `$XDG_DATA_HOME/NomiFun` on Linux); non-stable channels use a **sibling** such as `NomiFun-dev` or `NomiFun-beta` — channel dirs are never nested inside the stable root. The root `dev`, `dev:web`, and `build:fast` commands select dev, while the installed app, `serve:web`, and release builds remain stable. For an explicit location, point `NOMIFUN_DATA_DIR` or `--data-dir` somewhere else; development startup does not import historical Agent state from the stable dataset.
- At startup (before the database is opened) the backend takes an OS-level **exclusive lock** on `{data_dir}/server.lock`. A second backend process on the same data dir fails fast with an error naming the holder (pid + executable) and the two ways out: close the other instance, or give this one its own directory via `NOMIFUN_DATA_DIR` / `--data-dir`. The lock is advisory (`flock` / `LockFileEx` via `fs2`) and is released by the OS when the process exits or crashes — a leftover `server.lock` file is harmless. `nomicore doctor` and the `mcp-*` stdio subcommands do not take the lock (doctor is designed to run alongside a live server).
- `work-dir` holds per-conversation workspaces. When unset, it resolves in this order: `--work-dir` → the work-root binding persisted in `dir-config.json` → non-empty `NOMIFUN_WORK_DIR` env → the data dir itself. An inherited `NOMIFUN_WORK_DIR` that names a default data-root location or a directory that no longer exists is ignored — this guards against stale self-exports across auto-update restarts. Conversations create subdirectories under `<work-dir>/conversations/`; deleting a conversation deletes its workspace.
- Every host treats `NOMIFUN_DATA_DIR` / `--data-dir` as the requested data root without appending `/Nomi`. After a Settings storage relocation, its fixed locator resolves an old requested root to the committed current root. Settings moves the healthy current dataset, including history, attachments, configuration and managed workspaces, on restart; the source is retained and sealed. External project directories are unchanged. Remove an explicit `--work-dir` before using this operation. Historical self-export roots normalize to the channel default without importing retired datasets. See [storage relocation](../architecture/data-and-storage.md#relocating-the-current-dataset).

## Auth secret resolution

`JwtService` is constructed from a single secret; `AppServices::from_config` resolves it in this order:

1. `JWT_SECRET` environment variable, if set.
2. Otherwise, the value persisted on the installation-owner user row selected
   by `installation_identity.owner_user_id`.
3. Otherwise, a fresh cryptographically random secret is generated and **persisted to the database** for future boots.

The change-password flow rotates the JWT secret as a side effect, invalidating every existing session.

At-rest encryption uses a separate persistent key stored at `<data-dir>/encryption_key`.
On older installs where that file does not exist yet, startup seeds it from the currently resolved JWT secret so existing encrypted fields remain readable. After that first seed, changing the password or rotating the JWT secret does not change the data-encryption key.

## TLS / HTTPS cookie handling

NomiFun does not terminate TLS itself — put a TLS-terminating reverse proxy (Caddy, nginx, …) in front. When you do:

- Set `NOMIFUN_HTTPS=true` so cookies are flagged `Secure` and `SameSite=Strict`.
  Secure cookies are not sent over ordinary HTTP; keep the public origin and
  reverse-proxy scheme consistent.
- The WebSocket upgrade at `/ws` passes through any standards-compliant proxy without extra headers; Caddy handles it out of the box.

See [`guides/web-server-deployment.md`](../guides/web-server-deployment.md) for a worked Caddy + Docker setup.

## Logging

- All logs go to both stdout (so `journalctl`/`docker logs` capture them) and a daily-rolling file at `<log-dir>/nomicore.log`.
- `--log-level` accepts a full [`tracing` `EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) directive: a global level, or a comma-separated list of per-target overrides.

  Examples:

  - `info` — global info.
  - `debug` — global debug. Verbose; useful for short reproductions.
  - `info,nomifun_mcp=trace` — info everywhere, trace for the MCP module.
  - `warn,nomifun_conversation=info,nomifun_terminal=debug` — quieter overall, normal for the conversation engine, debug for terminals.

The standalone `nomicore --log-level` flag controls the backend filter.
The Web host does not expose this flag, and the backend does not read
`RUST_LOG` or `NOMIFUN_LOG_LEVEL`.

## See also

- [Web Server Deployment](../guides/web-server-deployment.md) — running `nomifun-web` with Docker, systemd, Caddy.
- [Running NomiFun as a Desktop App](../guides/desktop-app.md) — desktop-specific configuration.
- [API Overview](./api-overview.md) — what the backend exposes once it is configured and running.
- [Troubleshooting](./troubleshooting.md) — symptoms and fixes when configuration ends up wrong at runtime.
