# Releasing NomiFun

中文发版手册见 `RELEASING.zh-CN.md`.

This checklist is for maintainers preparing a public release.

## Versioning (single source of truth)

The release version lives in **one** place: the root `Cargo.toml`
`[workspace.package].version`. The backend's `CARGO_PKG_VERSION` / `app_version`
follows it, and `apps/desktop/tauri.conf.json` inherits it (it has no `version`
field of its own — Tauri reads it from the workspace), so the installer filename
and updater version stay in sync automatically.

Bump everything with one command:

```bash
bun run bump 1.2.3            # writes the version + syncs Cargo.lock + package.json/ui
bun run bump 1.2.3 --tag      # also: git commit + git tag v1.2.3 (needs a clean tree)
```

Tags use the `vX.Y.Z` form. The decorative `package.json` / `ui/package.json`
versions are kept in sync by the script but are not read by any build.

## Releasing the `nfagent` runtime

Desktop installers no longer bundle `nfagent`. On the first NomiRelay pairing,
restore, or agent restart, Desktop downloads the target-platform asset from the
fixed URL in
[`apps/desktop/nfagent-runtime.json`](apps/desktop/nfagent-runtime.json) and
checks its SHA-256. **Publish the `nfagent` assets before distributing any
Desktop release that references that manifest.**

The current manifest corresponds to:

- runtime version: `0.1.0`
- GitHub Release tag: `nfagent-v0.1.0`
- source repository: `nomifun/nomifun-net-infra`
- source commit: `3747c420f6895977edcac3614f5924d7fbdd367e`
- local upload directory: `dist/nfagent-runtime/` (gitignored)

Upload these six fixed binary names plus `SHA256SUMS` for manual verification:

```text
nfagent-windows-amd64.exe
nfagent-windows-arm64.exe
nfagent-darwin-amd64
nfagent-darwin-arm64
nfagent-linux-amd64
nfagent-linux-arm64
SHA256SUMS
```

### Rebuilding

For a future runtime update, check out the intended clean commit in the sibling
`../nomifun-net-infra` repository and run that repository's required local
gate. GitHub Actions are forbidden there, so builds and checks are local:

```bash
cd ../nomifun-net-infra
VERSION=<new-version>
COMMIT=$(git rev-parse --short HEAD)
make check
make dist VERSION="$VERSION" COMMIT="$COMMIT"
```

`make dist` emits more NomiRelay targets; the Desktop manifest currently uses
only the six `nfagent` targets listed above. Copy those files into this
repository's `dist/nfagent-runtime/`, regenerate `SHA256SUMS`, and put each new
full SHA-256 into `nfagent-runtime.json`. Never change only the URL while
retaining an old hash.

### Manual browser upload to GitHub

The maintainer should perform the GitHub operation in a browser:

1. Open `https://github.com/nomifun/nomifun-net-infra/releases/new`.
2. Under **Choose a tag**, enter `nfagent-v0.1.0`, create the new tag, and target
   commit `3747c420f6895977edcac3614f5924d7fbdd367e`.
3. Use a title such as `nfagent v0.1.0`; record the source commit and
   `SHA256SUMS` in the notes.
4. Drag the seven files listed above from `dist/nfagent-runtime/` into the asset
   area. Preserve every filename exactly.
5. Publish a real Release; a draft is not downloadable by normal Desktop users.
6. Confirm every asset exists and has this fixed URL shape:

   ```text
   https://github.com/nomifun/nomifun-net-infra/releases/download/nfagent-v0.1.0/<filename>
   ```

7. Before distributing Desktop, download at least the current-platform asset
   back from the Release, verify it against `SHA256SUMS`, and run:

   ```powershell
   .\nfagent-windows-amd64.exe -version
   ```

Do not replace binaries under an already published tag. Any byte change requires
a new runtime version and tag, new SHA-256 values, an updated
`nfagent-runtime.json`, and then a new Desktop release. Manifest URLs must not
use `latest`, query strings, or replaceable generic filenames.

## Before Tagging

1. Update `CHANGELOG.md`.
2. Run the documented verification commands for the changed surface.
3. Confirm `docs/`, `README.md`, `STATUS.md`, and packaging guides match the
   release behavior.
4. Confirm no private keys, local paths, proprietary assets, or internal-only
   roadmap claims are included.
5. Confirm third-party licenses and attributions are current.

## Desktop Release

Desktop releases have two asset groups:

