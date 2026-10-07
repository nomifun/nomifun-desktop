---
name: drive-nomifun
description: >-
  Connect an authorized external client to NomiFun's canonical Remote ingress.
  Use an installation access token and a local-owner-created RemoteBinding,
  then operate the explicit open/turn/observe/cancel AgentSession lifecycle.
---

# Drive NomiFun through canonical Remote

NomiFun Remote is an ingress into the canonical AgentSession aggregate. It is
not a generic platform capability registry and does not infer a Session from an
MCP connection, token, IP address, companion, or recent activity.

## Required inputs

Obtain from the NomiFun installation owner:

- the `/mcp` or HTTP base URL;
- the installation access token;
- a `remote_binding_id` created from the exact desired Agent Preset revision.

Never request or expose provider credentials. Protect the installation token as
a high-privilege secret.

## MCP configuration

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://<host>:25808/mcp",
      "headers": {
        "Authorization": "Bearer <installation-access-token>"
      }
    }
  }
}
```

Reject configuration drift if `tools/list` is not exactly:

- `open`
- `turn`
- `observe`
- `cancel`

Do not use the removed `/mcp-agent`, `/v1`, `profile`, or `domains` surfaces.

## Operating sequence

1. Call `open` with `binding_id`, a stable `idempotency_key`, and optional
   `initial_input`.
2. Persist the returned `agent_session_id` and cursor. The MCP transport session
   ID is not the product identity.
3. If `open_state` is `opening`, call `observe` with the same
   `agent_session_id` and returned cursor until Session events prove `ready` or
   `open_failed`.
4. Call `turn` only on the explicit ready Session. Reuse its idempotency key
   after an ambiguous network failure.
5. Use `observe` with the returned `next_cursor`; cursors are exclusive.
6. Call `cancel` only when the Session has an active turn.

Binding updates affect only later opens. Binding deletion does not rewrite or
cancel an existing Session.

## Failure handling

- `REMOTE_AUTH_REQUIRED`: stop and ask the owner to provide/rotate the token.
- `REMOTE_INVALID_REQUEST`: fix the exact schema; never add selector fields.
- `REMOTE_SESSION_OPENING`: preserve the Session ID and cursor. If details say
  `host_restart_reconcile`, ask the owner to restore storage/host health.
- `REMOTE_OPEN_FAILED`: inspect observed Session events; do not treat it as
  ready.
- `REMOTE_SESSION_BUSY`: observe current state before issuing another mutation.
- `SNAPSHOT_EXECUTOR_UNAVAILABLE`: report the frozen Snapshot/Runtime
  availability blocker to the owner.

There is no confirmation retry protocol. Remote is FullAuto within the frozen
Snapshot authority.
