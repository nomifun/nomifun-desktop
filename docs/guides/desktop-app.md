# Desktop application

NomiFun uses a Tauri shell to start the shared Rust backend locally and display the workstation. Its WebView uses a local trust credential generated at each launch, so the desktop window needs no login. Other devices connect through [Connections & Devices](webui-remote-access.md).

![NomiFun desktop window](../images/desktop-01-main-window.png)

## Install and launch

See [Installation](../getting-started/installation.md) for source prerequisites and commands. Release bundles, code signing and update artifacts are covered by [Building and Packaging](../contributing/building-and-packaging.md).

Packaged applications require the system WebView: WebView2 on Windows, WKWebView on macOS, and WebKitGTK on Linux. The current macOS bundle requires macOS 14.0 or later, as declared in `apps/desktop/tauri.conf.json`.

## Window and system features

- The main window defaults to 1280×832 and has a minimum size of 880×600. WebUI also targets desktop-class browsers.
- Windows and Linux use application titlebar controls; macOS keeps native traffic-light buttons.
- On Windows and Linux, launching a second instance focuses the existing window.
- Login startup is controlled in settings. System notifications require OS permission. Local automatic tasks stop when the application exits.
- The application registers `nomifun://`. Individual links still require a supported product destination.

## Data location

| System | Default stable data root |
| --- | --- |
| Windows | `%LOCALAPPDATA%\NomiFun` |
| macOS | `~/Library/Application Support/NomiFun` |
| Linux | `$XDG_DATA_HOME/NomiFun`, usually `~/.local/share/NomiFun` |

Development scripts use a separate `NomiFun-dev` directory. Set `NOMIFUN_DATA_DIR` before launch to choose another root; every host uses the value literally, without a suffix. Only one backend can hold the directory's `server.lock`.

The root contains the database, logs, companions, knowledge and runtime caches. External workspaces can live elsewhere. See [Data and Storage](../architecture/data-and-storage.md) for storage, backups and incompatible datasets. Copying a directory does not guarantee that every historical version can be restored. Exit all hosts using the directory before file-level backup or maintenance.

## Local trust and remote access

The desktop WebView automatically supplies local trust credentials. Knowing the backend port alone does not authorize another process. The OS account running NomiFun still owns the files and commands used by the Agent.

For another computer, enable the appropriate entry under [Connections & Devices](webui-remote-access.md). For an independent server, use [Web Server Deployment](web-server-deployment.md).

## Updates

Use application settings to check the configured release channel. Follow the displayed download, verification and installation results. Published updates require signed artifacts for the target platform. Developers maintain endpoints, keys and artifacts through the [Updater guide](../../apps/desktop/updater/README.md).

## Troubleshooting

- **Blank window**: check the system WebView. Confirm WebView2 on Windows or the WebKitGTK runtime on Linux.
- **Data directory in use**: exit the desktop or Web host using it. Give parallel instances separate data roots.
- **Command or component unavailable**: inspect command paths, cache execution permissions and the actual diagnostic. Terminals, MCP and hosted applications use their supported runtimes; opening diagnostics does not install every tool. See [External Tools](../architecture/external-tools.md).
- **Missing Linux build library**: use the development packages listed in [Installation](../getting-started/installation.md). Runtime packages do not replace link-time development files.