- **Manual installers**: files users download directly, such as macOS `.dmg`,
  Windows `.exe` / `.msi`, and Linux `.AppImage` / `.deb` / `.rpm`.
- **Updater assets**: the Tauri updater package, its `.sig`, and the merged
  `latest.json`. The updater package may also be a manual installer on Windows,
  but macOS still needs a separate `.dmg` for manual install.

Updater signing and OS code signing are separate. The updater private key proves
that an update package is ours; OS code signing controls Gatekeeper / SmartScreen
trust for people launching a manually downloaded app. You **cannot cross-compile**
reliable desktop installers — build each platform on its own machine.

The updater private key (`apps/desktop/signing/nomifun-updater.key`) is gitignored
and lives only in your key store. On a fresh build machine (e.g. a Windows box that
never generated it) copy it there before building; it must match the `pubkey`
embedded in `tauri.conf.json` (keyID `F3AA272E60AA7952`), or installed clients
silently reject the update. The `createUpdaterArtifacts` flag is supplied as a
committed overlay file `apps/desktop/tauri.updater.conf.json` passed with
`--config <path>` (not inline JSON — Windows PowerShell 5.1 mangles inline
`--config '{...}'`).

### Standard Release Runbook

Use this order for every desktop release.

1. **Pick and bump the version.**

   ```bash
   VERSION=1.2.3
   bun run bump "$VERSION"
   ```

2. **Build macOS on a Mac.** The command below emits both the manual `.dmg` and
   the updater `.app.tar.gz` + `.sig`.

   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   bun run build:mac --config apps/desktop/tauri.updater.conf.json
   bun run make:latest
   ```

   For public macOS distribution, configure `apps/desktop/signing/.env.signing`
   and use `--signed` so the `.app` / `.dmg` are Developer ID signed and
   notarized:

   ```bash
   bun run build:mac --signed --config apps/desktop/tauri.updater.conf.json
   ```

3. **Build Windows on a Windows machine.** Use the same updater private key.
   If you do not yet have Authenticode signing, omit `--signed`; the Tauri updater
   signature still works, but the manual installer can show Windows SmartScreen /
   unknown-publisher warnings.

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content apps/desktop/signing/nomifun-updater.key -Raw
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
   bun run build:win --config apps/desktop/tauri.updater.conf.json
   bun run make:latest
   ```

   Once a Windows code-signing certificate is available, import it into the
   current user's certificate store and build with Authenticode signing:

   ```powershell
   $env:WINDOWS_CERTIFICATE_THUMBPRINT = "A1B2C3..."
   bun run build:win --signed --config apps/desktop/tauri.updater.conf.json
   ```

