# Discover once; let the Office Host prepare a missing component

Before the first Feishu operation, find and reuse one absolute executable.
The CLI belongs to the current operating-system user, shared by local Agents,
workspaces, sessions and application datasets. New sessions repeat discovery,
not installation. Authentication errors never justify reinstalling the CLI.

## Read-only executable discovery

1. Honor an absolute executable explicitly selected by the user. Otherwise
   inspect the existing Host-shared launcher or use the platform's command
   lookup (`Get-Command` on Windows, `command -v` on macOS/Linux).
2. Resolve a real absolute launcher, not an alias. Probe it with `--version`
   and inspect the needed command's `--help` / `schema`.
   For the official npm package, prefer its already present native
   `bin/lark-cli` (`bin/lark-cli.exe` on Windows) after checking the public
   package manifest identifies `@larksuite/cli`. A launcher without that native
   file is a broken installation: do not run its wrapper as a readiness probe,
   because the wrapper may download the missing binary itself.
3. If npm and Node are both already available, `npm prefix --global` can locate an existing
   user installation missing from PATH. Read only that prefix, not the whole
   environment or npm configuration.
4. Reuse the working installation in place. Do not copy, move, delete, upgrade
   or install a second copy to standardize its location.

The Host's process environment may provide `NOMIFUN_USER_CLI_ROOT`, the
authoritative persistent shared npm prefix. Read only that location hint
through an existing authorized process; do not prepare Node solely to read an
environment variable. Typical locations are explanatory:

| Platform | Shared prefix | Launcher |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%\nomifun-user-tools\npm` | `lark-cli.cmd` |
| macOS | `~/Library/Application Support/nomifun-user-tools/npm` | `bin/lark-cli` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/nomifun-user-tools/npm` | `bin/lark-cli` |

The workbench can select the official npm package's native executable after
checking its identity and compatibility. Reuse the selected absolute path and
literal arguments. Do not redirect HOME, USERPROFILE or credential directories.
Runtime upgrades and Agent-data cleanup do not own this persistent installation.

## Missing, broken or incompatible installation

If no executable is found, tell the user to open:

**Office → Connection settings → Prepare environment → Prepare component**

The product route is `/office`. The Host performs discovery again,
reuses a compatible installation, and prepares the fixed official npm package
only when absent and explicitly requested. The component is not embedded in the
NomiFun installer. Workspaces and Agents can discover and reuse that shared
installation; the workbench's selected path and profile are not an implicit
selector for Agent process calls.

The skill must not execute its own `npm install`, `npm exec`, `npx`, package
download, install script or alternate-prefix repair for a missing component.
Do not install per project, Session, temporary directory or npx cache.
Do not try to obtain a local trust token, browser credential or owner credential
to call the workbench's protected API from an Agent process. Direct the user to
the authenticated product surface; no hidden API backchannel is needed.

A failed launch, expired login, missing scope, network error or unsupported flag
does not establish that the CLI is absent. Report the actual issue and direct
component repair to the same workbench surface. Do not bypass an incompatible
or broken installation by installing somewhere else. If location discovery is
unavailable, report that condition rather than inventing a root.

After the Host reports readiness, repeat read-only discovery and use the same
absolute executable for the original task. Installation readiness does not
authorize business writes or broaden the current Session's tool permissions.

## One Host supply entry, normal Agent execution

The Office supplier serializes its own preparation with the shared-prefix
cross-process lock. This skill's text is not a lock and does not coordinate
external npm processes. The skill delegates missing-component preparation
instead of running a competing installer.

Already-installed CLI business operations continue through the existing
authorized canonical process tools with literal argv. Do not create a second
Agent transcript or treat workbench activity as a record of all Agent calls.
The workbench connection switch does not revoke permissions granted in Feishu
or remove the user's ability to run their own CLI elsewhere.

The workbench also offers the official full setup command for the user to run
explicitly in their own terminal. That upstream wizard can install global
skills for other Agents and enter account setup; it is not the skill's hidden
installation prerequisite. NomiFun already bundles this opt-in skill.

For requested checks or updates, read [component ownership and updates](updates.md).
