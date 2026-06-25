# NomiFun Desktop Updater

This directory documents the Tauri updater wiring for `nomifun-desktop`.

The updater is a scaffold, not a ready production release channel. The plugin
is installed and the desktop command exists, but public releases still require
release-owner credentials, a real HTTPS endpoint, and a signing key pair that
is controlled outside the repository.

## Current State

- Rust plugin: `apps/desktop/Cargo.toml` includes `tauri-plugin-updater`.
- Frontend package: `ui/package.json` includes `@tauri-apps/plugin-updater`.
- Tauri config: `apps/desktop/tauri.conf.json` contains
  `plugins.updater.endpoints` and `plugins.updater.pubkey`.
- Desktop command: `check_for_updates` is exposed through Tauri invoke and
  returns a version string or `null`.
- Build script: `bun run build:updater` produces updater signatures (`.sig`)
  next to release installers when the signing environment variables are set.

The configured endpoint is a placeholder. The configured pubkey must be treated
as a development value unless the release owner has explicitly replaced it and
stored the matching private key in release infrastructure.

## Required Before Public Release

1. Generate an updater signing key pair owned by the release owner:

   ```bash
   bun x tauri signer generate -w <private-key-output-path>
   ```

2. Put the printed public key in `plugins.updater.pubkey` in
   `apps/desktop/tauri.conf.json`.
3. Store the private key and password in CI or another release-secret store.
   Never commit them.
4. Replace `plugins.updater.endpoints` with a real HTTPS URL that serves
   `latest.json`.
5. Build release artifacts with updater signing enabled.
6. Upload installers and signatures to your release hosting.
7. Publish `latest.json` with the correct URLs and signatures.

Updater signing is separate from OS code signing. macOS Developer ID signing and
notarization are documented in `apps/desktop/signing/README.md`. Windows
SmartScreen reputation still requires an external code-signing certificate and
publisher reputation.

## Build A Signed Update Artifact

Set the private key content, not a path:

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat <private-key-output-path>)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<private-key-password-or-empty>"
bun run build:updater
```

Artifacts land under `target/release/bundle/`. Each installer that supports
updates gets a sibling `.sig` file, for example:

```text
target/release/bundle/nsis/NomiFun_0.1.1_x64-setup.exe
target/release/bundle/nsis/NomiFun_0.1.1_x64-setup.exe.sig
```

Copy the full `.sig` file content into the matching platform entry in
`latest.json`.

## `latest.json`

The Tauri updater expects a manifest similar to:

```json
{
  "version": "0.1.1",
  "notes": "Release notes for users.",
  "pub_date": "2026-06-24T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents-of-.sig>",
      "url": "https://example.com/downloads/NomiFun_0.1.1_x64-setup.exe"
    },
    "darwin-aarch64": {
      "signature": "<contents-of-.sig>",
      "url": "https://example.com/downloads/NomiFun_0.1.1_aarch64.dmg"
    },
    "linux-x86_64": {
      "signature": "<contents-of-.sig>",
      "url": "https://example.com/downloads/nomifun_0.1.1_amd64.AppImage"
    }
  }
}
```

Common platform keys are:

- `windows-x86_64`
- `darwin-x86_64`
- `darwin-aarch64`
- `linux-x86_64`

## Client Behavior

The current command only checks for an available update:

```ts
import { invoke } from "@tauri-apps/api/core";

const newVersion = await invoke<string | null>("check_for_updates");
```

Download/install UX can be implemented either in the frontend with
`@tauri-apps/plugin-updater` (`check`, `downloadAndInstall`) or in the Rust
command by calling `download_and_install` after a user confirms the action.

## Safety Checklist

- Private updater key is stored only in release secrets.
- `plugins.updater.pubkey` matches the private key used by CI.
- `plugins.updater.endpoints` points to HTTPS.
- `latest.json` contains real installer URLs and exact signature contents.
- OS code signing and notarization are handled separately for each platform.
