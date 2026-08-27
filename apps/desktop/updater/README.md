# NomiFun Desktop Updater

中文说明见 `apps/desktop/updater/README.zh-CN.md`.

In-app auto-update for `nomifun-desktop`, built on **Tauri's native updater**
(`tauri-plugin-updater` + minisign signatures). CrabNebula Cloud is the primary
update and download service; GitHub Releases remains the fallback and archive.
The app checks → downloads → verifies → installs → relaunches.

## How it works

```
App (running version, from workspace Cargo.toml)
  └─ check() ──► CrabNebula dynamic endpoint
        ├─ request failure ──► GitHub Releases latest.json
        └─ newer version? ──► download the selected platform artifact
              ├─ non-GitHub download/verification failure ──► retry same version from GitHub
              └─ verify signature against plugins.updater.pubkey
                    └─ install (swap .app / run NSIS) ──► relaunch
```

- **Frontend wiring (done):** `ui/src/common/adapter/tauriUpdater.ts` uses
  `@tauri-apps/plugin-updater` for update checks and separate Rust commands for
  package download and installation (+ `plugin-process` for relaunch). The
  native download command retains the verified package; the install command can
  only consume that package and never performs a hidden second download. The
  in-app `UpdateModal` drives check → download (progress) → install. Entry points:
  - **About page** "检查更新" button (shell-gated on `isDesktopShell()`).
  - **Startup silent check** (`Layout.tsx`): on launch, if a newer version is
    available the modal opens automatically; otherwise it stays silent.
- **Config:** `apps/desktop/tauri.conf.json` →
  - Primary endpoint = CrabNebula dynamic Tauri endpoint.
  - Secondary endpoint = GitHub Releases `latest.json`.
  - `plugins.updater.pubkey` = the project updater public key (committed; safe).
- **Timeout and fallback:** metadata checks are bounded to eight seconds. The
  native download command retries GitHub explicitly when a package selected
  from a non-GitHub endpoint fails to download or verify. The retry must select
  the exact version requested by the renderer.
- **Permissions:** `apps/desktop/capabilities/default.json` grants the renderer
  `updater:allow-check` and `process:default` (relaunch/exit). Raw updater
  download/install permissions are intentionally absent; the version-bound Rust
  commands own those operations.

## Signing keys

A single minisign keypair signs **every** platform/chip; the matching public key
is embedded in the app for verification.

- **Public key** → `plugins.updater.pubkey` in `tauri.conf.json` (committed).
- **Private key** → `apps/desktop/signing/nomifun-updater.key` — **gitignored**
  (`*.key`), never committed, never printed. **Back it up** to your release
  secret store: lose it and already-installed users stop receiving auto-updates
  (they must reinstall once); the app itself keeps working.
- Generated with no password (set one and pass it via
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if you prefer). Regenerate with:

  ```bash
  bun x tauri signer generate -w apps/desktop/signing/nomifun-updater.key --password "" --ci -f
  # then paste the printed public key into tauri.conf.json → plugins.updater.pubkey
  ```

Updater signing is **separate** from OS code signing (Apple Developer ID /
Windows Authenticode). See `apps/desktop/signing/README.md`. macOS note: an
auto-updated `.app.tar.gz` should be signed + notarized, or Gatekeeper may block
the swapped bundle on other machines — that's the code-signing concern, not the
updater signature.

## Building signed update artifacts

`createUpdaterArtifacts` makes Tauri emit a `.sig` next to each updatable bundle.
The repo ships a tiny overlay config `apps/desktop/tauri.updater.conf.json`
(`{"bundle":{"createUpdaterArtifacts":true}}`) that you layer on with `--config`.
Pass it as a **file path**, not inline JSON: Windows PowerShell 5.1 strips the
double quotes from inline `--config '{...}'`, producing invalid JSON — a file path
has no quotes and works on every shell.

Set the private key **content** (not the path) in the environment first:

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""   # empty for the default key

# Windows (NSIS .exe + .sig):
bun run build:win --config apps/desktop/tauri.updater.conf.json

# macOS (Universal .app.tar.gz + .sig — one artifact serves both darwin chips):
bun run build:mac --config apps/desktop/tauri.updater.conf.json

