# Web Server Deployment

`nomifun-web` is the **headless, self-host** way to run NomiFun. It is the same Rust backend that the [desktop app](./desktop-app.md) embeds, but built as a standalone binary that also serves the SPA (`ui/dist`) on the same port. There is no GUI, no WebView, no `DISPLAY` requirement — it runs anywhere a Linux/macOS/Windows server will run a static binary.

Unlike the desktop shell, **`nomifun-web` requires authentication by default**. The first browser visitor either creates the admin account interactively (first-run setup), or you pre-seed credentials with `NOMIFUN_ADMIN_PASSWORD`.

> If you want to expose an *existing* desktop install for remote access without setting up a server, see [WebUI Remote Access](./webui-remote-access.md). That is a per-instance feature; this guide is for a dedicated server.

```text
  Browser / phone / LAN              nomifun-web  (one process, one port)
  ┌──────────────────┐               ┌───────────────────────────────────────┐
  │  SPA + login      │  HTTP / WS    │  axum router                          │
  │  (ui/dist)        │ ────────────► │   ├─ /            → SPA (ui/dist)      │
  └──────────────────┘               │   ├─ /api/*       → REST handlers      │
                                      │   ├─ /ws          → WebSocket events   │
                                      │   └─ /login …     → auth (on by default)│
                                      │                                       │
                                      │  embedded backend (nomifun-app)        │
                                      │   └─ SQLite · agents · cron · channels │
                                      └───────────────────────────────────────┘
```

## Quick start

### Run the binary directly

```bash
cargo build --release -p nomifun-web
./target/release/nomifun-web --host 127.0.0.1 --port 8787 \
  --data-dir ./data --dist ./ui/dist
```

Then open `http://127.0.0.1:8787` and the first visit lets you create the admin account. After that, the setup endpoint returns `409 Conflict` and the only way in is via the login form (or `NOMIFUN_ADMIN_PASSWORD`).

![First-run admin setup screen](../images/webserver-02-first-run-setup.png)

### Or via Cargo, from the repo

```bash
bun install
bun run build:ui              # produces ui/dist
cargo run -p nomifun-web      # picks up the default --dist=../../ui/dist
```

## CLI flags and environment variables

All flags below are read by `apps/web/src/main.rs`. Each has an environment-variable counterpart for systemd, Docker, and other deployment platforms.

| Flag | Env var | Default | Purpose |
|---|---|---|---|
| `--host` | `NOMIFUN_WEB_HOST` | `127.0.0.1` | IP to bind on. `0.0.0.0` accepts LAN/VPN/public traffic; pre-seed the admin or complete first-run setup before broad exposure. |
| `--port` | `NOMIFUN_WEB_PORT` | `8787` | TCP port. Serves the API, the WebSocket at `/ws`, and the SPA. |
| `--data-dir` | `NOMIFUN_DATA_DIR` | per-user dir | Backend data dir (SQLite database, agent state, logs, Bun cache). Defaults to the per-user location shared with the desktop app (`%LOCALAPPDATA%\NomiFun\Nomi`, `~/Library/Application Support/NomiFun/Nomi`, `$XDG_DATA_HOME/NomiFun/Nomi`). **Still set an explicit absolute path in production.** |
| `--dist` | `NOMIFUN_WEB_DIST` | `../../ui/dist` | Directory containing the built SPA. **Set this explicitly when deploying.** |
| `--admin-user` | `NOMIFUN_ADMIN_USERNAME` | `admin` | Username used when pre-seeding the first admin. Ignored once an admin exists. |
| `--admin-password` | `NOMIFUN_ADMIN_PASSWORD` | — | Pre-seed the first admin password at boot, skipping interactive setup. Ignored once an admin exists. |
| `--insecure-no-auth` | `NOMIFUN_WEB_INSECURE_NO_AUTH` | `false` | **DANGER.** Disables authentication entirely (desktop-style local mode). Only use on loopback or a fully trusted private network. |
| — | `NOMIFUN_HTTPS` | `false` | When `true`, session and CSRF cookies are flagged `Secure`. Set this whenever the app is reached over HTTPS (e.g. behind a TLS reverse proxy). |
| — | `SHELL` | platform default | Shell used by the agent engine when spawning processes. Set to `/bin/bash` on Linux servers if `$SHELL` is unset. |

