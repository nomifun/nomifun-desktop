# One installation shared by all local Agents

Resolve the executable once when first using Feishu in a session. Reuse that
absolute launcher for the rest of the task, including help and authentication.
New Agents and sessions repeat discovery, not installation. The CLI belongs
to the current operating-system user; it is shared across NomiFun Agents,
workspaces, sessions and application dataset directories on that host.

## Authoritative installation root

The Host supplies `NOMIFUN_USER_CLI_ROOT` in each process environment. Read that
single value through an authorized process, for example:

```json
{"component":"node","args":["-p","process.env.NOMIFUN_USER_CLI_ROOT"]}
```

This uses NomiFun's existing Node component, not a Lark component. Validate
that the returned value is a nonempty absolute path. Do not replace it with a
path derived from the workspace, session, model, Node version or npm prefix.
The normal platform locations below are explanatory; the Host-returned path
is authoritative, including relocated user-data directories.

| Platform | Shared npm prefix | Lark launcher relative to that prefix |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%\nomifun-user-tools\npm` | `lark-cli.cmd` |
| macOS | `~/Library/Application Support/nomifun-user-tools/npm` | `bin/lark-cli` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/nomifun-user-tools/npm` | `bin/lark-cli` |

This root is outside the replaceable Node/runtime cache and Agent datasets.
It survives runtime upgrades and Agent-data cleanup. Do not redirect HOME,
USERPROFILE or the CLI's credential directory; reuse the user's CLI account
configuration separately from the executable installation.

## Discover before installing

1. Honor an executable path explicitly chosen by the user. Otherwise check
   the shared launcher's absolute path and run it with `--version`.
2. If that launcher is absent, look for an existing user installation with
   the platform's command lookup (`Get-Command` on Windows, `command -v` on
   macOS/Linux). Resolve a real absolute launcher rather than a shell alias.
   Also inspect `npm prefix --global` for a pre-existing global launcher that
   is missing from PATH. Do not dump the rest of the environment or npm config.
3. If an existing launcher works, use that absolute path. Do not copy it into
   the shared root or install another copy simply to standardize its location.
4. Only install after these checks establish that no installation is present.
   A failed launch, expired login, missing scope, network failure or unsupported
   flag is not evidence that the CLI is absent. Diagnose an existing broken
   installation in place; do not create a second installation as a fallback.

If the Host root hint is unavailable, reuse a working existing installation
or report that location discovery is unavailable. Do not guess a new install
directory. If multiple installations already exist, select the applicable one
and report their paths when relevant; do not automatically delete user tools.

## First installation into the fixed prefix

Use the exact official package `@larksuite/cli`. Replace the placeholder with
the Host-returned absolute prefix; it is one literal argument, without shell
quotes, tilde expansion or environment-variable syntax:

```json
{"command":"npm","args":["install","--global","--prefix","<NOMIFUN_USER_CLI_ROOT>","@larksuite/cli"],"wait_ms":1000}
```

The explicit `--prefix` keeps this package in the same location even if
system Node, nvm, `.npmrc` or `NPM_CONFIG_PREFIX` changes. It does not rewrite
the user's global npm configuration. NomiFun supplies the Node/npm runtime
when needed and preserves existing proxy/certificate settings.

Observe this same install process until it exits, then verify `--version`
and `--help` using the shared launcher's absolute path. A new Agent, workspace
or missing PATH entry never justifies another install. If another install is
known to be in progress, observe/wait for it and recheck that same prefix;
do not launch a competing install or choose a different directory. Report
actual download, permission or launch failures without retrying into a new root.

Do not use a workspace-local `node_modules`, session or temporary directory,
runtime cache, or `npx`/`npm exec` package cache to obtain another Lark CLI.
Do not install on each task, run floating upgrades as a readiness check, or
use administrator privileges to work around an installation error. An update
or repair, when needed and authorized, targets the selected existing prefix.
Use the [in-place update workflow](updates.md) for version checks and upgrades;
do not use bare `lark-cli update` to maintain this npm installation.

The upstream `npx @larksuite/cli@latest install` wizard also installs upstream
skills into other agents' directories. This bundled skill needs only the CLI.
Do not run that wizard or `skills add -g` as a prerequisite. Requested extra
upstream skills use NomiFun's existing import/discovery and selection flow;
the active turn's frozen skill content does not change.
