# Troubleshooting

[简体中文](troubleshooting.zh.md)

Start with the actual process logs, selected data directory and host mode.
[Configuration](configuration.md) describes flags and directory precedence;
[API overview](api-overview.md) locates the owning route.

## Network and ports

| Symptom | Check |
| --- | --- |
| `invalid --host` | Use an IP literal such as `127.0.0.1` or `0.0.0.0`; hostnames are not accepted. |
| `address already in use` | Find the listener with `lsof -i :8787` (Unix) or `Get-NetTCPConnection -LocalPort 8787` (PowerShell); stop the intended process or select another port. |
| Another computer cannot connect | The Web host defaults to loopback. Check the intended bind address, firewall, reverse proxy and actual published port. Complete owner setup before broad exposure. |
| Desktop remote address is unavailable | Use the address and state shown in [Connections & Devices](../guides/webui-remote-access.md); LAN and relay access have independent controls. |

## Authentication and CSRF

`GET /api/auth/status` returning `needs_setup: true` is expected for a fresh
dedicated Web host without pre-seeded credentials. Complete first-run setup
before exposing it. `409` on setup means an owner already exists.
Startup admin environment variables do not replace an existing account.

For repeated `401` after login, inspect the request and response in browser
developer tools:

- Confirm the expected cookie/Bearer token reaches the same deployment origin.
- Confirm the proxy preserves cookies and authentication headers.
- For HTTPS deployments set `NOMIFUN_HTTPS=true`; this sets `Secure` and
  `SameSite=Strict`. A Secure cookie is not sent over ordinary HTTP.
- Check the server clock and whether a password change invalidated the session.

For `403 CSRF token validation failed`, the `nomifun-csrf-token` cookie and
`x-csrf-token` header must match. Check origin/cookie policy and proxy rewrites.
Public login/setup routes are exempt; post-login mutations follow the active
auth policy. Desktop uses the injected local-trust path.

If a password is forgotten, the trusted Desktop app can manage its WebUI
account from **Connections & Devices → Other computer browsers**.
A loopback browser or dedicated-host connection does not acquire desktop trust.
For a dedicated host, preserve its data and collect the auth error/logs for
maintainer-assisted recovery. Do not clear password hashes, delete the database,
or expose no-auth mode to regain access.

## WebSocket disconnects

Policy close code `1008` commonly means no token or an expired token.
Obtain a fresh `GET /api/ws-token`, then reconnect. Verify that the proxy
preserves the upgrade and selected authentication transport.
The normal close code is `1000`.

The server heartbeat defaults to 30s with a 60s timeout. Check proxy idle
timeouts and network changes if a connection stops receiving events.
Custom clients should reconnect with backoff and ignore unknown event types.

## Agent CLI and managed components

Conversations run the built-in `nomi` runtime. A terminal preset such as
`claude`, `codex` or `gemini` instead launches a separately installed CLI.

```bash
nomicore doctor
nomicore tool-doctor
```

Run diagnostics under the same user, PATH and data root as the application.
Correct the configured command or PATH when a third-party CLI is unavailable; subsequent launches resolve the updated tool environment.
`tool-doctor` reads the component catalogue. `tool-doctor --verify` explicitly
extracts/probes available components without opening the database or downloading
Node/Bun. The Web host executable does not expose these subcommands; build the
diagnostic binary with `cargo build --release -p nomifun-app --bin nomicore`.

Release builds embed OfficeCLI. Required Node/npm/npx or Bun/`bunx` commands prefer usable user tools, then automatically prepare approved managed supply when needed. Agent, Terminal and MCP use the same per-launch environment; hosted application processes use that environment too.
For systemd, verify service-account cache read/execute permissions and set
`SHELL` if required. See [external tools](../architecture/external-tools.md).

NomiRelay connect, restore and restart reuse approved local `nfagent` bytes or automatically prepare the current approved artifact. Progress, retry and cancellation remain available; an offline `NOMIFUN_NFAGENT_PATH` override still needs the manifest's hash.

## Office preview

Run `tool-doctor --verify` for the same data root when component admission fails.
Invalid/encrypted documents, unsupported formats, resource limits and unstable
writes have separate errors. An early process exit is a render failure.
Use the displayed retry when available; system-open and download remain useful.

For a blank artifact frame, preserve the full
`/api/office-preview-artifacts/{capability}/` path and query through the proxy.
Request identity, capability and nonce must match the active preview.
Do not weaken CSP or exempt arbitrary API routes. HTTP readiness and iframe
load alone do not prove display success. See
[Office artifact contracts](../architecture/external-tools.md).

## Data directory and database

For `unable to open database file`, check directory ownership, available space
and mount permissions for the actual service account. Docker bind mounts must
be writable by the container UID; a named volume can simplify ownership.
Preserve deployment hardening while granting the intended data path access.

For `data directory ... is already in use`, close the named holder or use a
different `NOMIFUN_DATA_DIR` / `--data-dir`.
Do not delete `server.lock` to bypass a live holder. The OS releases the lock
on exit; a leftover file is harmless. Hosts on the same channel share defaults;
development scripts select separate roots.

Incompatible readable database lineages may be rebuilt by the startup dataset
coordinator according to the current destructive baseline. Supported-schema
damage and I/O failures fail closed; restore does not gain rebuild authority.
Do not interpret schema migration counts as application version ordering.
See [data/storage](../architecture/data-and-storage.md).

## Docker build and startup

Read `docker compose logs nomifun` and `docker compose ps`.
Check port mapping, the built SPA path and data-volume permissions.
A missing admin password leaves first-run setup available; it is not a startup
crash.

Identify the failing network stage before selecting a mirror:

| Stage | Operator setting |
| --- | --- |
| Base image metadata / TLS timeout | Docker registry configuration or trusted `BUN_IMAGE`, `RUST_IMAGE`, `RUNTIME_IMAGE` |
| apt packages | `APT_MIRROR` |
| JS packages | `BUN_REGISTRY` |
| crates.io | `CARGO_REGISTRY_MIRROR` |

Use trusted compatible mirrors. See the
[deployment guide](../guides/web-server-deployment.md).

## Logs and issue reports

Logs go to stdout and daily files under `<data-dir>/logs` unless overridden.
Inspect `docker compose logs nomifun`, `journalctl -u nomifun-web` or the
local `nomicore.log`. For focused tracing, pass
`--log-level info,nomifun_mcp=trace` to `nomicore`.

Include the application version, OS, host mode, exact error, reproduction and
relevant redacted log excerpt. Keep credentials, private prompts and tokens out
of reports. Search an exact backend error with `rg` to find its owning check.