Boolean envs accept `1`, `true`, `yes`, `on` (case-insensitive).

A bad `--host` (anything that does not parse as an IP) fails fast at startup with a clear error rather than a cryptic socket error.

At startup the backend takes an OS-level exclusive lock on `{data_dir}/server.lock` — **one backend instance per data dir**. A second process pointed at the same directory fails fast with an error naming the current holder (pid + exe); to deploy multiple instances, give each its own `NOMIFUN_DATA_DIR` / `--data-dir`. The OS releases the lock on exit or crash, so a leftover `server.lock` file is harmless.

### Password and username rules

When the admin account is created (interactively or via pre-seed), values are validated server-side:

- **Username**: 3–32 chars, `[a-zA-Z0-9_-]`, must not start or end with `-` / `_`.
- **Password**: 8–128 chars, rejected if it appears in a small common-passwords list (`password`, `12345678`, `qwertyui`, …).

A weak `NOMIFUN_ADMIN_PASSWORD` will refuse to boot. A weak interactively-typed password will return `400` with the validation message.

## First-run admin provisioning

There are two supported paths.

### Interactive (default)

Leave `NOMIFUN_ADMIN_PASSWORD` unset. On a fresh data dir the install is "uninitialised": `GET /api/auth/status` reports `needs_setup: true`, the SPA shows the first-run form, and the **first browser visitor's chosen username + password become the admin** via an atomic `POST /api/auth/setup`. The write is a conditional UPDATE — even two concurrent first-run requests cannot both win; the loser receives `409 Conflict`.

> **Security note — the first-run window.** Between the moment the server is reachable and the moment you complete setup, anyone who can reach the port can claim the admin account. On a non-loopback bind the server logs a loud warning. Mitigate by completing setup over a trusted tunnel/VPN first, or pre-seed (next section) so the install is initialised before it goes live.

### Pre-seeded (recommended for automation)

Provide `NOMIFUN_ADMIN_PASSWORD` (and optionally `NOMIFUN_ADMIN_USERNAME`, default `admin`) before first boot. The bootstrap routine hashes and stores the credentials atomically, the first-run setup endpoint returns `409` from the very first start, and there is no window for someone else to claim the account.

```bash
NOMIFUN_ADMIN_USERNAME=alice \
NOMIFUN_ADMIN_PASSWORD='change-me-to-something-strong' \
nomifun-web --host 0.0.0.0 --port 8787 \
  --data-dir /var/lib/nomifun --dist /opt/nomifun/web
```

The pre-seed is **idempotent** — once an admin exists, the env vars are ignored on subsequent boots. To rotate credentials, use the in-app change-password / change-username flow rather than the env vars.

## Docker

The repo ships a multi-stage `Dockerfile` and a `docker-compose.yml`. The image:

1. Builds the SPA with Bun.
2. Compiles `nomifun-web` from the workspace.
3. Assembles a slim `debian:bookworm-slim` runtime that includes `bun`, `git`, and `ripgrep`.

It exposes port `8787` and uses `/data` as the data volume.

### Compose

```bash
docker compose up -d --build
# then open http://<server-ip>:8787 and create the first admin
```

`restart: unless-stopped` makes the service start on host boot — installing it *is* enabling it. The default ports block publishes `8787:8787` directly; pre-seed the admin or complete setup on a trusted network before exposing it broadly. Add TLS (next section) before exposing to the internet.

Verify readiness:

```bash
docker compose logs -f nomifun
# look for: "nomifun-web: embedded backend + SPA on one port"
```

The compose file mounts a named volume `nomifun-data:/data` which holds the SQLite DB, logs, the Bun runtime cache, and per-agent state. Back this up with the same care as any other database.

### Pre-seeding the admin in Compose

Uncomment the `environment:` block:

```yaml
environment:
  NOMIFUN_ADMIN_USERNAME: admin
  NOMIFUN_ADMIN_PASSWORD: "change-me-to-something-strong"
  NOMIFUN_HTTPS: "true"        # when fronted by Caddy / nginx with TLS
```

### Building behind a slow registry

The Rust stage accepts a `CARGO_REGISTRY_MIRROR` build arg for cargo registry mirroring (e.g. on a network where crates.io is slow):

