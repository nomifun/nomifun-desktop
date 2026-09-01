# Canonical Remote API · 对接示例

```bash
export HOST=127.0.0.1:25808
export TOKEN=<installation-access-token>
export BINDING_ID=<remote-binding-id>
```

## MCP 客户端

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

初始化后，`tools/list` 必须精确包含 `open`、`turn`、`observe`、`cancel`。

## REST：open

```bash
curl -s -X POST "http://$HOST/api/remote/open" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"binding_id\":\"$BINDING_ID\",
    \"idempotency_key\":\"open-1\",
    \"initial_input\":{\"text\":\"你好\"}
  }"
```

保存响应中的 `agent_session_id` 和 cursor。

## REST：observe

```bash
curl -s "http://$HOST/api/remote/observe?agent_session_id=$SESSION_ID&after_seq=0&limit=100" \
  -H "Authorization: Bearer $TOKEN"
```

后续使用返回的 `next_cursor.seq`；`after_seq` 是 exclusive cursor。

## REST：turn

仅在 observe 已证明 Session ready 后调用：

```bash
curl -s -X POST "http://$HOST/api/remote/turn" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"agent_session_id\":\"$SESSION_ID\",
    \"input\":{\"text\":\"继续\"},
    \"idempotency_key\":\"turn-1\"
  }"
```

## REST：cancel

```bash
curl -s -X POST "http://$HOST/api/remote/cancel" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"agent_session_id\":\"$SESSION_ID\",
    \"idempotency_key\":\"cancel-1\"
  }"
```

## Python REST

```python
import requests

BASE = f"http://{HOST}"
HEADERS = {"Authorization": f"Bearer {TOKEN}"}

opened = requests.post(
    f"{BASE}/api/remote/open",
    headers=HEADERS,
    json={
        "binding_id": BINDING_ID,
        "idempotency_key": "open-1",
        "initial_input": {"text": "你好"},
    },
).json()

session_id = opened["agent_session_id"]
observed = requests.get(
    f"{BASE}/api/remote/observe",
    headers=HEADERS,
    params={"agent_session_id": session_id, "after_seq": 0, "limit": 100},
).json()
print(observed)
```

## Python Streamable HTTP MCP

```python
from mcp import ClientSession
from mcp.client.streamable_http import streamablehttp_client

async def main():
    headers = {"Authorization": "Bearer " + TOKEN}
    async with streamablehttp_client(
        "http://%s/mcp" % HOST,
        headers=headers,
    ) as (read, write, _):
        async with ClientSession(read, write) as session:
            await session.initialize()
            tools = await session.list_tools()
            assert [tool.name for tool in tools.tools] == [
                "open", "turn", "observe", "cancel"
            ]
            opened = await session.call_tool(
                "open",
                {
                    "binding_id": BINDING_ID,
                    "idempotency_key": "mcp-open-1",
                    "initial_input": {"text": "你好"},
                },
            )
            print(opened)
```

## `nomicore` CLI

```bash
export NOMIFUN_URL=http://$HOST
export NOMIFUN_ACCESS_TOKEN=$TOKEN

nomicore remote open "$BINDING_ID" \
  --initial-input '{"text":"你好"}' \
  --idempotency-key open-1

nomicore remote observe "$SESSION_ID" --after-seq 0 --limit 100
nomicore remote turn "$SESSION_ID" '{"text":"继续"}' --idempotency-key turn-1
nomicore remote cancel "$SESSION_ID" --idempotency-key cancel-1
```

## Headless token 播种

```bash
export NOMIFUN_ACCESS_TOKEN="$(openssl rand -hex 32)"
nomifun-web --host 127.0.0.1 --port 8787
```

已删除的 `/mcp-agent`、`/v1` 和 `profile`/`domains` selector 应返回 route failure
或 `REMOTE_INVALID_REQUEST`；不要继续重试。
