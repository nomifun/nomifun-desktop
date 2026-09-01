# Canonical Remote API

NomiFun exposes one installation-owner Remote ingress over MCP and REST. Remote
is a transport, not an Agent type or a capability profile:

1. A local owner creates a `RemoteBinding` that freezes an exact Agent Preset
   revision, resolved snapshot, and typed resources.
2. `open(binding_id)` creates a durable `AgentSession` and returns its UUIDv7
   `agent_session_id`.
3. `turn`, `observe`, and `cancel` always carry that explicit ID.

The installation token authenticates the owner only. It never selects a
companion, model, profile, domain list, workspace, or recent Session.

See [Canonical Remote examples](./remote-capability-api-examples.md) for
copy-ready commands.

## Security model

The installation token is a high-privilege credential:

- give it only to authorized clients;
- prefer loopback, VPN, or a private network;
- use TLS, firewall rules, and rate limits for network exposure;
- rotate or revoke it immediately if it leaves your control;
- do not put it in URLs, logs, screenshots, or issue reports.

Remote execution is FullAuto. There is no `confirm: true`,
`needs_confirmation`, token scope, or profile/domain selector. Authority comes
from the authenticated owner and the immutable Snapshot frozen into the
AgentSession.

## Endpoints

| Endpoint | Contract |
| --- | --- |
| `/mcp` | Streamable-HTTP MCP exposing exactly `open`, `turn`, `observe`, and `cancel`. |
| `POST /api/remote/open` | Open a Session from a `RemoteBinding`. |
| `POST /api/remote/turn` | Start a turn on an explicit Session. |
| `GET /api/remote/observe` | Read events and message projections after an exclusive cursor. |
| `POST /api/remote/cancel` | Cancel the active turn on an explicit Session. |
| `/api/remote-bindings` | Authenticated local management API for owner bindings. |
| `/api/webui/access-token` | Mint/rotate, inspect, or revoke the installation token. |

Authenticate Remote MCP/REST requests with:

```http
Authorization: Bearer <installation-access-token>
```

The removed `/mcp-agent`, `/v1`, generic tool discovery/calls, and
`profile`/`domains` query parameters are not compatibility aliases. They fail
closed.

## Create a RemoteBinding

Create and save an Agent Preset in Agent Settings, resolve its exact revision,
then create a RemoteBinding from the local management UI. The binding contains
only:

- `remote_binding_id`
- `owner_user_id`
- `name`
- canonical `agent_binding`

Updating a binding affects only Sessions opened afterward. Deleting it prevents
new opens but does not rewrite or cancel an existing Session.

## Installation token

The plaintext token is shown only when it is minted. SQLite stores only its
SHA-256 verifier.

```bash
# Trusted local desktop context, or an authenticated installation owner on
# nomifun-web.
curl -X POST http://127.0.0.1:<port>/api/webui/access-token

curl http://127.0.0.1:<port>/api/webui/access-token

curl -X DELETE http://127.0.0.1:<port>/api/webui/access-token
```

For a headless host, seed the same installation token at startup:

```bash
NOMIFUN_ACCESS_TOKEN="$(openssl rand -hex 32)" \
  nomifun-web --host 127.0.0.1 --port 8787
```

Changing the environment value on a later start rotates the stored verifier.

## MCP client configuration

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://127.0.0.1:25808/mcp",
      "headers": {
        "Authorization": "Bearer <installation-access-token>"
      }
    }
  }
}
```

`tools/list` must return exactly:

```text
open
turn
observe
cancel
```

The MCP transport session ID is connection lifecycle state only. Never persist
or reuse it as the product AgentSession identity.

## Open lifecycle

`open` commits the Session before crossing the Runtime process boundary, so its
first successful response may report:

```json
{"open_state":{"state":"opening"}}
```

Continue with `observe` using the returned cursor. Under ordinary conditions the
Session converges to `ready` or `open_failed`. Runtime scheduling is bounded to
5 seconds, the complete admission attempt to 35 seconds, and failure-fact
persistence to 10 seconds.

If durable storage is unavailable while the failure fact is being written, an
error retains `agent_session_id`, cursor, and
`recovery: "host_restart_reconcile"`. Do not create a replacement Session or
blindly retry sidecar launch; preserve the ID and restore the database/host.

## Canonical CLI

`nomicore` mirrors the same four REST operations:

```bash
export NOMIFUN_URL=http://127.0.0.1:25808
export NOMIFUN_ACCESS_TOKEN=<installation-access-token>

nomicore remote open <binding_id> \
  --initial-input '{"text":"hello"}' \
  --idempotency-key open-1

nomicore remote observe <agent_session_id> --after-seq 0 --limit 100

nomicore remote turn <agent_session_id> '{"text":"continue"}' \
  --idempotency-key turn-1

nomicore remote cancel <agent_session_id> \
  --idempotency-key cancel-1
```

If an idempotency key is omitted, the CLI prints and uses a generated one.
Reuse the printed key after an ambiguous network failure.

## Common failures

| Code | Meaning |
| --- | --- |
| `REMOTE_AUTH_REQUIRED` | Token is missing, invalid, or revoked. |
| `REMOTE_INVALID_REQUEST` | Unknown fields/query parameters or malformed input. |
| `REMOTE_BINDING_NOT_FOUND` | The binding does not exist for this owner. |
| `REMOTE_SESSION_NOT_FOUND` | The explicit Session is absent, foreign, or not Remote-owned. |
| `REMOTE_SESSION_OPENING` | Runtime admission has not reached a durable terminal state. |
| `REMOTE_OPEN_FAILED` | Runtime admission failed; inspect Session events. |
| `REMOTE_SESSION_BUSY` | The requested turn/cancel conflicts with current Session state. |
| `SNAPSHOT_EXECUTOR_UNAVAILABLE` | The frozen Snapshot cannot currently execute. |

## Related docs

- [Canonical Remote examples](./remote-capability-api-examples.md)
- [WebUI Remote Access](./webui-remote-access.md)
- [Web Server Deployment](./web-server-deployment.md)
- [Computer Use And Browser Use](./computer-browser-use.md)
