# Building and Packaging

[简体中文](building-and-packaging.zh.md)

Use [Development](development.md) for local loops and
[RELEASING.md](../../RELEASING.md) for public release operations.
Build desktop packages on their target OS.

## Build targets

| Target | Command | Output |
| --- | --- | --- |
| Shared SPA | `bun run build:ui` | `ui/dist/` |
| Web server | `cargo build --release -p nomifun-web` | `target/release/nomifun-web` |
| Desktop | `bun run build` | Platform bundles under `target/` |
| macOS arm64 / Intel | `bun run build:mac` / `bun run build:mac intel` | Native architecture packages |
| Windows | `bun run build:win` | NSIS/MSI outputs |
| Linux | `bun run build:linux` | AppImage/deb/rpm outputs |
| Updater | `bun run build:updater` | Updatable package and `.sig` |

The desktop config is
[tauri.conf.json](../../apps/desktop/tauri.conf.json): product identity,
platform requirements and `frontendDist` live there. The workspace version
comes from [Cargo.toml](../../Cargo.toml).

Linux `build` and `build:linux` build and freeze the system runner for the selected
architecture before bundling. Deb/rpm install the runner and its polkit action at
fixed system paths; AppImage carries an explicit installer because mounting an
image cannot install host authentication components. `build:fast` stages a debug
runner and prints its manual installation command. Development startup does not
change system files. Installation, upgrade/removal, authentication requirements
and checks are in the [Linux system execution guide](../../apps/desktop/linux-system-execution/README.md).

Linux Computer also links GStreamer core/app/video (system version 1.14 or newer).
The build preflight checks all three pkg-config modules. Debian/Ubuntu build
packages are `libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev`; runtime
packages are `gstreamer1.0-pipewire`, `gstreamer1.0-plugins-base` and
`gstreamer1.0-plugins-good`. The RPM dependency names follow Fedora's
`pipewire-gstreamer`, `gstreamer1-plugins-base` and `gstreamer1-plugins-good`.
The native packages declare these runtime-loaded plugins and `xdg-desktop-portal`;
ELF library scanning cannot discover the required `pipewiresrc`, `videoconvert`,
`videoflip` and `appsink` factories by itself. Wayland additionally needs the
logged-in desktop's working PipeWire service and a portal backend implementing
the requested ScreenCast/RemoteDesktop interfaces and native consent.

AppImage still requires those matching host services and GStreamer plugins.
Its actual linked libraries/plugin search path must be inspected in the finished
image with the target desktop; the bundler's `bundleMediaFramework` option alone
does not prove that `pipewiresrc` or a host portal backend is present. AppImage
Wayland operation remains unverified until that packaged runtime is exercised.

## Web deployment artifacts

Build the SPA before running a non-embedded Web host:

```bash
bun run build:ui
cargo build --release -p nomifun-web
target/release/nomifun-web --host 127.0.0.1 --port 8787 --dist ui/dist
```

The host needs a writable data directory. Authentication is enabled by default;
first-run setup creates the owner unless credentials were pre-seeded.
For containers, run `docker compose up -d --build` or use the published image.
Docker, reverse proxy, volume and native systemd instructions are maintained in
[Web server deployment](../guides/web-server-deployment.md) and
[packaging/linux](../../packaging/linux/README.md).

The desktop package and its production WebUI share the same embedded frontend
assets. An explicit `NOMIFUN_WEBUI_DIST` override must match the build identity.

## Desktop runtime components

Release binaries embed OfficeCLI. Backend startup does not download Node/Bun. Actual launches automatically prepare a missing required runtime, preferring usable user tools and verified caches. Node supply contains the complete official Node/npm/npx distribution. Agent, Terminal, MCP and hosted application processes share the per-launch user tool environment. See [external tools](../architecture/external-tools.md)
for admission, cache and offline behavior.

Desktop installers do not bundle NomiRelay's `nfagent`. Connect, restore and restart reuse an approved local cache or automatically prepare a missing/current artifact. The fixed immutable URL and
SHA-256 for each platform are in
[nfagent-runtime.json](../../apps/desktop/nfagent-runtime.json).
Downloads are bounded, staged, verified and atomically published.

For offline diagnostics, `NOMIFUN_NFAGENT_PATH` may name an existing absolute
file; it must still match the current platform's approved SHA-256.
Publish every new manifest asset before distributing a referencing Desktop
build; the update procedure is in [RELEASING.md](../../RELEASING.md).

## Signing and updater packages

Updater signatures and OS code signing serve separate purposes. Supply the
existing updater private key from the release secret store; do not generate a
replacement for an established installation.

For macOS Developer ID signing/notarization:

```bash
cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing
bun run build:signed
```

Fill local signing values before building. The real environment files and keys
stay untracked. See [signing setup](../../apps/desktop/signing/README.md)
and [updater operations](../../apps/desktop/updater/README.md) for key loading,
Windows Authenticode, package verification and manifest generation.

macOS supports native arm64 and Intel packages with system WKWebView; Universal
packages are rejected. The embedded Browser target is macOS 14+. The final
signed/notarized App must precede DMG and updater byte generation. Package
verification checks extracted bytes, modes and symlinks and rejects bundled
browser runtimes. Intel ONNX resources and licenses remain required.

`bun run build:updater` uses the committed
[updater overlay](../../apps/desktop/tauri.updater.conf.json).
CrabNebula Cloud is the primary distribution source and GitHub Releases the
fallback. See the release runbook before publishing.

## Verification

Run targeted packaging tests and build on the destination OS. Smoke-test launch,
local authentication and the affected runtime components. For public macOS
artifacts, verify `codesign`, `spctl` and `xcrun stapler`. For Web deployments,
verify first-run setup/login, `/health` and WebSocket connectivity through the
actual reverse proxy. Record platform checks that could not run.
