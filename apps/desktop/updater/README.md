# Desktop updater

[简体中文](README.zh-CN.md) · [Release workflow](../../../RELEASING.md)

This page owns updater behavior and artifact integrity. Build, upload and publish steps live
in the release workflow; do not maintain a second release checklist here.

## Runtime contract

- [tauri.conf.json](../tauri.conf.json) owns the ordered endpoints and public key: CrabNebula
  is primary and GitHub Releases is fallback. Metadata checks use an eight-second timeout.
- The [renderer adapter](../../../ui/src/common/adapter/tauriUpdater.ts) checks metadata.
  Native commands download, verify and retain the exact selected version; installation
  consumes those retained bytes without a second download.
- A non-GitHub package download or verification failure may retry the **same version** from
  GitHub. It cannot silently select another version. A native slot already holding an update
  remains authoritative when the modal reopens.
- Renderer capabilities allow metadata checking and relaunch, not the raw updater plugin's
  download/install operations. The [native install context](../src/updater_install_context.rs)
  and commands own installation preflight and failure recovery.
- On macOS the update check also reads the native install context. A running bundle that
  cannot be replaced in place (disk image or external volume, App Translocation, a volume
  other than the temporary directory's, or an unreadable location) is reported with its
  specific cause and running path, and the renderer offers manual download instead of
  downloading a package it cannot install. Installation repeats the same preflight and
  fails closed.
- A failure after installer admission retains the native slot and reports manual recovery.
  Correcting an already published bad archive requires a newly signed artifact and matching
  manifests; changing only the application source cannot repair downloaded bytes.

## Signing and artifacts

The updater key is separate from [OS code signing](../signing/README.md). Keep the existing
private key in the release secret store; `TAURI_SIGNING_PRIVATE_KEY` receives its **content**,
with `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` when applicable. It must match
`plugins.updater.pubkey`. Key rotation is not a normal release step, and losing the signing
key prevents existing installations from trusting future packages.

The [updater overlay](../tauri.updater.conf.json) enables `createUpdaterArtifacts`. Pass the
configuration as a file path. Follow [building and packaging](../../../docs/contributing/building-and-packaging.md)
for platform commands. A package and its adjacent `.sig` form one artifact identity.

`bun run make:latest` merges local artifacts into [latest.json](latest.json), preserving
other platform entries. Every shipped OS/architecture needs its exact signed package entry.
On Linux, AppImage is the updater package; DEB/RPM remain manual installers. CrabNebula and
GitHub must publish the same version, bytes and signatures. The
[cloud release helper](../../../scripts/crabnebula-release.mjs) verifies upload inputs against
this manifest; do not bootstrap a new cloud configuration over the existing signing identity.

## macOS archive contract

Create the updater archive from the final signed/notarized App, with `COPYFILE_DISABLE=1`,
no extended attributes and no AppleDouble entries (`._*`, `__MACOSX`). Every entry belongs to
one `NomiFun.app/` root because installation strips one path component. Extracted file bytes,
modes and symlinks must match the final App, including native libraries and signatures.

The [bundle verifier](../../../scripts/lib/macos-app-bundle.mjs) checks raw entries with
libarchive's `!mac-ext` handling and verifies the extracted layout. Relevant regression:

```bash
bun test ./scripts/lib/macos-app-bundle.test.mjs
```
