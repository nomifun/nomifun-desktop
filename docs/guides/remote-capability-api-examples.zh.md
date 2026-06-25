# Remote 能力 API · 对接示例 cookbook

> 配套 `remote-capability-api.zh.md`。所有示例用同一枚**伙伴访问令牌**（绑定到某个具体伙伴，调用即以该伙伴身份运行）；端点 = WebUI/LAN 端口（默认 `25808`）或 `nomifun-web` 的服务端口。下文用 `$HOST`/`$TOKEN` 占位。

## 0. 先决：拿到端点 + 令牌

- **端点**：`http://<你的实例IP>:25808`（开启 WebUI 远程访问后），或本机 `http://127.0.0.1:<port>`。
- **令牌（运维侧一次性发放，绑定到一个伙伴）**：
  - 桌面应用：WebUI/远程访问面板为某个伙伴点「生成访问令牌」（明文只显示一次）。
  - 无头服务器：启动时 `NOMIFUN_COMPANION_TOKEN=$(openssl rand -hex 32) nomifun-web --host 127.0.0.1 --port 8787`，绑定到默认伙伴，把这串 hex 当令牌。
  - 本机可信上下文（桌面 webview / dev NoAuth）可 `curl -X POST http://127.0.0.1:<port>/api/webui/companions/<companion-id>/access-token`（远程/普通 curl 会 403——铸造刻意只限本地可信）。
- 拿到后：`export TOKEN=<令牌>`；所有请求带 `Authorization: Bearer $TOKEN`。
- **以伙伴身份运行**：调用继承所绑定伙伴的模型/人格/知识库；`nomi_agent_run` 不带 `model` 时用该伙伴的 profile 模型，**所以该伙伴要先配置好可用模型**（否则铸造响应里会带 `warning`）。
- **能力发现**：`GET /v1/tools`（或 `/v1/tools?profile=agent` 精瘦集）列出所有工具名 + 描述 + JSON Schema；下文工具名以此为准（`nomi_agent_run` 一定有）。

---

## 1. MCP 客户端（Claude Code / Cursor / 任意 MCP Agent）—— 旗舰

最省事：把 NomiFun 作为一个 Streamable-HTTP MCP server 配进去。Claude Code / Cursor 的 `mcpServers`：

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://$HOST:25808/mcp-agent",
      "headers": { "Authorization": "Bearer $TOKEN" }
    }
  }
}
```

- `/mcp-agent` = curated「干活」工具集（agent/browser/computer/knowledge/files）；要全平台控制面用 `/mcp`。
- 连上后 `tools/list` 即见 `nomi_*` 工具，`tools/call` 驱动。委派整件事就调 `nomi_agent_run`。

Python 通用 MCP SDK：

```python
from mcp.client.streamable_http import streamablehttp_client
from mcp import ClientSession

async def main():
    headers = {"Authorization": "Bearer " + TOKEN}
    async with streamablehttp_client("http://%s:25808/mcp" % HOST, headers=headers) as (r, w, _):
        async with ClientSession(r, w) as s:
            await s.initialize()
            tools = await s.list_tools()
            res = await s.call_tool("nomi_agent_run", {"goal": "调研 X 并写 notes.md"})
            print(res)
```

---

## 2. curl（HTTP/REST，最通用）

```bash
# 列能力（精瘦 agent 档）
curl -s "http://$HOST:25808/v1/tools?profile=agent" -H "Authorization: Bearer $TOKEN"

# 委派一个目标（一句话把活交给一个自治 nomi agent）
curl -s -X POST "http://$HOST:25808/v1/tools/nomi_agent_run" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"goal":"调研竞品定价并写入 notes.md","timeout_secs":600}'
# => 200 {"result":{"conversation_id":123,"status":"completed","text":"..."}}
#    长任务 => {"result":{"conversation_id":123,"status":"running",...}}，之后轮询：
curl -s -X POST "http://$HOST:25808/v1/tools/nomi_agent_result" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"conversation_id":123}'

# 调任意能力（名字来自 /v1/tools）
curl -s -X POST "http://$HOST:25808/v1/tools/<tool_name>" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{...args...}'

# 危险操作：先返回 {"needs_confirmation":true,...}(HTTP 409) → 向用户复述后带 confirm 重试
curl -s -X POST "http://$HOST:25808/v1/tools/<tool>" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{...args..., "confirm": true}'
```

**结果信封**：成功 `200 {"result": <payload>}`；工具报错 `422 {"error":..}`；需确认 `409 {"needs_confirmation":true,..}`；未知工具 `404`；无/错令牌 `401`。

### 流式（SSE）

```bash
curl -N -X POST "http://$HOST:25808/v1/tools/nomi_agent_run/stream" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"goal":"..."}'
# 每行一个 data: JSON 事件（agent 的 text/tool_call delta），
# 末帧 data: {"type":"__result__","data":{"result":{...终值...}}}
```

---

## 3. Python（REST + SSE）

```python
import requests, json
BASE = "http://%s:25808" % HOST
H = {"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"}

# 调用
r = requests.post(f"{BASE}/v1/tools/nomi_agent_run", headers=H, json={"goal": "..."})
print(r.json())   # {"result": {...}} / {"error":...} / {"needs_confirmation":...}

# 流式
with requests.post(f"{BASE}/v1/tools/nomi_agent_run/stream", headers=H,
                   json={"goal": "..."}, stream=True) as resp:
    for line in resp.iter_lines():
        if line and line.startswith(b"data: "):
            ev = json.loads(line[6:])
            if ev.get("type") == "__result__":
                print("FINAL:", ev["data"]); break
            print("delta:", ev)
```

---

## 4. nomicore CLI（人/脚本）

```bash
export NOMIFUN_URL=http://$HOST:25808
export NOMIFUN_COMPANION_TOKEN=$TOKEN

nomicore tools                              # 离线列出 Remote 能力（无需运行实例）
nomicore call nomi_agent_run '{"goal":"..."}'
nomicore agent "调研竞品定价并总结"           # nomi_agent_run 的便捷包装
```

---

## 5. 任意 HTTP 自动化（n8n / Zapier / Make / shell 脚本）

把一个 HTTP 节点指向 `POST http://$HOST:25808/v1/tools/{name}`，Header `Authorization: Bearer $TOKEN`，Body = 该工具的 JSON 参数。零 SDK。

## 6. 从 OpenAPI 生成客户端

`GET http://$HOST:25808/v1/openapi.json[?profile=agent]` 是 OpenAPI 3.1 契约 —— 喂给 `openapi-generator` 生成任意语言的 typed client，或导入 Postman/Insomnia/Bruno。

## 7. 接进别的 LLM agent 框架（LangChain / OpenAI tool-calling / 自研）

`GET /v1/tools` 每个工具自带 `name` + `description` + `input_schema`（标准 JSON Schema）。把它们直接注册成你框架的工具列表；模型决定调用某工具时，转一发 `POST /v1/tools/{name}`（带 `confirm` 处理 409）。等于让 NomiFun 全平台能力即插即用地成为你 agent 的工具集。

---

## 备注

- **安全**：持令牌即全权（≈授予 RCE 等价能力）；只发给可信客户端，公网前置 TLS 反代，令牌可吊销（`DELETE /api/webui/companions/{id}/access-token`，只影响对应伙伴）。
- **MCP vs REST 选择**：agent/MCP 客户端用 `/mcp`(-agent)；脚本/自动化/其它语言用 `/v1`；要实时进度用 `/v1/tools/{name}/stream`(SSE) 或（MCP 端长任务）`nomi_agent_run` 的 `{status:running}` 句柄 + `nomi_agent_result` 轮询。
