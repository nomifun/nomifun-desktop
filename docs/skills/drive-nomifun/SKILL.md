---
name: drive-nomifun
description: >-
  Use to connect to and drive a NomiFun instance from an external agent
  (Claude Code / Cursor / any MCP client). Delegate goals to NomiFun's
  autonomous agent, drive its browser / computer / knowledge base / files, and
  manage the platform — over MCP or REST, with a per-companion access token (you
  run AS the bound companion). Use this whenever the user asks to control,
  automate, or hand work off to "NomiFun", "their NomiFun", "the desktop
  companion", or a running NomiFun server.
---

# Drive NomiFun (external companion)

NomiFun exposes its full platform capability set — an autonomous agent, browser
automation, computer control, knowledge bases, files, terminals, and platform
management — to external callers through one **MCP** endpoint and an equivalent
**REST** API, authenticated by a **per-companion access token**. Each token is
bound to one specific companion: calling with it runs you **as that companion**,
inheriting its profile model / persona / knowledge bases. Connecting makes you
an "external companion": you drive the platform exactly as its built-in desktop
companion does.

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
  (agent delegation, browser, computer, knowledge, files). Use **`/mcp`** instead
  for the full platform-control surface (~140 tools incl. channels, companions,
  cron, providers, …).
- `<token>` is a **per-companion access token** — it binds you to one companion.
  Get it from the NomiFun operator: in the desktop WebUI/remote panel, or by
  minting one (see "Minting a token" below). The companion you bind to **must
  have a configured model** for `nomi_agent_run` (and other model-backed caps) to
  work — otherwise minting returns a `warning` and those calls fail until a model
  is set in Model Management.

REST equivalent (for scripts): `POST http://<host>:25808/v1/tools/<name>` with the
same Bearer token; `GET /v1/tools?profile=agent` lists tools; `GET
/v1/openapi.json?profile=agent` is a machine-readable contract.

### Minting a token (operator, local-trust only)

The mint/query/revoke endpoints are **local-trust gated** (reachable only from
the desktop client / loopback, not from a remote browser). `{id}` is the
companion id you want the token bound to:

```bash
# Mint — returns the plaintext token ONCE, bound to that companion.
curl -X POST http://127.0.0.1:<port>/api/webui/companions/<id>/access-token
# => { "token": "<64-hex token>", "companion_id": "<id>" }
#    If that companion has no usable model yet, the body also carries
#    "warning": "…" (the token is still minted, but nomi_agent_run etc. will fail
#    until you configure a model).

# Query whether one is configured (does NOT return the token):
curl http://127.0.0.1:<port>/api/webui/companions/<id>/access-token
# => { "configured": true }

# Revoke:
curl -X DELETE http://127.0.0.1:<port>/api/webui/companions/<id>/access-token
```

For a **headless** server, seed a token at startup via the
`NOMIFUN_COMPANION_TOKEN` env var — it binds to the **default companion** (only
if no token is configured yet; it won't overwrite an existing one):

```bash
NOMIFUN_COMPANION_TOKEN="$(openssl rand -hex 32)" nomicore   # or nomifun-web
```

## 2. Delegate a goal (the headline move)

To hand a whole task to NomiFun's own autonomous agent, call **`nomi_agent_run`**:

```json
{ "goal": "Research X and write a summary to notes.md", "timeout_secs": 600 }
```

It spins up a fresh autonomous NomiFun agent (full tools, using the **bound
companion's** profile model — which must be configured), runs the goal to
completion, and returns `{ "status": "completed", "text": "<final answer>", "conversation_id": <id> }`.
For long tasks it returns `{ "status": "running", "conversation_id": <id> }` —
poll **`nomi_agent_result`** with that `conversation_id` until completed.

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
- **Trust model:** holding a per-companion access token grants full,
  RCE-equivalent control of that NomiFun instance (as the bound companion). Treat
  the token as a high-value secret; only connect to instances you are authorized
  to drive. Revoking a token affects only its companion; other companions' tokens
  are unaffected.

## 5. Failure handling

- `401` → missing / invalid / revoked access token.
- REST `409` (or `needs_confirmation` in the body) → re-call with `confirm: true`.
- REST `422` / a `{ "error": ... }` body → the tool rejected the arguments;
  check the schema from `/v1/tools` and retry.
- Connection refused → NomiFun isn't running or the URL/port is wrong.
