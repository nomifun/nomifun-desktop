# FAQ

[简体中文](faq.zh.md)

## What runs locally?

The desktop app and its embedded backend run on your machine. A dedicated
`nomifun-web` deployment runs tools and Agent turns on its server.
Configured model services, MCP servers, channels, updates and optional downloads
can make network requests. A selected provider or gateway has its own data and
billing policies.

NomiFun is self-hosted software. An optional independently operated model gateway
is a provider choice, not a hosted NomiFun workspace or central account.
See [model routing](../guides/model-routing.md).

## Does Desktop require login?

The desktop WebView uses a per-boot trust secret and opens without a login screen.
Other browsers require authentication. Dedicated `nomifun-web` also requires
login by default. See [Connections & Devices](../guides/webui-remote-access.md).

## Where is my data?

Stable defaults are `%LOCALAPPDATA%\NomiFun` on Windows,
`~/Library/Application Support/NomiFun` on macOS and
`$XDG_DATA_HOME/NomiFun` (or `~/.local/share/NomiFun`) on Linux.
Explicit `NOMIFUN_DATA_DIR` / `--data-dir` values are used literally.
Docker uses its mounted data volume; development channels use separate roots.

Only one running backend can hold a data root. For work-directory precedence,
storage relocation and backup behavior, read
[configuration](configuration.md) and [data/storage](../architecture/data-and-storage.md).
Superseded Agent datasets are not imported into the current canonical lineage.

## Which Agent and providers are supported?

Conversations use the built-in `nomi` runtime. Configure model providers and
keys in the application; keys are encrypted at rest.
Third-party Agent CLIs remain separate programs usable through
[in-app terminals](../guides/terminal.md), with their own authentication and
approval flows. `nomicore doctor` reports detected terminal CLIs.

## What runs applications and skills?

Application hosting owns foreground processes in managed application directories.
Skills supply instructions to Agent context. They do not own application
processes. See [application hosting](../guides/app-hosting.md).

Release builds embed OfficeCLI. Node/Bun preparation is explicit and verified;
backend startup does not download them. Application hosting uses system or
user-configured Node. See [external tools](../architecture/external-tools.md).

## Can I use another computer or a phone?

Use a desktop browser against a dedicated Web host or Desktop's WebUI.
The shared renderer supports desktop-class viewports of at least 880x600.
Phones use the separate NomiFun Mobile client. Pairing and optional relay
transport are described in [Connections & Devices](../guides/webui-remote-access.md).

## How do I install or update?

Use [installation](../getting-started/installation.md), the current download
links in the [project README](../../README.md), or the
[Web deployment guide](../guides/web-server-deployment.md).
Desktop uses signed updater packages; see
[desktop updates](../guides/desktop-app.md#updates).
The official Web image is `nomifun/nomifun-web`; pin a version or digest for
repeatable deployment.

A server build consists of the Rust binary and its built SPA directory
(or a build configured to embed it). Desktop distribution uses platform packages.
See [building and packaging](../contributing/building-and-packaging.md).

## I forgot the administrator password

For Desktop WebUI, use the trusted in-app account/password controls in
**Connections & Devices → WebUI**.
A loopback browser request does not gain desktop local trust.
Changing startup `NOMIFUN_ADMIN_PASSWORD` does not replace an existing account.

For a dedicated host, preserve its data directory and consult
[authentication troubleshooting](troubleshooting.md).
Do not clear password hashes or expose a no-auth listener as a recovery shortcut.

## What is the name and license?

Use **NomiFun** for the product. Lowercase `nomifun` remains in technical
identifiers such as crate names and environment variables.
The project is [Apache-2.0](../../LICENSE); preserve its license and notices
when redistributing.

## Further reference

- [Configuration](configuration.md)
- [API overview](api-overview.md)
- [Troubleshooting](troubleshooting.md)