4. **Build Linux on Linux** if that platform is part of the release.

   Prepare the native Linux packaging dependencies first:

   ```bash
   sudo apt-get install -y pkg-config libgbm-dev libayatana-appindicator3-dev librsvg2-dev
   ```

   `build:linux` preflights the `gbm`, AppIndicator, and `librsvg-2.0`
   `pkg-config` entries and sets `APPIMAGE_EXTRACT_AND_RUN=1` so linuxdeploy can
   run on builders without FUSE2.

   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="$(cat apps/desktop/signing/nomifun-updater.key)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   bun run build:linux --config apps/desktop/tauri.updater.conf.json
   bun run make:latest
   ```

   Linux uploads include `.AppImage`, `.deb`, and `.rpm`, but the updater
   manifest should point at the `.AppImage`. `make:latest` applies that
   preference explicitly when several signed Linux bundles exist.

5. **Merge `latest.json` before publishing.** `bun run make:latest` preserves
   existing real platform entries and fills the entries for the current build
   machine. If platforms are built on different machines, carry the newest
   `apps/desktop/updater/latest.json` to the next machine before running
   `make:latest`, or replace the Release asset later with `--clobber`.

6. **Commit and tag the release state.**

   ```bash
   git add Cargo.toml Cargo.lock package.json ui/package.json apps/desktop/updater/latest.json
   git commit -m "chore(release): v$VERSION"
   git tag "v$VERSION"
   git push origin main "v$VERSION"
   ```

7. **Create or update the GitHub Release.** Upload updater assets, `latest.json`,
   and manual installers.

   ```bash
   gh release create "v$VERSION" \
     target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz \
     target/universal-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz.sig \
     dist/desktop/NomiFun_${VERSION}_universal.dmg \
     apps/desktop/updater/latest.json \
     --title "v$VERSION" \
     --notes "Release notes"
   ```

   If the Release already exists, upload more platform assets instead:

   ```bash
   gh release upload "v$VERSION" <new-assets...>
   gh release upload "v$VERSION" apps/desktop/updater/latest.json --clobber
   ```

   On Windows, upload the files printed by `bun run make:latest`; those are the
   updater package, its `.sig`, and the current `latest.json`. Also upload any
   manual-only installer from `dist/desktop/` that was not already uploaded, such
   as an `.msi` if one was generated.

8. **Verify the published Release.**

   ```bash
   gh release view "v$VERSION" --json tagName,assets,url
   curl -fsSL https://github.com/nomifun/nomifun-tauri/releases/latest/download/latest.json
   ```

   Confirm the downloaded manifest version is `VERSION`, every shipped platform
   has a `platforms[...]` entry, and all URLs point to the same `v$VERSION`
   Release.

### Releasing macOS (append to an existing release, or macOS-first)

The one-click script auto-detects two scenarios:

- **APPEND** — a Release for this version already exists (Windows may have gone
  first): add the macOS assets and merge the `darwin-x86_64` / `darwin-aarch64`
  entries into `latest.json`.
- **CREATE** — no Release exists yet (macOS goes first): create the tag and the
  Release (with release notes) and upload the macOS assets; `-Version` can bump
  the version number as part of the same command.

**One-click (recommended).** After one-time setup, repeat with a single command:

1. Copy the updater private key `apps/desktop/signing/nomifun-updater.key` from
   your key store (gitignored; must match the `pubkey` in `tauri.conf.json`).
2. Configure `apps/desktop/signing/.env.signing` for Developer ID signing and
   notarization.
3. Either run `gh auth login`, or copy
   `apps/desktop/signing/.env.release.example` to `.env.release` (gitignored) and
   set `GH_TOKEN=...`.

   ```bash
   git pull

   # APPEND (Windows already released; add macOS):
   bun run release:mac

   # CREATE (macOS first): -Version bumps; -NotesFile/-Notes are required for a first release
   bun run release:mac -Version 0.1.14 -NotesFile notes.md
   bun run release:mac -Version 0.1.14 -Notes "- Fixes"

   # Switches:
   bun run release:mac -DryRun     # read-only preflight + plan (incl. detected mode); no pull/bump/build/upload/push
   bun run release:mac -NoPush     # APPEND: still uploads but no git push; CREATE: local bump/build only, no Release
   bun run release:mac -SkipPull   # skip git pull during real execution
   ```

**Version** is the single source `Cargo.toml [workspace.package].version`. APPEND
uses the current version. For CREATE, `-Version X.Y.Z` (when different) first runs
`bun run bump`, then commits as `nomifun`, tags, and creates the Release.

**Release notes are LLM-friendly**: passed on the command line (no CHANGELOG
coupling), and the **same text is used for both the GitHub Release body and the
`latest.json` `notes`**. Prefer `-NotesFile <md>` for multi-line notes. A first
release with no `-NotesFile`/`-Notes` aborts rather than creating an empty-notes
Release; in APPEND mode notes are optional.

The script loads `GH_TOKEN` from `.env.release` when present (or uses existing
`gh auth login`), loads the updater signing key, detects the mode, builds
`build:mac --signed --config apps/desktop/tauri.updater.conf.json`, validates
stapling / codesign / Gatekeeper status, merges the darwin entries via
`make:latest`, uploads (`--clobber` for APPEND, or `gh release create` for
CREATE), commits `latest.json` (plus the bump for CREATE) as author `nomifun`,
and verifies the updater endpoint. It aborts with a clear error on any failure.

### Releasing Windows (append to an existing release, or Windows-first)

The one-click script auto-detects two scenarios:

- **APPEND** — a Release for this version already exists (macOS usually went
  first): just add the Windows assets and merge the `windows-x86_64` entry into
  `latest.json`.
- **CREATE** — no Release exists yet (Windows goes first): create the tag and the
  Release (with release notes) and upload the Windows assets; `-Version` can bump
  the version number as part of the same command.

**One-click (recommended).** After a one-time setup, repeat with a single command:

1. Copy the updater private key `apps/desktop/signing/nomifun-updater.key` from
   your key store (gitignored; must match the `pubkey` in `tauri.conf.json`).
2. Copy `apps/desktop/signing/.env.release.example` to `.env.release` (gitignored)
   and set `GH_TOKEN=...` (a classic PAT with `repo`, or a fine-grained PAT with
   Contents: read/write on this repo).

   ```powershell
   git pull

   # APPEND (macOS already released; add Windows):
   bun run release:win

   # CREATE (Windows first): -Version bumps; -NotesFile/-Notes are required for a first release
   bun run release:win -Version 0.1.14 -NotesFile notes.md
   bun run release:win -Version 0.1.14 -Notes "- Fixes"

   # Switches:
   bun run release:win -DryRun     # read-only preflight + plan (incl. detected mode); no bump/build/upload/push
   bun run release:win -NoPush     # APPEND: still uploads but no git push; CREATE: local bump/build only, no Release
   bun run release:win -SkipPull   # skip git pull
   ```

**Version** is the single source `Cargo.toml [workspace.package].version`. APPEND
uses the current version. For CREATE, `-Version X.Y.Z` (when different) first runs
`bun run bump`, then commits as `nomifun`, tags, and creates the Release.

**Release notes are LLM-friendly**: passed on the command line (no CHANGELOG
coupling), and the **same text is used for both the GitHub Release body and the
`latest.json` `notes`**. Prefer `-NotesFile <md>` for multi-line notes (the script
routes them through a file to avoid PowerShell multi-line arg issues). A first
release with no `-NotesFile`/`-Notes` aborts rather than creating an empty-notes
Release; in APPEND mode notes are optional.

The script loads `GH_TOKEN` from `.env.release`, loads the signing key, detects the
mode, removes stale NSIS artifacts, builds updater artifacts, merges the
`windows-x86_64` entry via `make:latest`, uploads (`--clobber` for APPEND, or
`gh release create` for CREATE), commits `latest.json` (plus the bump for CREATE)
as author `nomifun`, and verifies the updater endpoint. It aborts with a clear
error on any failure.

**Manual steps** (equivalent to what the script runs internally):

1. Pull the release commit/tag and confirm the same version.

   ```powershell
   git pull
   git checkout main
   git describe --tags --exact-match  # should print v<version> only if you checked out the tag
   ```

   If you stay on `main`, confirm `Cargo.toml` still has the same version as the
   existing Release.

2. Build Windows updater artifacts with the same updater private key. The key is
   gitignored, so first copy `apps/desktop/signing/nomifun-updater.key` onto this
   Windows machine from your key store (it must match the `pubkey` in
   `tauri.conf.json`).

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content apps/desktop/signing/nomifun-updater.key -Raw
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
   bun run build:win --config apps/desktop/tauri.updater.conf.json
   bun run make:latest
   ```

