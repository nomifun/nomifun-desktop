# MCP & Skills

NomiFun has three extension mechanisms that are easy to confuse:

- **MCP servers** are external tool servers. They expose callable tools over
  stdio, HTTP, or SSE.
- **Skills** are markdown/folder knowledge bundles. They tell an agent how to do
  a workflow; they are not long-running tool servers.
- **Connectors** are third-party account connections (such as Feishu). They
  manage credentials in one place and offer capabilities to knowledge bases and
  sessions.

Current pages:

| Capability | Page |
| --- | --- |
| MCP servers | `/enhanced-tools?category=mcp` |
| Skills | `/enhanced-tools?category=skills` |
| Connectors | `/enhanced-tools?category=connectors` |
| Agent workbench | `/agent` |
| Open capabilities (MCP / REST) | `/connections?task=tools` |

MCP and Skills share the **MCP & Skills** navigation entry, while retaining
their own data and operations. Search follows the selected category. The header's
Add menu exposes JSON import, one-click import, and MCP Market for MCP; or
Import from Agent, Import Skills, Import .zip, and Skill Market for Skills.
Markets open with `view=market`; closing them returns to the same category.
Legacy `/mcp`, `/skills`, and settings URLs redirect here, preserving old market
selection, skill highlighting, and other query state.

## MCP Servers

Open **MCP & Skills → MCP** (`/enhanced-tools?category=mcp`) to add, import,
test, enable, and disable MCP servers. Each row keeps its connection check,
configuration, authentication, deletion, and tool-list actions.

Each server row owns:

- name;
- transport: `stdio`, `http`, or `sse`;
- command / args / env for stdio, or URL for HTTP/SSE;
- raw imported JSON, when imported from another agent config;
- enabled state;
- last connection-test result.

Connection test uses a temporary MCP client, performs the handshake, lists tools,
and persists the result. Failure codes include command-not-found, permission,
timeout, HTTP, RPC, and protocol errors.

The enable/disable switch beside each server controls global availability separately
from connection testing. Market imports remain disabled. Complete the configuration,
test it, and enable the server to select it in a Session. Enabling before testing is
also supported, but new Session selections require a successful test and discovered
tools. Testing does not enable a server, and toggling does not launch a test. After
either operation completes, mounted Session MCP lists re-read the backend catalog
without replacing the user's selection draft.

The canonical owner currently supports Streamable HTTP/stdio at `2025-03-26`
and explicit legacy SSE at `2024-11-05`. It does not silently convert between
transports and does not yet claim the `2026-07-28` `server/discover` lifecycle
without `initialize`; a server that only supports that lifecycle returns a
typed protocol error. That is a separate owner and security-model migration,
not part of market configuration import.

### OAuth login

OAuth-backed HTTP/SSE connections authenticate by their saved server ID. The
backend reads the Bearer challenge and Protected Resource Metadata, then
follows the advertised authorization-server issuer. Client identity comes from
a pre-registered client, supported CIMD, or dynamic registration, rather than a
fixed universal client ID. The settings page creates a one-use attempt, opens
its authorization URL in the user's browser, and queries the attempt result.
Cancellation, timeout, denial, and token-exchange failures carry safe diagnostics
with a stage, code, retry guidance, and trace ID.

