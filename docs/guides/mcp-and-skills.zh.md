# MCP、技能与 Agent 工作台

> MCP 与 Skill 是平台能力供给/说明层；Agent 工作台（`/agent`）是唯一的 Agent
> authoring 入口。本文不再把 MCP、Skill 或旧 Preset 当作同一个产品配置对象。

NomiFun 有两种容易混淆的扩展机制：

- **MCP server** 是外部工具服务器，通过 stdio、HTTP 或 SSE 暴露可调用工具。
- **技能** 是 markdown/文件夹知识包，告诉 agent 如何完成某个工作流；它不是常驻工具服务器。

当前页面：

| 能力 | 页面 |
| --- | --- |
| MCP server 管理 | `/mcp` |
| 技能管理 | `/skills` |
| Agent 能力设计与试用 | `/agent` |
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

每条服务旁的启用/停用开关控制全局可用性，与连接检测分开。市场导入后仍默认停用；
补齐配置并检测成功后，手动打开开关即可在会话中勾选。也可以先启用再检测，但检测
成功且发现工具前不能在会话中新增选择。检测不会自动启用服务，切换开关也不会启动检测。
启停或检测完成后，已打开的会话 MCP 列表会重新读取后端目录，保留用户的会话选择草稿。

需要 OAuth 的 HTTP/SSE server 走 `/api/mcp/oauth/*` 流程。

当前 canonical owner 的协议边界是 Streamable HTTP/stdio `2025-03-26` 与显式 legacy SSE
`2024-11-05`。它不会把新旧 transport 静默互转，也尚未宣称支持 `2026-07-28` 的
`server/discover` / 无 initialize 生命周期；只支持该新生命周期的服务会返回明确的协议错误。
这是一次独立的 owner/安全模型迁移，不属于市场配置导入。

### MCP 市场的边界

MCP 市场是**配置目录**，不是包管理器。点击添加时，NomiFun 会从市场说明中选择更可移植的
MCP 配置候选（优先 `npx`/`uvx` 与 HTTPS，避开 Docker、全局命令和占位 endpoint），把
`streamableHttp`、`baseUrl` 等常见写法规范化，并以停用状态导入。它不会替用户安装 Node.js、
Docker、Python/uv、浏览器驱动，也不会完成第三方登录或生成 API Key。

导入确认页会显示实际命令、参数、URL、env/header 键和仍需填写的字段。含 `${...}`、
`<...>`、`xxxxx`、`YOUR_*` 等占位内容的 server 在补全前不能执行连接测试，因此不会误把
模板 URL 发到网络或误启动未配置的进程。已从市场导入但检测失败的条目可在市场中选择
“修复配置”；替换内容仍须再次确认，并会保持停用。

连接测试只代表该配置在检测时完成了 MCP 握手和 `tools/list`，不是常驻在线状态。失败时
UI 会区分缺少本地运行时、HTTP 状态、超时、RPC 与协议错误；URL 型服务只有在服务端明确
返回 OAuth Bearer challenge 时才进入 OAuth 登录流程，API Key/Header 配置不会被误判为 OAuth。

`npx`、`bunx`、`uvx` 等便携包运行器在手动检测时使用独立的 120 秒首次引导预算，普通 MCP
握手仍保持 30 秒预算。stdio 子进程通过统一代理策略只接入父进程代理变量或系统代理（并保留
失效本地代理探测），同时继续隔离父进程的 API Token 和无关环境变量。子进程原始 stderr
不会进入 API 或日志；后台
只保留无敏感信息的失败分类，例如包不存在、下载/网络失败、依赖缺失、配置缺失、权限问题或进程
提前退出。

指向 `localhost` 的 HTTP URL 只是客户端连接描述，NomiFun 不会代替第三方项目启动本地程序或
Docker 容器。本地探测失败时会明确显示 host/port 并提示先启动前置服务，不再把运行时、前置服务
或网络问题一律描述成“MCP JSON 配置错误”。

## 导入和同步外部 Agent 配置