# Linux (AppImage + .sig for updater; deb/rpm remain manual installers):
bun run build:linux --config apps/desktop/tauri.updater.conf.json
```

You **cannot cross-compile**: build Windows on Windows, macOS on macOS, Linux on
Linux. Each platform's artifacts land under `target/**/release/bundle/`. The
updater private key is gitignored, so on a fresh build machine (e.g. your Windows
box) copy it to `apps/desktop/signing/nomifun-updater.key` from your key store
first — it must match the `pubkey` embedded in `tauri.conf.json` (keyID
`F3AA272E60AA7952`), or installed clients silently reject the update.

Windows note: the command above signs the updater package with the Tauri updater
key, but it does **not** Authenticode-sign the Windows installer. That is enough
for updater verification, but manual downloads can still show SmartScreen /
unknown-publisher warnings. When a Windows certificate is available, set
`WINDOWS_CERTIFICATE_THUMBPRINT` and run (the `--signed` cert injection still uses
inline JSON, so run it under PowerShell 7+):

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content apps/desktop/signing/nomifun-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
$env:WINDOWS_CERTIFICATE_THUMBPRINT = "A1B2C3..."
bun run build:win --signed --config apps/desktop/tauri.updater.conf.json
```

## Generating the GitHub fallback `latest.json`

`bun run make:latest` scans `target/` for this machine's updater artifacts + their
`.sig` files, derives the `<os>-<arch>` platform keys, and **merges** them into
`apps/desktop/updater/latest.json` (preserving entries built on other machines).
This file serves GitHub fallback clients and is also the local source of truth
used by the CrabNebula upload helper:

```bash
bun run make:latest                     # version from Cargo.toml, notes from CHANGELOG
bun run make:latest --collect           # also copy artifacts + .sig + latest.json → dist/desktop/
bun run make:latest --version 0.1.11 --notes "..."   # overrides
```

Run it once per build machine; carry `latest.json` between them (or commit it) so
the merged manifest ends up complete. The report prints which platform keys are
filled and which are still missing — **a missing `<os>-<arch>` entry means those
users silently get no update**, so make sure every platform you ship has one.

If one platform produces several signed bundle types, `make:latest` uses the
updater package for the manifest and still leaves the other installers available
for upload. On Linux, the updater entry prefers `.AppImage`; `.deb` and `.rpm`
are manual download assets.

## CrabNebula Cloud

Create the CrabNebula application and API key once, then place `CN_API_KEY` and
optionally `CN_APP` in the gitignored
`apps/desktop/signing/.env.release`. Do **not** run `cn bootstrap` against this
existing application: it may replace the updater key/configuration, which would
strand already-installed clients.

The repository helper wraps the official `cn` CLI:

```bash
# After choosing/bumping the version, create one hidden draft and record its ID.
bun run release:cloud -- draft --version <version> --notes-file notes.md

# Run on each platform after build + make:latest. Pass the same release ID.
bun run release:cloud -- upload --release-id <release-id>

# Publish only after every shipped platform is present.
bun run release:cloud -- publish --release-id <release-id>
bun run release:cloud -- verify
```

`upload` only accepts a local updater artifact when its adjacent `.sig` exactly
matches the signature already merged into `latest.json`. Windows NSIS and Linux
AppImage packages also become public manual downloads; DMG/DEB/RPM files found
in `dist/desktop/` are uploaded as manual installers. The helper rejects a
stale `latest.json` whose version does not match the release version.

## Releasing

1. Create one CrabNebula release draft and share its release ID between build
   machines.
2. Build signed updater artifacts on each platform.
3. Run `bun run make:latest` on each, merging into one `latest.json`.
4. Upload each platform to the hidden CrabNebula draft with
   `bun run release:cloud -- upload --release-id <id>`.
5. After all shipped platforms are present, publish CrabNebula and run
   `bun run release:cloud -- verify`.
6. Create/update the matching GitHub Release and upload **all** manual
   installers, updater packages, updater `.sig` files, and `latest.json`. For
   macOS this means both:
   - `dist/desktop/NomiFun_<version>_universal.dmg` for manual install.
   - `target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz`
     plus `NomiFun.app.tar.gz.sig` for auto-update.
   For Windows, the updater `.exe` is also the normal manual installer; upload
   any `.msi` only if the build generated one.
7. Verify the GitHub `latest.json` version, platform set, and signatures match
   CrabNebula.

The first build containing the CrabNebula endpoint must still be published to
GitHub. Existing installations have the old GitHub-only endpoint embedded, so
they need one GitHub update (or one manual CrabNebula install) before subsequent
checks use CrabNebula.

## Safety checklist

- Private updater key stored only in your release secret store; never committed.
- `plugins.updater.pubkey` matches the private key used to sign.
- CrabNebula is fully uploaded and published before announcing the GitHub
  release to users.
- CrabNebula and GitHub contain the same version, artifact bytes, and signatures.
- `plugins.updater.endpoints` contains HTTPS CrabNebula primary + GitHub fallback.
- Every shipped `<os>-<arch>` has a `latest.json` entry with the exact `.sig` content.
- OS code signing / notarization handled separately per platform.