The native desktop entry binds a temporary `127.0.0.1` loopback callback on the
backend machine. Remote WebUI requires an operator-configured
`NOMIFUN_MCP_OAUTH_CALLBACK_URL`: an externally reachable HTTPS URL whose path
is exactly `/api/mcp/oauth/callback`, registered with the OAuth client. It does
not redirect to the remote user's loopback. A product-controlled CIMD URL can
be configured with `NOMIFUN_MCP_OAUTH_CLIENT_METADATA_URL`; see
[configuration](../reference/configuration.md#mcp-oauth).

Saved connection tests and Runtime calls use the same installation-owned
authentication service and reuse or refresh the saved grant after login.
Tokens are not copied into MCP JSON. Authentication and connection-test status
remain separate. Changing the connection, logging out, or deleting it cancels
pending authorization and invalidates its grant. Metadata edits, an unchanged
save, enable toggles, and ordinary probes retain authorization. OAuth refresh
or re-login does not require rebuilding Agent Sessions.

Upgrades clear previous URL-only OAuth credentials whose issuer, resource,
and client ownership cannot be proven, so those connections require re-login.
MCP configuration, other non-Agent configuration, and current Session history
are preserved.


### MCP market boundary

The MCP market is a **configuration catalog**, not a package manager. When an
entry is added, NomiFun ranks the configuration examples in its market
documentation (preferring portable `npx`/`uvx` launchers and HTTPS over Docker,
global commands, or placeholder endpoints), normalizes common forms such as
`streamableHttp` and `baseUrl`, and imports the result disabled. It does not
install Node.js, Docker, Python/uv, browser drivers, complete third-party login,
or create API keys.

The confirmation screen shows the actual command, arguments, URL, env/header
keys, and fields that still need values. A server containing `${...}`, `<...>`,
`xxxxx`, `YOUR_*`, or similar placeholders cannot be tested until those fields
are completed, preventing template URLs from being contacted or unconfigured
processes from being launched. Failed entries previously imported from the
market expose a **Repair config** action; replacements are reviewed again and
remain disabled.

A connection test only proves that the configuration completed the MCP
handshake and `tools/list` at that moment; it is not a live presence signal.
Failures distinguish missing runtimes, HTTP status, timeout, RPC, and protocol
errors. URL-based servers enter the OAuth flow only after an explicit OAuth
Bearer challenge, so API-key/header configurations are not treated as OAuth.

Connection tests and real invocations share one bounded setup policy. Runtime
preparation, process launch, handshake and catalog verification have phase
limits within a cumulative setup deadline; changing phases never renews it.
Portable package runners (`npx`, `bunx`, `uvx`, and equivalent launcher forms)
may take 120 seconds for first startup; ordinary protocol setup has a 30-second
phase budget. Neither deadline includes subsequent tool execution.

Tool execution has no default total or silence deadline. An HTTP service may
wait until computation finishes before returning response headers. Business
waiting ends on user cancellation, an explicit execution deadline, a real
transport failure, a protocol/resource violation, or host shutdown. Logs, ping
and missing progress do not prove a stalled tool. Notifications use bounded
processing without a lifetime count that cuts off healthy long operations.
Cancellation targets the original request and retains local cleanup ownership;
disconnect, cancellation delivery and process exit do not prove absence of
earlier effects. Unknown results are never automatically replayed. A caller
ending its wait does not declare the tool failed.

Stdio children receive
only proxy-related variables from the parent or detected system proxy through
the shared proxy policy (including its stale loopback-proxy guard), while API
tokens and unrelated parent environment remain isolated. Raw child stderr is
never returned or logged: it is drained and
reduced to non-secret categories such as package-not-found, download/network,
missing dependency, missing configuration, permission, or early process exit.

A failed probe does not replace the same configuration's last fully verified
tool catalog with an empty list. A hosted app's transient timeout or connection
failure likewise retains the verified catalog for the same running process and
address while reporting the failed probe honestly. New selections still require
successful discovery. Existing frozen calls retain exact pre-dispatch schema
verification; cached descriptors grant no new authority.

An HTTP URL on `localhost` is only a client connection descriptor. NomiFun does
not start the referenced application or Docker container. A failed local probe
therefore reports the host and port and asks the user to start that prerequisite
service; it no longer presents every runtime or service failure as malformed
MCP JSON.

## Importing Agent Configs

`GET /api/mcp/agent-configs` performs read-only detection of supported local
agent CLI and NomiFun MCP configurations. The UI can import detected servers;
this endpoint does not write external CLI configurations. A Session still
chooses which MCP servers it uses.

## Global capabilities and Session selection

Every Agent preset can use the same installed Skill Library and global MCP catalog. The Workbench has
no Skill, MCP, or tool-search master switch. The two icons beside the composer select what this Session
uses, without editing the Agent preset or global installation state.

New Sessions select auto-injected Skills by default. MCP servers require an explicit choice; enabling
or successfully testing a server never selects it for a Session. The Session picker shows enabled MCP
sources only; disabled sources remain available on the MCP management page. Users can deselect default
Skills and choose other installed Skills or ready servers. Global Skill bodies and supporting text/images
are captured as immutable reference data.
Selected instructions accompany accepted input; long bodies and supporting resources use the native
context reader. Skill scripts, hooks, and tool declarations grant no additional permissions.

Creation accepts top-level `session_capabilities` with `skill_names` and `mcp_server_ids`. Omission keeps
default Skills and the Agent's explicitly saved MCP grants without adding other enabled sources;
explicit empty arrays select none. Existing Sessions use
`GET` / `PUT /api/agent-sessions/{id}/capability-selection` with `expected_binding_version`.
Sending waits for selection admission; failures preserve input and selection for an explicit retry.

Selections compile into the existing immutable Revision/Snapshot and apply through canonical binding
transitions. Running or paused Turns, unsettled effects, Remote bindings, and read-only Attempts cannot
change selection. Agent switches retain selection; workspace and other resources remain intact.
Accepted input and native checkpoints are not rewritten. An unchanged connection test or metadata
update keeps the same MCP connection identity; actual transport configuration or tool schema changes
require reapplying selection. OAuth grants and tokens belong to the current connection's credential
lifecycle, not its Session Snapshot. Ordinary refresh and re-login do not change frozen Agent
permissions or MCP selections.

## MCP API

| Operation | Endpoint |
| --- | --- |
| List / create | `GET`, `POST /api/mcp/servers` |
| Import batch | `POST /api/mcp/servers/import` |
| Get / update / delete | `GET`, `PUT`, `DELETE /api/mcp/servers/{id}` |
| Toggle | `POST /api/mcp/servers/{id}/toggle` |
| Test connection | `POST /api/mcp/test-connection` |
| Detect agent configs | `GET /api/mcp/agent-configs` |
| Authentication status / logout | `GET`, `DELETE /api/mcp/servers/{id}/auth` |
| Start OAuth attempt | `POST /api/mcp/servers/{id}/auth/attempts` |
| Query / cancel attempt | `GET`, `DELETE /api/mcp/servers/{id}/auth/attempts/{attempt_id}` |
| OAuth callback | `GET /api/mcp/oauth/callback`, bound to the original attempt by one-use state |

## Skills

Open **MCP & Skills → Skills** (`/enhanced-tools?category=skills`). Skills retain
their descriptions, source labels, detail inspection, and custom-skill deletion.

A skill is either a single markdown file or a directory containing `SKILL.md`.
Sources:

| Source | Meaning |
| --- | --- |
| Builtin | Shipped with the app. Some are auto-injected. |
| Custom | Imported by the user or placed in a configured skill directory. |

Skills can be tagged, imported, exported/symlinked, and scanned from external paths.
Custom and bundled Skills use the same Session capture and native context reader.

The Skill Market Add action is owned by the Skill Library service: the backend validates the market identity, downloads and safely extracts the archive, and imports it into the user skill directory. It neither creates an Agent Session nor executes a command supplied by the market; installation and later Session use are separate phases.

### Official Feishu / Lark CLI skill

The bundled **Feishu CLI** (`lark-cli`, shown as **飞书 CLI** in Chinese) skill uses
the [official Lark CLI](https://github.com/larksuite/cli/blob/v1.0.98/README.zh.md)
for messages, documents, Drive, Base, spreadsheets, calendars, tasks, mail and wikis.
The Feishu connector provides it: once the connector is ready, select it in the
conversation skill picker per session; it is not automatically injected.
Example requests include opening a linked document, reading next week's agenda
or updating a specified Base table.

Each Session discovers and reuses an existing absolute executable through the
canonical process tools with literal arguments. A connector path or profile
selection is not automatically injected into Agent calls. Missing components go
to **MCP & Skills → Connectors → Feishu connector**, open **Connection
settings**, and use its **Prepare environment** step. The skill does not launch
a competing npm/npx installer or obtain trust tokens to call protected Host APIs.

The CLI is prepared only after explicit user opt-in and is not bundled in NomiFun.
The Host verifies and reuses compatible installations before preparing a missing
fixed-version official npm package. The shared prefix is separate from projects,
runtime caches and Agent data. `NOMIFUN_USER_CLI_ROOT` is its Host-derived hint:

| Platform | Shared npm prefix | Launcher |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%\nomifun-user-tools\npm` | `lark-cli.cmd` |
| macOS | `~/Library/Application Support/nomifun-user-tools/npm` | `bin/lark-cli` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/nomifun-user-tools/npm` | `bin/lark-cli` |

For the official npm package, the Host selects the already present native binary.
A wrapper missing that binary is reported as broken rather than executed to
trigger a download. Runtime upgrades and Agent cleanup retain the installation.
The Host coordinates its own preparation with a shared-prefix lock; skill prose
does not lock independent external npm operations.

[Feishu Knowledge](office-hub.md) shares the same three-step connection dialog,
native paragraph editing and single-submit business saves with the connector.
The skill remains an ordinary
opt-in Agent capability through canonical processes. Its business responses often
use `ok`/`data`; configuration, profiles and authorization have their own raw JSON
shapes. Follow each installed command's contract.

Credentials stay in the CLI's store. Application permissions, user consent and
resource membership are separate. Selecting the skill grants no additional tools
or Feishu access, and disabling the connector does not revoke remote authorization.

The full official setup command, `npx @larksuite/cli@latest install`, is available
for the user to run explicitly in their terminal. It may install global upstream
skills and enter account setup; it is not a silent prerequisite.
Connector version checks do not install or synchronize skills. Host-shared
preparation and compatibility policy stay with the connector; external installs
keep their original toolchain and verified prefix. An explicit external update
request can update that installation in place and verify the same program.
Missing upgrade paths are reported rather than bypassed with another installation.
Bundled skill content continues to update with NomiFun releases.

## Connectors

**MCP & Skills → Connectors** (`/enhanced-tools?category=connectors`) manages
third-party account connections in one place, replacing the former top-level
"Office" entry. The first connector is the **Feishu connector**: it hosts the
same three-step connection flow that used to live under Office (component
preparation, app configuration, account sign-in and permissions) and surfaces
the capabilities it provides — [Feishu Knowledge](office-hub.md)
(`/knowledge/feishu`) and the `lark-cli` skill.

Connection state, account and permissions exist exactly once; the connector card
and the Feishu knowledge page header open the same connection settings. Until
the connection is ready, `lark-cli` appears in the session skill picker as
"pending connection" and cannot be checked. Once configured, it still requires
an explicit per-session opt-in and is never injected automatically.

## Skill API

| Operation | Endpoint |
| --- | --- |
| List | `GET /api/skills` |
| Builtin auto-injected list | `GET /api/skills/builtin-auto` |
| Tags | `PUT /api/skills/{name}/tags` |
| Info / paths | `POST /api/skills/info`, `GET /api/skills/paths` |
| Import / export / delete | `POST /api/skills/import`, `POST /api/skills/import-symlink`, `POST /api/skills/export-symlink`, `DELETE /api/skills/{name}` |
| Scan / detect paths | `POST /api/skills/scan`, `GET /api/skills/detect-paths`, `GET /api/skills/detect-external` |
| External paths | `GET`, `POST`, `DELETE /api/skills/external-paths` |
| Skills market | `POST /api/skills/market/enable`, `POST /api/skills/market/disable`, `POST /api/skills/market/rankings/sync`, `POST /api/skills/market/install` |

## Related

- [Agent Workbench](./presets.md)
- [Remote Capability API](./remote-capability-api.md)
- [Terminal](./terminal.md)
