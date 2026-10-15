# Releasing NomiFun

[简体中文](RELEASING.zh-CN.md)

This is the maintainer release sequence. Build targets and local packaging are
documented in [building and packaging](docs/contributing/building-and-packaging.md);
signing and updater mechanics are maintained beside their owners:
[signing](apps/desktop/signing/README.md) and
[updater](apps/desktop/updater/README.md).

## Prepare

1. Start from the intended reviewed commit with a clean working tree.
2. Update `CHANGELOG.md` and prepare a release-notes file. Verify docs and
   install/deployment guides against the release behavior.
3. Run checks covering the changed surfaces and destination platform packaging.
   Confirm licenses, attributions and redistribution rights.
4. Retrieve the existing updater private key from the release secret store.
   It must match `plugins.updater.pubkey` in
   [tauri.conf.json](apps/desktop/tauri.conf.json).
   Keep credentials in the ignored `apps/desktop/signing/.env.release`;
   use the contributor's configured Git identity.

For a destructive data change, put its warning first in the exact release notes
sent to GitHub and the updater. State what is lost and what users must do before
upgrading. A changelog entry alone is insufficient: updater clients display the
manifest's `notes`, and fallback generation skips `Unreleased`.

The version source is `Cargo.toml [workspace.package].version`.
`bun run bump 1.2.3` synchronizes lock/package metadata; `--tag` also commits
and creates `v1.2.3`. Do not bump/tag twice when a release helper owns that step.

## Desktop release helpers

Run each helper on its native OS. It detects whether the version's GitHub
Release already exists: create a version/tag/Release when absent, otherwise
append the platform's assets and merge `latest.json`.

| Platform | Preview | Create a version | Append current version |
| --- | --- | --- | --- |
| macOS | `bun run release:mac -DryRun` | `bun run release:mac -Version 1.2.3 -NotesFile notes.md` | `bun run release:mac` |
| Windows | `bun run release:win -DryRun -SkipPull` | `bun run release:win -Version 1.2.3 -NotesFile notes.md` | `bun run release:win` |
| Linux | `bun run release:linux -DryRun` | `bun run release:linux -Version 1.2.3 -NotesFile notes.md` | `bun run release:linux` |

First creation requires release notes. Prefer `-NotesFile` for multiline text.
macOS defaults to Apple Silicon; `-Architecture intel` appends the independent
Intel build. Build Windows with the intended Authenticode configuration and
macOS with Developer ID/notarization configured.

On Windows, pair `-DryRun` with `-SkipPull` for a read-only preview:
`-DryRun` alone still runs `git pull --ff-only` before the preview branch.
The macOS/Linux previews skip pull. `-NoPush` is **not** a universal offline/dry-run
switch: in append mode it still uploads assets. `-SkipPull` skips the helper's
pull. Read the platform script before using these switches:
[macOS](scripts/release-mac.sh), [Windows](scripts/release-win.ps1),
[Linux](scripts/release-linux.sh).

## Distribution and manifests

CrabNebula Cloud is the primary updater/download source; GitHub Releases is
the fallback and archive. Keep the same version and byte-identical signed
artifacts on both sources. Keep a Cloud release draft until all intended
platforms have uploaded.

```bash
bun run release:cloud -- draft --version 1.2.3 --notes-file notes.md
bun run release:cloud -- upload --release-id <release-id>
bun run release:cloud -- publish --release-id <release-id>
bun run release:cloud -- verify
```

Run upload on each platform as needed. For manual builds,
`bun run make:latest --collect` scans signed artifacts and merges platform
entries into `apps/desktop/updater/latest.json`, collecting assets under
`dist/desktop/`. Carry and merge the manifest across build machines without
dropping existing platform entries.

Verify before publication:

- Every intended OS/architecture has the correct version, URL and signature.
- Manual installers and updater packages are both present. macOS uses a DMG for
  manual installation and `.app.tar.gz` for updates; Linux updater entries use
  AppImage while deb/rpm remain manual downloads.
- The updater archive comes from the final signed/notarized App and passes its
  extraction verifier.
- A published URL returns the exact signed bytes. Test update check, download,
  install and relaunch from the preceding supported release.
- Destructive-change warnings are visible in the updater notes.

## Updating the NomiRelay connection component

Desktop installers do not bundle `nfagent`. Restore/restart reuse valid local
bytes; downloading a missing or changed component requires the user's explicit
choice. Approved versions, immutable URLs and full SHA-256 values are declared
in [nfagent-runtime.json](apps/desktop/nfagent-runtime.json).

For a component update:

1. Build the intended clean commit in `nomifun-net-infra` using that
   repository's current validation/release instructions.
2. Publish an immutable versioned Release with the six manifest targets:
   `nfagent-{windows,darwin,linux}-{amd64,arm64}`
   (Windows filenames end in `.exe`), plus `SHA256SUMS`.
3. Update manifest version, URLs and full hashes together. Never replace bytes
   under an already published tag; changed bytes require a new version/tag.
4. Confirm every manifest URL is publicly downloadable and does not use
   `latest`, query strings or fragments. Download and hash-check at least
   the current platform's asset, and run its `-version` command.
5. Only then distribute a Desktop release referencing the new manifest.

## Server release and follow-up

Build/push the Web image using the maintained
[deployment instructions](docs/guides/web-server-deployment.md). Publish matching
version tags and use explicit tags/digests where reproducibility matters.

After publishing, verify download links, first-run setup, login and WebSocket
connectivity; check the updater from an older installation on each released
platform. Record known limitations and announce only artifacts actually available.
