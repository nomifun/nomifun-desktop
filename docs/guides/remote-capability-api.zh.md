# Remote 能力 API（外部伙伴 / MCP 接入指南）

NomiFun 把整个平台的能力（agent / browser / computer / 知识库 / 文件 / 以及平台控制）通过一个**网络可达、伙伴访问令牌鉴权的 MCP 端点**暴露出来。任何 MCP 客户端（Claude Code、Cursor、自研 LLM agent）填一个 URL + 一枚访问令牌，就能像"桌面伙伴"一样驱动平台——这就是"**外部伙伴**"。每枚令牌**绑定到一个具体伙伴**：持令牌调用即以该伙伴的身份运行，继承它的 profile 模型 / 人格 / 知识库，互不串扰。

> 📋 **可复制的对接示例**（MCP 客户端 / curl / Python / CLI / 自动化 / OpenAPI codegen / LLM 框架）见 **`remote-capability-api-examples.zh.md`**。

## ⚠️ 安全须知

持有伙伴访问令牌即可调用平台能力，**等价于授予远程代码执行（RCE）能力**（可驱动 agent、读写文件、操作 computer/browser）。因此：

- 只把令牌交给你信任的客户端/agent。
- 仅在可信网络暴露；公网暴露务必前置 TLS 反代 + 防火墙。
- 令牌可随时吊销/轮换（见下）；吊销只影响对应伙伴，其它伙伴的令牌不受影响。
- 默认安全栏：危险能力（`secret.*`、`system.factory_reset` 等）在 Remote 面被拒；破坏性操作需二次确认（协议级握手，见「权限模型」）。

## 端点

`/mcp`（MCP Streamable-HTTP）随后端进程内挂载，与 WebUI 共用监听器：

- **本机**：`http://127.0.0.1:<port>/mcp`（桌面应用的回环端口，或 `nomifun-web` 的服务端口）
- **局域网/远程**：开启 WebUI 远程访问后 `http://<你的IP>:25808/mcp`

鉴权：HTTP 头 `Authorization: Bearer <伙伴访问令牌>`。

## 一、获取伙伴访问令牌

令牌**只存哈希、明文只在铸造时返回一次**，且**绑定到一个具体伙伴**（`{id}` = 伙伴 id）。两种获取方式：

### 桌面应用（本机可信客户端）

桌面 webview 自带本地信任，可直接调本地端点（也会有 UI 入口）。下文 `<companion-id>` 为要绑定的伙伴 id：

```bash
# 铸造（返回明文一次，并绑定到该伙伴）
curl -X POST http://127.0.0.1:<loopback-port>/api/webui/companions/<companion-id>/access-token
# => {"success":true,"data":{"token":"<64位hex令牌>","companion_id":"<companion-id>"}}
#    若该伙伴尚无可用模型，data 还会带 "warning":"…"（令牌照常铸造，但 nomi_agent_run 等
#    需要模型的能力会失败，先去「模型管理」配置）

# 查询是否已配置（不返回令牌）
curl http://127.0.0.1:<loopback-port>/api/webui/companions/<companion-id>/access-token
# => {"success":true,"data":{"configured":true}}

# 吊销
curl -X DELETE http://127.0.0.1:<loopback-port>/api/webui/companions/<companion-id>/access-token
# => {"success":true,"data":{"configured":false}}
```

> 这些 `/api/webui/companions/{id}/access-token` 端点仅本地可信客户端可达（`require_local_trust`），远程浏览器拿不到。每个伙伴各持一枚令牌（再次铸造会覆盖旧令牌）。

### 无头服务器（headless `nomifun-web`）

无头部署用环境变量在启动时播种，**绑定到默认伙伴**（仅当该令牌尚未配置时生效，不覆盖已有；若实例中尚无任何伙伴会跳过并告警）：

```bash
NOMIFUN_COMPANION_TOKEN="$(openssl rand -hex 32)" \
  nomifun-web --host 127.0.0.1 --port 8787
```

把这串 hex 作为客户端的 Bearer 令牌。

## 二、连接 MCP 客户端

### Claude Code / 通用 MCP 客户端（Streamable-HTTP）

```json
{
  "mcpServers": {
    "nomifun": {
      "type": "streamable-http",
      "url": "http://127.0.0.1:25808/mcp",
      "headers": { "Authorization": "Bearer <伙伴访问令牌>" }
    }
  }
}
```

连上后 `tools/list` 即可看到平台在 Remote 面暴露的工具（`nomi_*`）；`tools/call` 驱动。

## 三、权限模型（Remote 面）

外部调用方落在 `Surface::Remote`，权限矩阵：

| 能力危险级 | Remote 行为 |
|---|---|
| 读 / 写 | 允许 |
| 破坏性（删除等） | 需确认：先返回 `{"needs_confirmation":true,...}`，agent 复述动作征得用户同意后，带 `"confirm": true` 重试 |
| 敏感（`secret.*` / `factory_reset`） | **拒绝**（默认不在 Remote 暴露） |

被拒的工具**不出现在 `tools/list`**（更好的 UX + 纵深防御）。

## 四、能力继承

平台能力通过同一条能力总线（`nomifun-gateway` 的 Capability Registry）暴露到 MCP/HTTP/CLI/Skill 等外部面。新增能力时，应同时评估它是否适合 Remote surface、是否需要确认，以及是否应进入 `/mcp-agent` 精简集。

调用方**以令牌所绑定的伙伴身份运行**：继承该伙伴的 profile 模型、人格与知识库，伙伴之间彼此隔离。因此 `nomi_agent_run` 在不显式指定 `model` 时，会解析所绑定伙伴的 profile 模型——**该伙伴必须配置好可用模型**（否则铸造时会返回 `warning`，且需要模型的能力会失败）。

## 当前可用面

- ✅ **MCP**：`/mcp`（全量 ~140 工具）+ `/mcp-agent`（curated 干活子集）。
- ✅ **委派目标**：`nomi_agent_run(goal,workspace?,model?,timeout_secs?)` 一句话把任务交给一个自治 nomi agent，跑完返回终稿；长任务返回 `{status:running}` 句柄，用 `nomi_agent_result(conversation_id)` 轮询。
- ✅ **HTTP REST**：`POST /v1/tools/{name}`、`GET /v1/tools[?profile=agent]`、`GET /v1/openapi.json[?profile=agent]`（OpenAPI 3.1，同令牌）。
- ✅ **CLI**：`nomicore tools`（离线列能力）、`nomicore call <name> [json]`、`nomicore agent "<目标>"`（读 `NOMIFUN_URL`/`NOMIFUN_COMPANION_TOKEN` 或 `--url`/`--token`）。
- ✅ **Skill**：`docs/skills/drive-nomifun/SKILL.md` —— 教外部 agent 如何连上并驱动 NomiFun（可发布到技能市场）。
- ✅ **Computer**：桌面版（`computer-use` 构建）暴露 `nomi_computer_*`（snapshot/click/type/key/scroll/launch/screenshot/…），外部调用方可驱动桌面（headless/web 构建不含）。
- ✅ **流式**：`POST /v1/tools/{name}/stream`（SSE）—— 流式工具（如 `nomi_agent_run`）实时吐 `{type:..}` delta，末帧 `{type:"__result__"}` 带终值；非流式工具仅末帧。`nomi_agent_run` 已流式（订阅 agent 广播逐条转发）。
