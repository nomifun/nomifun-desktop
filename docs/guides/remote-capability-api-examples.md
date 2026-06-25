# Remote Capability API Examples

These examples use one companion access token bound to one companion. Replace
`$HOST` with your NomiFun host and `$TOKEN` with the token shown when it was
created.

```bash
export HOST=127.0.0.1:25808
export TOKEN=<companion-access-token>
```

## MCP Client

For Claude Code, Cursor, or any MCP client that supports Streamable HTTP:

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://$HOST/mcp-agent",
      "headers": {
        "Authorization": "Bearer $TOKEN"
      }
    }
  }
}
```

Use `/mcp-agent` for the curated worker surface. Use `/mcp` only when you need
the broader platform-control surface.

## curl

List curated tools:

```bash
curl -s "http://$HOST/v1/tools?profile=agent" \
  -H "Authorization: Bearer $TOKEN"
```

Delegate a task to an autonomous NomiFun agent:

```bash
curl -s -X POST "http://$HOST/v1/tools/nomi_agent_run" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"goal":"Research competitor pricing and write notes.md","timeout_secs":600}'
```

Poll a long-running delegated task:

```bash
curl -s -X POST "http://$HOST/v1/tools/nomi_agent_result" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"conversation_id":123}'
```

Call any discovered tool:

```bash
curl -s -X POST "http://$HOST/v1/tools/<tool_name>" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"argument":"value"}'
```

For a confirmation-required destructive action, first show the returned
challenge to the user. Retry only after explicit approval:

```bash
curl -s -X POST "http://$HOST/v1/tools/<tool_name>" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"argument":"value","confirm":true}'
```

## SSE Streaming

```bash
curl -N -X POST "http://$HOST/v1/tools/nomi_agent_run/stream" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"goal":"Summarize this repository"}'
```

The final event is:

```json
{"type":"__result__","data":{"result":{}}}
```

## Python REST

```python
import requests

base = f"http://{HOST}"
headers = {
    "Authorization": f"Bearer {TOKEN}",
    "Content-Type": "application/json",
}

response = requests.post(
    f"{base}/v1/tools/nomi_agent_run",
    headers=headers,
    json={"goal": "Research competitor pricing and write notes.md"},
)
print(response.json())
```

## Python Streamable HTTP MCP

```python
from mcp.client.streamable_http import streamablehttp_client
from mcp import ClientSession

async def main():
    headers = {"Authorization": "Bearer " + TOKEN}
    async with streamablehttp_client(
        "http://%s/mcp-agent" % HOST,
        headers=headers,
    ) as (read, write, _):
        async with ClientSession(read, write) as session:
            await session.initialize()
            tools = await session.list_tools()
            print([tool.name for tool in tools.tools])
            result = await session.call_tool(
                "nomi_agent_run",
                {"goal": "Research competitor pricing and write notes.md"},
            )
            print(result)
```

## Headless Server Token Seed

For a local headless server:

```bash
export NOMIFUN_COMPANION_TOKEN="$(openssl rand -hex 32)"
nomifun-web --host 127.0.0.1 --port 8787
```

For LAN or public access, finish admin setup first, bind intentionally, and
place the server behind TLS:

```bash
nomifun-web --host 0.0.0.0 --port 8787
```

## OpenAPI

Generate a typed client from:

```bash
curl -s "http://$HOST/v1/openapi.json?profile=agent" \
  -H "Authorization: Bearer $TOKEN" > nomifun-openapi.json
```

## Notes

- MCP clients should prefer `/mcp-agent`.
- Scripts and automation systems can use `/v1/tools/{name}` directly.
- Use `/v1/tools/{name}/stream` when live progress matters.
- Tokens can be revoked with
  `DELETE /api/webui/companions/{id}/access-token` from a trusted local
  desktop context.
