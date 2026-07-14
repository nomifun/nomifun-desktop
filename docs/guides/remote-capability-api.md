# Remote Capability API

NomiFun can expose its platform capabilities through a network-reachable,
token-authenticated MCP and REST front door. A trusted external agent or MCP
client can connect with a URL plus a companion access token and then call the
same capability registry used by the desktop app.

Each token is bound to one companion. Calls made with that token run as that
companion and inherit its profile model, persona, and knowledge context.

For copy-ready integrations, see
[Remote Capability API Examples](./remote-capability-api-examples.md).

## Security Model

A companion access token is high privilege. It can call exposed platform
capabilities, read and write files, and in desktop builds may operate browser or
computer-use capabilities. Treat it like remote code execution authority:

- Give tokens only to clients and agents you trust.
- Prefer loopback, VPN, or a private network.
- Put TLS, firewall rules, and rate limits in front of any public exposure.
- Rotate or revoke tokens immediately if they leave your control.
- Sensitive tools such as secrets and factory reset are not exposed on the
  remote surface by default.
- Destructive tools require a confirmation retry: the first call returns a
  confirmation challenge; the caller must show the action to the user and retry
  with `confirm: true`.

## Endpoints

The network front door is mounted by the same backend process as the Web UI.

| Endpoint | Purpose |
| --- | --- |
| `/mcp` | Full Streamable-HTTP MCP server. |
| `/mcp-agent` | Curated MCP profile for external working agents. |
| `/v1/tools` | REST tool discovery. Add `?profile=agent` for the curated set. |
| `/v1/tools/{name}` | REST tool call. |
| `/v1/tools/{name}/stream` | SSE streaming wrapper for tools that emit progress. |
| `/v1/openapi.json` | OpenAPI 3.1 description for the REST tool surface. |

Authenticate every request with:

```http
Authorization: Bearer <companion-access-token>
```

Common base URLs:

- Desktop remote access: `http://<LAN-IP>:25808`
- Standalone server: `http://<host>:8787` unless you changed the port
- Local development or embedded desktop backend: `http://127.0.0.1:<port>`

## Creating A Companion Token

Tokens are stored hashed. The plaintext token is shown only once.

### Desktop App

Use the Open Capabilities / remote access UI, or call the trusted local API
from the desktop WebView context:

```bash
curl -X POST \
  http://127.0.0.1:<loopback-port>/api/webui/companions/<companion-id>/access-token
```

The response returns the plaintext token once:

```json
{
  "success": true,
  "data": {
    "token": "<64-character-hex-token>",
    "companion_id": "<companion-id>"
  }
}
```

Status and revoke use the same path:

```bash
curl http://127.0.0.1:<loopback-port>/api/webui/companions/<companion-id>/access-token

curl -X DELETE \
  http://127.0.0.1:<loopback-port>/api/webui/companions/<companion-id>/access-token
```

These token-management endpoints require local trust. A remote browser or plain
curl client cannot mint tokens.

### Headless `nomifun-web`

Seed a token at startup with `NOMIFUN_COMPANION_TOKEN`. The value binds to the
default companion when no token is already configured:

```bash
NOMIFUN_COMPANION_TOKEN="$(openssl rand -hex 32)" \
  nomifun-web --host 127.0.0.1 --port 8787
```

Use the generated hex string as the Bearer token. For non-local exposure,
finish admin setup first and put the server behind TLS.

## MCP Client Configuration

Example Streamable-HTTP MCP configuration:

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://127.0.0.1:25808/mcp-agent",
      "headers": {
        "Authorization": "Bearer <companion-access-token>"
      }
    }
  }
}
```

Use `/mcp-agent` when an external agent mostly needs work tools
(browser/computer/knowledge/files/conversations). Use `/mcp` when you
intentionally want the broader platform control surface.

## REST Tool Calls

Discover tools:

```bash
curl -s "http://127.0.0.1:25808/v1/tools?profile=agent" \
  -H "Authorization: Bearer $TOKEN"
```

Call a tool returned by discovery, using that tool's JSON Schema:

```bash
curl -s -X POST "http://127.0.0.1:25808/v1/tools/<tool-name>" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"argument":"value"}'
```

Successful REST calls return `200 {"result": ...}`. Tool validation failures
return `422`, unknown tools return `404`, invalid tokens return `401`, and
confirmation-required calls return `409`.

## Streaming

SSE streaming is available for tools that report progress:

```bash
curl -N -X POST "http://127.0.0.1:25808/v1/tools/<tool-name>/stream" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"argument":"value"}'
```

Each event is a `data: <json>` line. The final event uses
`{"type":"__result__","data":{"result":...}}`.

## Agent Collaboration Boundary

Persistent single- and multi-Agent work uses one execution contract:

- `nomi_delegate` creates an Agent execution from a goal or explicit steps.
- `nomi_execution_get` reads its plan, attempts, results, and current state.
- `nomi_execution_update` applies all lifecycle and plan mutations.

Availability is authority-bound, not transport-bound. Desktop and Channel
callers derive authority from their calling Conversation and execution link.
An owner-bound companion token may also use the three tools through Remote
MCP/REST: `nomi_delegate` records that companion as the immutable creator, and
subsequent reads or updates are limited to that exact companion's executions.
Secondary users see none of the three tools on any surface. Because minting a
companion token delegates installation-owner authority, it is restricted to a
trusted local owner context; discover the effective Remote catalog through
`/v1/tools` and protect the token as a high-privilege credential.

## Companion Context

Every Remote call runs as the companion bound to the token. Model-backed tools
can therefore use that companion's configured provider/model where their
schema supports it. Configure a usable model before relying on such tools;
token creation may warn when the companion has none.

## Related Docs

- [Remote Capability API Examples](./remote-capability-api-examples.md)
- [WebUI Remote Access](./webui-remote-access.md)
- [Web Server Deployment](./web-server-deployment.md)
- [Computer Use And Browser Use](./computer-browser-use.md)