```bash
docker build --build-arg CARGO_REGISTRY_MIRROR=https://rsproxy.cn/index/ -t nomifun-web:local .
```

```text
$ docker compose up -d
[+] Running 2/2
 ✔ Network nomifun_default  Created
 ✔ Container nomifun-web    Started

$ docker compose logs -f web
nomifun-web  | listening on 0.0.0.0:8787 (auth: enabled)
```

## TLS via Caddy reverse proxy

A `Caddyfile` is included for Caddy 2. Caddy auto-provisions HTTPS certificates (Let's Encrypt or ZeroSSL by default) and proxies to the app. The WebSocket upgrade at `/ws` passes through automatically, no extra config required.

```caddy
your.domain.com {
    encode zstd gzip
    reverse_proxy nomifun:8787
}
```

To enable the Caddy service in `docker-compose.yml`:

1. Edit `Caddyfile` and replace `your.domain.com` with your real domain.
2. Set `NOMIFUN_HTTPS=true` in the `nomifun` service env (so cookies get the `Secure` flag).
3. Replace `ports: ["8787:8787"]` with `expose: ["8787"]` so only Caddy is published.
4. Uncomment the `caddy:` service and the `caddy-data` / `caddy-config` volumes.
5. `docker compose up -d`.

The app already provides its own login screen, so **do not configure HTTP basic auth in Caddy** — Caddy's job is purely TLS termination and proxying.

For a LAN-only host without a public domain you can use an internal name with `tls internal`, or just publish port `8787` directly without Caddy (the in-app login still protects it).

## systemd (Linux server, no Docker)

The repo includes `packaging/linux/nomifun-web.service` and a long-form Linux deployment guide at `packaging/linux/README.md`.

### Build artifacts

You need a Linux build host (cross-compiling the C dependencies from Windows is painful — the easiest workaround is to extract the binary from the Docker image with `docker cp`). On Linux:

```bash
bun install
bun run build:ui                      # → ui/dist (~21MB)
cargo build --release -p nomifun-web  # → target/release/nomifun-web
```

### Layout

```
/opt/nomifun/nomifun-web    # the binary
/opt/nomifun/web/           # contents of ui/dist
/var/lib/nomifun/           # data dir (created by systemd's StateDirectory)
```

```bash
sudo useradd --system --home /var/lib/nomifun --shell /usr/sbin/nologin nomifun
sudo mkdir -p /opt/nomifun/web
sudo cp target/release/nomifun-web /opt/nomifun/
sudo cp -r ui/dist/. /opt/nomifun/web/
```

### Bun must be on the system `PATH`

The agent engine requires **`bun ≥ 1.3.13`** as a runtime dependency. Because the service runs under a `nologin` system account, an install in someone's `~/.bun/bin/` is invisible to it. Pick one:

- **System install**: `curl -fsSL https://bun.sh/install | bash`, then `sudo install ~/.bun/bin/bun /usr/local/bin/bun`.
- **Embed in the binary**: build with `NOMIFUN_EMBED_BUN=1 cargo build --release -p nomifun-web`. Bun is bundled into the binary and self-extracts into the data dir on first run.

Verify: `sudo -u nomifun -s -- which bun` must return a path. Otherwise the first agent spawn will fail with an opaque error.

### Install the unit

```bash
sudo cp packaging/linux/nomifun-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nomifun-web
sudo systemctl status nomifun-web
```

The shipped unit:

- Binds `127.0.0.1:8787` by default. Change `NOMIFUN_WEB_HOST` to
  `0.0.0.0` only after first-run setup is complete or
  `NOMIFUN_ADMIN_PASSWORD` is configured.
- Sets `NOMIFUN_DATA_DIR=/var/lib/nomifun` to match the systemd-managed `StateDirectory=nomifun`. **Keep these two in sync** — if you drop the env line, the data dir silently falls back to the service user's per-user directory (`$XDG_DATA_HOME/NomiFun/Nomi`, typically `~nomifun/.local/share/NomiFun/Nomi`), decoupled from systemd state.
- Runs as a dedicated `nomifun` user (`User=nomifun`, `Group=nomifun`).
- Restarts on failure with a 3 s backoff.
- Applies moderate hardening (`NoNewPrivileges=yes`, `PrivateTmp=yes`). **Do not add** `ProtectHome=yes` or strict `ProtectSystem` — the agent engine reads/writes operator-directed files and over-sandboxing breaks core features.

To enable HTTPS cookies behind a TLS proxy, uncomment:

```ini
Environment=NOMIFUN_HTTPS=true
```

To pre-seed the admin instead of interactive setup:

```ini
Environment=NOMIFUN_ADMIN_USERNAME=admin
Environment=NOMIFUN_ADMIN_PASSWORD=change-me-to-something-strong
```

```text
$ sudo systemctl status nomifun-web
● nomifun-web.service - NomiFun web host
     Loaded: loaded (/etc/systemd/system/nomifun-web.service; enabled; preset: enabled)
     Active: active (running) since Tue 2026-06-25 09:12:03 UTC
   Main PID: 12345 (nomifun-web)
     CGroup: /system.slice/nomifun-web.service
             └─12345 /usr/local/bin/nomifun-web --host 127.0.0.1 --port 8787 …
nomifun-web[12345]: listening on 127.0.0.1:8787 (auth: enabled)
```

## Linux runtime dependencies

| Dependency | Required? | Notes |
|---|---|---|
| `glibc` + `ca-certificates` | Yes | sqlite is statically linked, TLS uses rustls — **no openssl, no libsqlite needed**. |
| `bun` ≥ 1.3.13 | **Yes** | Agent execution runtime. 1.1.38 has an stdin bug; do not use. Already inside the Docker image. |
| `node` / `npm` / `npx` | Recommended | Many user-configured MCP stdio servers launch via `npx -y …`. |
| `git` | Recommended | Skill discovery and a few built-in tools. |
| `ripgrep` (`rg`) | Recommended | Code-search backend. Falls back to `grep` if missing. |
| `DISPLAY` / X11 / WebView | **No** | `nomifun-web` is fully headless. |

## Security checklist

- **Use TLS for any public deployment.** Cookies and login credentials over plain HTTP can be sniffed. Behind a TLS proxy, set `NOMIFUN_HTTPS=true` so the session cookie is flagged `Secure`.
- **Strong admin password.** The validator rejects passwords below 8 chars and a few obvious dictionary entries, but it does not enforce a strength score — pick something long and random. Change it from the in-app flow whenever you suspect compromise; the change-password endpoint rotates the JWT secret, invalidating every existing session.
- **Close the first-run window** with `NOMIFUN_ADMIN_PASSWORD` for any host that becomes reachable before you are ready to interactively complete setup. Alternatively keep the service on `127.0.0.1` until setup is finished, then intentionally bind `0.0.0.0`.
- **`--insecure-no-auth` is hostile by default.** It disables authentication completely; *anyone* who can reach the port becomes a privileged user with shell, file, and agent access. Only use on a loopback bind or a fully trusted private network. The server logs a warning when it is enabled on a non-loopback address.
- The backend has terminal, filesystem, and agent execution capabilities — running it remotely is, by design, equivalent to giving yourself remote code execution on the host. Auth + TLS are the floor, not the ceiling. Treat the data dir and the admin password the same way you would treat root credentials.

## Troubleshooting

**`invalid --host '<value>'`.** Pass an IP literal (`127.0.0.1`, `0.0.0.0`, an explicit interface IP). Hostnames are not parsed.

**Cookies don't stick over HTTPS.** Set `NOMIFUN_HTTPS=true` so the `Secure` flag is added. Without it, browsers reject the cookie on HTTPS responses.

**Agent commands fail with `bun: command not found` under systemd.** Install bun system-wide (see the bun-on-PATH section above) or rebuild with `NOMIFUN_EMBED_BUN=1`.

**Healthcheck.** Use `GET /health` for process liveness. Use
`GET /api/auth/status` only when the caller also needs setup/auth state.

## See also

- [Running NomiFun as a Desktop App](./desktop-app.md)
- [WebUI Remote Access](./webui-remote-access.md) — turn an existing desktop install into a remotely-accessible server (without provisioning a separate machine).
- `packaging/linux/README.md` — deeper Linux notes (mostly Chinese; this guide subsumes the English content).
- `apps/web/src/main.rs` — the source of truth for flags, env vars, and bootstrapping order.