3. Upload the Windows assets printed by `bun run make:latest`, then replace the
   Release `latest.json` so it includes both macOS and Windows platform entries.

   ```powershell
   gh release upload v<version> <windows-updater-package> <windows-updater-package>.sig
   gh release upload v<version> apps/desktop/updater/latest.json --clobber
   ```

   Upload any additional manual-only Windows installer from `dist/desktop/` if
   the build produced one and it was not already the updater package.

4. Commit and push the updated `latest.json` back to `main` so the repository
   matches the Release asset.

   ```powershell
   git add apps/desktop/updater/latest.json
   git commit -m "chore(release): add Windows assets to v<version>"
   git push origin main
   ```

Windows without Authenticode signing is acceptable for internal updater testing:
the Tauri updater `.sig` still protects the automatic update package. It is not
equivalent to a trusted public Windows installer. For public distribution, obtain
a Windows code-signing certificate, set `WINDOWS_CERTIFICATE_THUMBPRINT`, and
rerun `build:win --signed --config apps/desktop/tauri.updater.conf.json` (under
PowerShell 7+, since the cert injection uses inline JSON) before publishing the
Windows assets.

See:

- `apps/desktop/updater/README.md`  (full updater flow + signing keys)
- `apps/desktop/signing/README.md`  (macOS Developer ID / notarization)

## Server Release

1. Build `nomifun-web` and the SPA.
2. Build and smoke-test the Docker image.
3. Verify first-run admin setup and `NOMIFUN_ADMIN_PASSWORD` pre-seeding.
4. Verify `127.0.0.1` default binding and explicit `0.0.0.0` deployment docs.

## After Release

1. Create a GitHub release with notes from `CHANGELOG.md`.
2. Attach platform artifacts.
3. Update website/download links.
4. Watch issues for install, updater, and migration regressions.
