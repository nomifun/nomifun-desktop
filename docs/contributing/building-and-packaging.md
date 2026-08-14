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
| Docker / Compose | Official Docker Hub image [`nomifun/nomifun-web`](https://hub.docker.com/repository/docker/nomifun/nomifun-web) is available; local image and compose builds remain supported. |
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

## On-demand `nfagent` runtime

Desktop installers do **not** bundle `nfagent`. The first NomiRelay pairing,
restore, or agent restart downloads the binary for the current OS/architecture
from the immutable HTTPS URL in
[`apps/desktop/nfagent-runtime.json`](../../apps/desktop/nfagent-runtime.json).
Users who never use NomiRelay therefore do not carry the extra roughly 6–7 MiB
agent in the installer.

The managed runtime installation has these constraints:

- The full SHA-256 is checked before every use; a corrupt cache is removed and
  downloaded again.
- Downloads are capped at 32 MiB, written to a staging file, verified, and only
  then published atomically.
- The cache lives at
  `<data-dir>/runtime/nfagent/<version>/<sha256-prefix>/<asset-name>` and is
  reused on later launches.
- Initial pairing installs the runtime before consuming the one-shot Relay
  invitation, so a download failure does not waste the invitation.
- Runtime URLs cannot use `latest`, query parameters, or fragments, and asset
  names must match the fixed target-platform names.

For local development or offline debugging, set this before launching Desktop:

```text
NOMIFUN_NFAGENT_PATH=<absolute path to an existing nfagent binary>
```

An explicit override is treated as a developer-supplied trusted file and does
not go through the managed manifest SHA-256 check. Do not use it as the normal
end-user installation path.

When updating `nfagent`, first publish new immutable, versioned assets from
`nomifun-net-infra`, then update the version, URLs, and SHA-256 values in
`nfagent-runtime.json`. Do not distribute a Desktop build until every URL in
the manifest is live. See the root [`RELEASING.md`](../../RELEASING.md) for the
manual upload runbook.

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

Published runtime image:
[`nomifun/nomifun-web`](https://hub.docker.com/repository/docker/nomifun/nomifun-web).
The command below builds the image locally from the current checkout.

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
