---
name: drive-nomifun
description: >-
  Use to connect to and drive a NomiFun instance from an external agent
  (Claude Code / Cursor / any MCP client). Drive its browser / computer /
  knowledge base / files and manage the platform over MCP or REST, with a
  NomiFun Desktop installation access token. The token authenticates the owner
  and never binds or impersonates a companion. Use this whenever
  the user asks to control,
  automate, or hand work off to "NomiFun", "their NomiFun", "the desktop
  companion", or a running NomiFun server.
---

# Drive NomiFun Desktop

NomiFun exposes its Remote-safe platform capability set — browser automation,
computer control, knowledge bases, files, terminals, conversations, and platform
management — to external callers through one **MCP** endpoint and an equivalent
**REST** API, authenticated by one **installation access token**. Calling with
it runs under the NomiFun Desktop installation owner. It never selects a
companion and does not inherit a companion profile, persona, knowledge binding,
active thread, or workspace.

## 1. Connect (MCP, recommended)

Configure NomiFun as a Streamable-HTTP MCP server:

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://<host>:25808/mcp-agent",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

- `<host>` is the machine running NomiFun (`127.0.0.1` locally, or its LAN/public
  address with WebUI remote access enabled).
- **`/mcp-agent`** advertises a tight, curated tool set for getting work done
  (browser, computer, knowledge, files, conversations). Use **`/mcp`** instead
  for the full platform-control surface (~140 tools incl. channels, companions,
  cron, providers, …).
- `<token>` is the **NomiFun Desktop installation access token**.
  Get it from the NomiFun operator: in the desktop WebUI/remote panel, or by
  minting it (see "Minting a token" below). Model-backed capabilities use an
  explicit model or an enabled instance provider; minting returns a `warning`
  when the instance has no enabled provider.

REST equivalent (for scripts): `POST http://<host>:25808/v1/tools/<name>` with the
same Bearer token; `GET /v1/tools?profile=agent` lists tools; `GET
/v1/openapi.json?profile=agent` is a machine-readable contract.

### Minting a token (operator, local-trust only)

The mint/query/revoke endpoints are **local-trust gated** (reachable only from
the desktop client / loopback, not from a remote browser):

```bash
# Mint — returns the plaintext token ONCE.
curl -X POST http://127.0.0.1:<port>/api/webui/access-token
# => { "token": "<64-hex token>" }

# Query whether one is configured (does NOT return the token):
curl http://127.0.0.1:<port>/api/webui/access-token
# => { "configured": true }

# Revoke:
curl -X DELETE http://127.0.0.1:<port>/api/webui/access-token
```

For a **headless** server, seed a token at startup via the
`NOMIFUN_ACCESS_TOKEN` env var. This does not require a companion:

```bash
NOMIFUN_ACCESS_TOKEN="$(openssl rand -hex 32)" nomicore   # or nomifun-web
```

## 2. Respect the Agent collaboration boundary

Persistent Agent collaboration uses one contract: `nomi_delegate` creates an
execution, `nomi_execution_get` reads it, and `nomi_execution_update` changes
its plan or lifecycle. Desktop and Channel callers derive execution ownership
from the active calling Conversation. The installation token can also receive
the three tools on the Remote surface and uses the same installation-owner
boundary as Desktop. It does not create a companion identity or scope
executions by companion. Secondary users receive none of the three tools.

Use the three tools only when they are advertised by the live catalog. A Remote
client must call the capabilities returned by `tools/list` directly and protect
its token as a high-privilege delegation of installation-owner authority;
never invent execution ids or try to bypass the installation-owner boundary.

## 3. Or drive capabilities directly

Use individual tools when you want fine control: `nomi_browser_*` (navigate /
observe / act), the computer tools, `nomi_knowledge_*` (search / read / write
knowledge bases), `nomi_fs_*` (read / write / browse files), `nomi_create_terminal`,
and the conversation tools. `GET /v1/tools` (or MCP `tools/list`) is the live,
authoritative catalog with JSON Schemas.

## 4. Confirmations & limits

- **Destructive actions** (deletes, etc.) return `{ "needs_confirmation": true,
  "restate": "..." }`. Restate the exact action to the user, get agreement, then
  re-call the same tool with `"confirm": true`.
- **Sensitive actions** (secrets, factory reset) are **denied** on this surface.
- **Trust model:** holding the installation access token grants full,
  RCE-equivalent control of that NomiFun instance. Treat
  the token as a high-value secret; only connect to instances you are authorized
  to drive. Rotating or revoking it affects every Remote client of that installation.

## 5. Failure handling

- `401` → missing / invalid / revoked access token.
- REST `409` (or `needs_confirmation` in the body) → re-call with `confirm: true`.
- REST `422` / a `{ "error": ... }` body → the tool rejected the arguments;
  check the schema from `/v1/tools` and retry.
- Connection refused → NomiFun isn't running or the URL/port is wrong.