`GET /api/mcp/agent-configs` 会探测已支持本地 agent CLI 的 MCP 配置。UI 可把探测到
的 server 导入 NomiFun，也可在 adapter 支持写入时把 NomiFun 的 MCP 列表同步回选中的
agent 配置。

这只是配置管理。某次会话最终能看到哪些 MCP server，仍由该会话的选择决定。

## 全局能力与会话选择

所有 Agent 预设共享已安装的技能库和全局 MCP 目录，Agent 工作台不提供技能、MCP 或工具搜索的总开关。
输入框旁的技能与 MCP 图标用于选择本会话使用哪些项目，不改变全局安装、启停状态，也不修改 Agent 预设。

新会话默认选中自动注入技能，以及已启用、连接测试成功且有工具的 MCP 服务器。用户可以取消默认选择，
或增加其它已安装技能和可用服务器。所有全局技能的正文与资源都冻结为会话可用的参考库存；选中的技能
随正式输入提供正文，较长的正文和辅助资源通过同一个原生上下文资源读取接口按需读取。
技能中的脚本、hooks 和工具声明不会增加 Agent 权限。

创建请求使用顶层 `session_capabilities`（`skill_names`、`mcp_server_ids`）；省略使用全局默认，
显式空数组表示不选。已有会话通过 `GET` / `PUT /api/agent-sessions/{id}/capability-selection`
读取和更新选择，更新必须携带 `expected_binding_version`。输入框在下次发送前等待配置保存成功；
失败时保留输入与选择并显示重试入口。

服务端把选择编译到同一不可变 Revision/Snapshot，并通过 canonical binding transition 应用。
运行中、暂停恢复、效果待核对、远程绑定和只读 Attempt 不能修改选择。切换 Agent 时保留会话选择；
工作区和其它已有资源授权不因扩展选择而改变。已有 accepted input 和 native checkpoint 不原地改写。

MCP 调用继续校验服务器启停、连接配置和工具 schema。连接身份按实际 transport 配置计算，
相同配置的重复测试、状态或描述修改不会使绑定变旧；配置、凭据或 schema 的实际变化仍需重新应用选择。

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

打开 `/skills`。

![技能页](../images/mcp-03-skills.png)

技能可以是单个 markdown 文件，也可以是包含 `SKILL.md` 的目录。

| 来源 | 含义 |
| --- | --- |
| Builtin | 随应用发布；部分会自动注入。 |
| Custom | 用户导入或放入配置目录。 |

技能可打标签、导入、导出/符号链接和扫描外部目录。自定义技能与内置技能使用同一会话冻结和读取路径。

技能市场的“添加”由技能库服务直接完成：服务端校验市场条目身份，下载并安全解压归档，再导入用户技能目录。该流程不会创建 Agent 会话，也不执行市场返回的命令；技能安装与之后在会话中使用技能是两个独立阶段。

## 技能 API

| 操作 | Endpoint |
| --- | --- |
| 列表 | `GET /api/skills` |
| 自动注入 builtin 列表 | `GET /api/skills/builtin-auto` |
| 标签 | `PUT /api/skills/{name}/tags` |
| 信息 / 路径 | `POST /api/skills/info`, `GET /api/skills/paths` |
| 导入 / 导出 / 删除 | `POST /api/skills/import`, `POST /api/skills/import-symlink`, `POST /api/skills/export-symlink`, `DELETE /api/skills/{name}` |
| 扫描 / 探测路径 | `POST /api/skills/scan`, `GET /api/skills/detect-paths`, `GET /api/skills/detect-external` |
| 外部路径 | `GET`, `POST`, `DELETE /api/skills/external-paths` |
| 技能市场 | `POST /api/skills/market/enable`, `POST /api/skills/market/disable`, `POST /api/skills/market/rankings/sync`, `POST /api/skills/market/install` |

## 相关

- [Agent 工作台](./presets.zh.md)
- [远程能力 API](./remote-capability-api.zh.md)
- [终端](./terminal.zh.md)
