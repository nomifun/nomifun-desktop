# Building and Packaging

This page covers release artifacts from the current **NomiFun** monorepo: the
React SPA, `nomifun-web`, Tauri desktop bundles, updater payloads, Docker, and
native Linux service files.

For day-to-day loops, see [`development.md`](development.md). For operator
deployment, see [`../guides/web-server-deployment.md`](../guides/web-server-deployment.md).

## Current Status

| Artifact | Current state |
| --- | --- |
| SPA (`ui/dist`) | Built by `bun run build:ui`; consumed by desktop and web hosts. |
| `nomifun-web` | Supported self-hosted binary; auth on by default. |
| Tauri desktop bundles | Built by `bun run build` for the current OS. |
| macOS Developer ID signing + notarization | Supported through `bun run build:signed` when local Apple signing credentials are configured. |
| Tauri updater artifacts | `bun run build:updater` emits updater `.sig` files; production endpoint/key management still needs release setup. |
| Docker / Compose | Local image and compose stack are supported; no public registry image is promised here. |
| Native Linux + systemd | Unit and README live under `packaging/linux/`. |
| Windows signing | Requires an external code-signing certificate; not configured by this repository. |

## SPA

```bash
bun run build:ui
```

Output: `ui/dist/`.

Desktop builds bundle this directory through `frontendDist` in
`apps/desktop/tauri.conf.json`. `nomifun-web` serves it from `--dist` /
`NOMIFUN_WEB_DIST`; when running from the repo, the default points at
`../../ui/dist` from `apps/web`.

## Web Binary

```bash
bun run build:ui
cargo build --release -p nomifun-web
```

Runtime requirements:

- built SPA directory;
- writable data directory;
- Bun on `PATH`, unless the binary was built with `NOMIFUN_EMBED_BUN=1`;
- configured auth/admin flow, or explicit `--insecure-no-auth` for trusted
  loopback-only development.

Example:

```bash
target/release/nomifun-web --host 127.0.0.1 --port 8787 --dist ui/dist
```

First browser visit creates the admin account unless `NOMIFUN_ADMIN_USERNAME`
and `NOMIFUN_ADMIN_PASSWORD` pre-seed it.

## Desktop Bundles

```bash
bun run build
```

This runs Tauri build with `apps/desktop/tauri.conf.json`, builds the SPA first,
then creates OS-specific bundles under `target/release/bundle/`.

Product identity comes from `apps/desktop/tauri.conf.json`:

- `productName: "NomiFun"`
- `identifier: "com.nomifun.desktop"`
- version from workspace package metadata
- dev URL `http://localhost:5173`
- bundled frontend `../../ui/dist`

Tauri desktop bundles are best built on their target OS. Cross-OS desktop
packaging is not part of the supported workflow.

## macOS Signing and Notarization

Unsigned/ad-hoc macOS artifacts are useful for local testing but are not suitable
for distributing to other people. To produce a Developer ID signed and notarized
DMG:

```bash
cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing
# fill local Apple signing/notary values
bun run build:signed
```

The real `.env.signing` file and Apple private keys are ignored by git. The
wrapper script is [`scripts/desktop-build-signed.sh`](../../scripts/desktop-build-signed.sh);
the detailed setup guide is
[`apps/desktop/signing/README.md`](../../apps/desktop/signing/README.md).

## Updater Artifacts

```bash
bun run build:updater
```

This enables Tauri's `createUpdaterArtifacts` and emits `.sig` files next to the
installers. These signatures are for the Tauri updater, not for OS trust. macOS
Gatekeeper still requires Developer ID signing/notarization; Windows still needs
code signing.

The updater scaffold exists, but a production release still needs:

- production updater key management;
- hosted `latest.json` endpoint;
- release-channel policy;
- renderer flow for download/apply/restart beyond the current check surface.

See [`apps/desktop/updater/README.md`](../../apps/desktop/updater/README.md).

## Docker

```bash
docker compose up -d --build
```

The root `Dockerfile` builds the SPA with Bun, builds `nomifun-web` in release
mode, and copies the binary plus `ui/dist` into a slim runtime image. Compose
starts one `nomifun` service on port `8787` with `/data` as `NOMIFUN_DATA_DIR`.

Open `http://<server>:8787` after boot. If no admin was pre-seeded, the first
reachable browser gets the first-run admin setup screen.

The optional Caddy service in `docker-compose.yml` is commented out; use it or a
similar reverse proxy for TLS and set `NOMIFUN_HTTPS=true` when the browser
reaches the app over HTTPS.

## Native Linux + systemd

See [`packaging/linux/README.md`](../../packaging/linux/README.md). The short
shape is:

```bash
bun install
bun run build:ui
cargo build --release -p nomifun-web
sudo cp target/release/nomifun-web /opt/nomifun/
sudo cp -r ui/dist/. /opt/nomifun/web/
sudo cp packaging/linux/nomifun-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nomifun-web
```

For systemd, set `SHELL` explicitly if agent child processes need a shell; a
nologin service user often has none.

## Checks Before Sharing an Artifact

- Run `cargo check --workspace`.
- Run `bun run build:ui`.
- For desktop, build on the target OS and smoke-test launch.
- For macOS distribution, validate `codesign`, `spctl`, and `xcrun stapler`.
- For web/Docker, verify first-run admin setup, login, `/health`, and WebSocket
  connection through the intended host/reverse proxy.
