# MCP 与技能

NomiFun 有两种容易混淆的扩展机制：

- **MCP server** 是外部工具服务器，通过 stdio、HTTP 或 SSE 暴露可调用工具。
- **技能** 是 markdown/文件夹知识包，告诉 agent 如何完成某个工作流；它不是常驻工具服务器。

当前页面：

| 能力 | 页面 |
| --- | --- |
| MCP server | `/mcp` |
| 技能 | `/assistants?tab=skills` |
| 助手 | `/assistants?tab=assistants` |
| 对外能力暴露 | `/open-capabilities` |

旧 Settings URL 会重定向到这些页面。

## MCP Server

打开 `/mcp` 可以新增、导入、测试、启用/禁用和同步 MCP server。

![MCP 页面](../images/mcp-01-capabilities.png)

每条 server 记录包含：

- 名称；
- transport：`stdio`、`http` 或 `sse`；
- stdio 的 command / args / env，或 HTTP/SSE 的 URL；
- 从其他 agent 配置导入时保留的 raw JSON；
- enabled 状态；
- 最近一次连接测试结果。

连接测试会启动临时 MCP client，完成握手、列出工具并持久化结果。失败码覆盖命令
不存在、权限、超时、HTTP、RPC 和协议错误。

需要 OAuth 的 HTTP/SSE server 走 `/api/mcp/oauth/*` 流程。

## 导入和同步 Agent 配置

`GET /api/mcp/agent-configs` 会探测已支持本地 agent CLI 的 MCP 配置。UI 可把探测到
的 server 导入 NomiFun，也可在 adapter 支持写入时把 NomiFun 的 MCP 列表同步回选中的
agent 配置。

这只是配置管理。某次会话最终能看到哪些 MCP server，仍由该会话的选择决定。

## 每会话选择

全局启用 MCP server 只是让它可用，不会自动注入每个 agent。会话启动时最终 MCP 列表来自：

- 全局 enabled server；
- 该会话选择的 server；
- 当前能力集需要的 builtin bridge server。

最终列表会进入 agent session start payload。

## MCP API

| 操作 | Endpoint |
| --- | --- |
| 列表 / 创建 | `GET`, `POST /api/mcp/servers` |
| 批量导入 | `POST /api/mcp/servers/import` |
| 获取 / 更新 / 删除 | `GET`, `PUT`, `DELETE /api/mcp/servers/{id}` |
| 启用切换 | `POST /api/mcp/servers/{id}/toggle` |
| 连接测试 | `POST /api/mcp/test-connection` |
| 探测 agent 配置 | `GET /api/mcp/agent-configs` |
| OAuth | `POST /api/mcp/oauth/check-status`, `/login`, `/logout`; `GET /api/mcp/oauth/authenticated` |

## 技能

打开 `/assistants?tab=skills`。

![技能页](../images/mcp-03-skills.png)

技能可以是单个 markdown 文件，也可以是包含 `SKILL.md` 的目录。

| 来源 | 含义 |
| --- | --- |
| Builtin | 随应用发布；部分会自动注入。 |
| Custom | 用户导入或放入配置目录。 |
| Extension | 已安装扩展提供。 |

技能可打标签、导入、导出/符号链接、扫描外部目录，也可按某个 agent 后端进行
materialize。

## 技能 API

| 操作 | Endpoint |
| --- | --- |
| 列表 | `GET /api/skills` |
| 自动注入 builtin 列表 | `GET /api/skills/builtin-auto` |
| 标签 | `PUT /api/skills/{name}/tags` |
| 信息 / 路径 | `POST /api/skills/info`, `GET /api/skills/paths` |
| 导入 / 导出 / 删除 | `POST /api/skills/import`, `POST /api/skills/import-symlink`, `POST /api/skills/export-symlink`, `DELETE /api/skills/{name}` |
| 扫描 / 探测路径 | `POST /api/skills/scan`, `GET /api/skills/detect-paths`, `GET /api/skills/detect-external` |
| 为 agent materialize | `POST /api/skills/materialize-for-agent` |
| 助手规则/技能文件 | `/api/skills/assistant-rule/*`, `/api/skills/assistant-skill/*` |
| 外部路径 | `GET`, `POST`, `DELETE /api/skills/external-paths` |
| 技能市场 | `POST /api/skills/market/enable`, `POST /api/skills/market/disable` |

## 相关

- [助手](./assistants.zh.md)
- [远程能力 API](./remote-capability-api.zh.md)
- [终端](./terminal.zh.md)
