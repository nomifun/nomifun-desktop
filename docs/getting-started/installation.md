# Installation

NomiFun runs in a Tauri desktop shell or as an independent Web host. Both share the backend and desktop-class UI, with a minimum supported viewport of 880×600. This page covers source prerequisites and first launch; persistent hosting configuration lives in [Web Server Deployment](../guides/web-server-deployment.md).

## Prerequisites

Building from source requires:

| Tool or platform | Requirement |
| --- | --- |
| Rust | Stable toolchain supporting the workspace's edition 2024 |
| Bun | ≥1.3.13 for workspace dependencies and frontend builds |
| Git, CMake | Source checkout and native dependencies |
| Windows | MSVC build tools and WebView2 |
| macOS | Xcode Command Line Tools; current desktop bundles require macOS 14.0+ |
| Linux | C/C++ toolchain, pkg-config, Perl and the development packages below |

Tauri CLI is a repository development dependency; no global installation is needed. Typical Debian/Ubuntu source-build packages:

```bash
sudo apt install build-essential cmake pkg-config git perl \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  libpipewire-0.3-dev libgbm-dev libclang-dev
```

Linux AppImage packaging also needs `librsvg2-dev` and `xdg-utils`. `libgbm-dev` supplies the link-time `libgbm.so`; PipeWire development files support the capture stack, and `libclang-dev` supports binding generation. Initial builds download Rust, JS and some native components. See [Development](../contributing/development.md) for network troubleshooting.

User commands may additionally need ripgrep, Node/npm, Python/uv or third-party CLIs on the host. Install what the selected tools require; importing MCP configuration does not install their runtimes.

## Get the source

```bash
git clone <your-fork-or-mirror>/nomifun-desktop.git
cd nomifun-desktop
bun install
```

Run the remaining commands from the repository root.

## Desktop development and bundles

```bash
bun run dev
```

The command starts Vite, builds and opens the Tauri shell. The backend starts in-process on an available localhost port. The WebView supplies local trust credentials and does not require login.

![NomiFun desktop workstation](../images/desktop-01-main-window.png)

Build a release bundle for the current platform:

```bash
bun run build
```

Bundles appear under `target/release/bundle/`. Distribution signing, Updater artifacts, platform differences and release validation are maintained in [Building and Packaging](../contributing/building-and-packaging.md). See [Desktop Application](../guides/desktop-app.md) for everyday operation and troubleshooting.

## Web host

For a first run:

```bash
bun run serve:web
```

The script builds the UI and runs `nomifun-web` at `http://127.0.0.1:8787`. Create an administrator on first use, then log in. Provision the administrator before opening a non-local listener; see [Web Server Deployment](../guides/web-server-deployment.md).

For an independent binary:

```bash
bun run build:ui
cargo build --release -p nomifun-web --features static-webui
./target/release/nomifun-web --dist ui/dist --data-dir /absolute/path/to/nomifun-data
```

On Windows use the corresponding `nomifun-web.exe` path and a local absolute data path. `--dist` must point to the built UI. CLI flags, Docker/Compose, TLS, systemd and robot networking are maintained in the [Deployment guide](../guides/web-server-deployment.md).

## Data and verification

Stable data roots default to `%LOCALAPPDATA%\NomiFun` on Windows, `~/Library/Application Support/NomiFun` on macOS, and `$XDG_DATA_HOME/NomiFun` on Linux. Development scripts use a separate root. Override it with `NOMIFUN_DATA_DIR` or the Web host's `--data-dir`. Only one backend can run against a root at a time.

Confirm the desktop workstation opens. On Web, check login, the home page and API reachability:

```bash
curl http://127.0.0.1:8787/api/auth/status
```

A successful launch does not configure a model. Follow [Quick Start](quick-start.md) to add a provider and send the first message. See [Data and Storage](../architecture/data-and-storage.md) for compatibility and backups.
