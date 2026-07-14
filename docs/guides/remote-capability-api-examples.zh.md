# Remote 能力 API · 对接示例

本文配套 [Remote 能力 API](./remote-capability-api.zh.md)。所有远程示例使用一枚绑定到具体伙伴的访问令牌；调用方只应使用实时发现到的 Remote 工具。

```bash
export HOST=127.0.0.1:25808
export TOKEN=<伙伴访问令牌>
```

## MCP 客户端

Claude Code、Cursor 或其他支持 Streamable HTTP 的 MCP 客户端可以配置：

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://127.0.0.1:25808/mcp-agent",
      "headers": {
        "Authorization": "Bearer <伙伴访问令牌>"
      }
    }
  }
}
```

`/mcp-agent` 是精简的干活工具面；只有确实需要完整平台控制能力时才使用 `/mcp`。连接后先调用 `tools/list`，以返回的名称和 JSON Schema 为准。

Python MCP SDK 示例：

```python
from mcp import ClientSession
from mcp.client.streamable_http import streamablehttp_client

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
            result = await session.call_tool(TOOL_NAME, arguments)
            print(result)
```

## curl / REST

先发现精简工具集：

```bash
curl -s "http://$HOST/v1/tools?profile=agent" \
  -H "Authorization: Bearer $TOKEN"
```

按发现结果中的工具名称和 schema 调用：

```bash
curl -s -X POST "http://$HOST/v1/tools/<tool_name>" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"argument":"value"}'
```

成功返回 `200 {"result": ...}`；参数或工具执行错误返回 `422`；未知工具返回 `404`；令牌无效返回 `401`。破坏性操作首次调用会返回 `409` 确认挑战。向用户清楚复述操作并得到明确授权后，再携带 `"confirm": true` 重试。

## SSE 流式调用

对支持进度的已发现工具使用流式端点：

```bash
curl -N -X POST "http://$HOST/v1/tools/<tool_name>/stream" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"argument":"value"}'
```

每个事件是一行 `data: <json>`；最后一个事件为：

```json
{"type":"__result__","data":{"result":{}}}
```

## Python REST

```python
import requests

base = "http://" + HOST
headers = {
    "Authorization": "Bearer " + TOKEN,
    "Content-Type": "application/json",
}

response = requests.post(
    f"{base}/v1/tools/{TOOL_NAME}",
    headers=headers,
    json=arguments,
)
print(response.json())
```

## nomicore CLI

```bash
export NOMIFUN_URL=http://$HOST
export NOMIFUN_COMPANION_TOKEN=$TOKEN

nomicore tools
nomicore call <tool_name> '{"argument":"value"}'
```

## OpenAPI 与自动化平台

OpenAPI 3.1 契约可用于生成类型客户端或导入 Postman、Insomnia、Bruno：

```bash
curl -s "http://$HOST/v1/openapi.json?profile=agent" \
  -H "Authorization: Bearer $TOKEN" > nomifun-openapi.json
```

n8n、Zapier、Make 等自动化平台可直接调用 `POST /v1/tools/{name}`。工具的 `name`、`description` 与 `input_schema` 均来自 `GET /v1/tools`；不要在客户端写死未发现的能力。

## Agent 协作契约的边界

持久化 Agent 协作只有一套按权限收敛的契约：`nomi_delegate` 创建 execution，`nomi_execution_get` 读取 execution，`nomi_execution_update` 修改计划或生命周期。可信本地所有者可以铸造安装所有者绑定的伙伴令牌，其 Remote 工具列表会包含这三项能力；Remote 委派会把伙伴记录为创建者，后续读取和修改只能访问该伙伴创建的 execution。次级用户在任何 surface 上都看不到这三项能力。

远程客户端应以 `tools/list` 的实际发现结果为准。伙伴令牌是安装所有者权限的高权限委派，不能交给不可信客户端，也不能借它访问其他伙伴或其他用户的 execution。

## 安全提醒

- 持有伙伴令牌近似拥有该实例的远程代码执行权限，只能交给可信客户端。
- 公网暴露时必须使用 TLS、网络访问控制和速率限制。
- 敏感能力默认不在 Remote surface 暴露。
- 令牌只能从可信本地上下文吊销或轮换；明文只在创建时显示一次。
