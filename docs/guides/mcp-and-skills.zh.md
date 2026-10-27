# MCP、技能与 Agent 工作台

> MCP 与 Skill 是平台能力供给/说明层；Agent 工作台（`/agent`）是唯一的 Agent
> authoring 入口。本文不再把 MCP、Skill 或旧 Preset 当作同一个产品配置对象。

NomiFun 有两种容易混淆的扩展机制：

- **MCP server** 是外部工具服务器，通过 stdio、HTTP 或 SSE 暴露可调用工具。
- **技能** 是 markdown/文件夹知识包，告诉 agent 如何完成某个工作流；它不是常驻工具服务器。

当前页面：

| 能力 | 页面 |
| --- | --- |
| MCP server 管理 | `/enhanced-tools?category=mcp` |
| 技能管理 | `/enhanced-tools?category=skills` |
| Agent 能力设计与试用 | `/agent` |
| 开放能力（MCP / REST） | `/connections?task=tools` |

MCP 与技能共享侧栏的“MCP & 技能”入口，各自保留数据与业务操作。搜索框跟随当前分类，
右上角“添加”菜单在 MCP 下提供从 JSON 导入、一键导入与 MCP 市场，在技能下提供
从 Agent 导入、导入技能、导入 zip 与技能市场。市场以 `view=market` 打开，关闭后回到原分类。
旧 `/mcp`、`/skills` 与 Settings URL 保留原市场选择、技能高亮和其他查询参数后重定向。

## MCP Server

打开“MCP & 技能 → MCP”（`/enhanced-tools?category=mcp`）可以新增、导入、测试和启用/禁用
MCP server。每行保留连接检测、配置、授权、删除和工具列表操作。

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

当前 canonical owner 的协议边界是 Streamable HTTP/stdio `2025-03-26` 与显式 legacy SSE
`2024-11-05`。它不会把新旧 transport 静默互转，也尚未宣称支持 `2026-07-28` 的
`server/discover` / 无 initialize 生命周期；只支持该新生命周期的服务会返回明确的协议错误。
这是一次独立的 owner/安全模型迁移，不属于市场配置导入。

### OAuth 登录

需要 OAuth 的 HTTP/SSE server 使用已保存的 server ID 发起登录。后端先读取服务端
Bearer challenge 与 Protected Resource Metadata，再按声明的 issuer 发现授权服务器；
客户端身份采用已注册客户端、服务器支持的 CIMD 或动态注册结果，不固定一个通用 client ID。
设置页创建一次性 attempt 后在用户浏览器打开授权地址，并查询该 attempt 的结果；取消、超时、
授权拒绝和 token 交换失败分别返回带阶段、错误码、重试建议与 trace ID 的安全诊断。

桌面原生入口在后端机器的 `127.0.0.1` 建立临时 loopback callback。远端 WebUI 不使用用户
浏览器机器的 loopback；运维需设置 `NOMIFUN_MCP_OAUTH_CALLBACK_URL` 为可从授权服务器
重定向到的 HTTPS 地址，路径必须为 `/api/mcp/oauth/callback`，并将该地址注册到客户端。
可选的产品控制 CIMD 地址使用 `NOMIFUN_MCP_OAUTH_CLIENT_METADATA_URL`，详见
[配置参考](../reference/configuration.zh.md#mcp-oauth)。

设置页的已保存连接测试与 Runtime 调用共享同一个安装级认证服务，登录后直接复用已保存的
授权并按需刷新，不把 token 写入 MCP JSON。授权状态与连接测试结果分别展示。
更换连接配置、注销或删除会取消该连接的待授权 attempt 并失效原 grant；描述修改、原样保存、
启用切换和普通检测不会注销。OAuth token 刷新或重新登录不要求重建 Agent Session。

升级时，旧的仅按 URL 保存、缺少 issuer/resource/client 归属证明的 OAuth 凭据会被清除，
需要重新登录。MCP 连接配置、其他非 Agent 配置和当前 Session 历史保留。


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

连接测试与正式调用共用有累计上限的接入策略；运行时准备、进程启动、握手和目录核验各有阶段预算，
切换阶段不会重新获得完整的累计预算。`npx`、`bunx`、`uvx` 等包运行器的首次启动阶段可等待
120 秒，普通协议接入阶段为 30 秒。这些期限不包含后续工具执行。

工具执行默认没有总时长或静默期限；HTTP 服务直到计算结束才返回响应头也可以正常等待。
用户取消、明确的执行截止时间、真实传输故障、协议或资源违规以及宿主关闭才结束业务等待。
日志、ping 或缺少 progress 都不证明工具已经卡死；进度通知使用有界处理，不按整个调用的累计数量
截断正常长任务。取消始终命中原调用，并等待本地清理结论；断流、取消通知和进程退出都不证明
此前没有外部效果，未知结果不会自动重放。调用方停止等待也不等于工具失败。

stdio 子进程通过统一代理策略只接入父进程代理变量或系统代理（并保留
失效本地代理探测），同时继续隔离父进程的 API Token 和无关环境变量。子进程原始 stderr
不会进入 API 或日志；后台
只保留无敏感信息的失败分类，例如包不存在、下载/网络失败、依赖缺失、配置缺失、权限问题或进程
提前退出。

失败探测不会用空目录覆盖同一配置最后一次完整验证的工具。应用来源的短暂超时或连接失败也保留
同一运行进程及地址的已验证目录，最新探测状态仍显示失败。新会话选择继续要求成功探测；已有冻结
调用仍由 owner 在真正派发前核验确切工具 schema，缓存不构成新的调用权限。

指向 `localhost` 的 HTTP URL 只是客户端连接描述，NomiFun 不会代替第三方项目启动本地程序或
Docker 容器。本地探测失败时会明确显示 host/port 并提示先启动前置服务，不再把运行时、前置服务
或网络问题一律描述成“MCP JSON 配置错误”。

## 导入外部 Agent 配置

`GET /api/mcp/agent-configs` 只读探测受支持的本地 agent CLI 与 NomiFun 的 MCP 配置。
UI 可将探测到的 server 导入 NomiFun；此入口不会写回外部 CLI 配置。
某次会话最终能看到哪些 MCP server，仍由该会话的选择决定。

## 全局能力与会话选择

所有 Agent 预设共享已安装的技能库和全局 MCP 目录，Agent 工作台不提供技能、MCP 或工具搜索的总开关。
输入框旁的技能与 MCP 图标用于选择本会话使用哪些项目，不改变全局安装、启停状态，也不修改 Agent 预设。

新会话默认选中自动注入技能；MCP 服务器需要明确勾选，启用或连接成功不会自动选入会话。
会话选择列表只显示已启用的 MCP，停用项仍可在 MCP 管理页查看。用户可以取消默认技能，
或增加其它已安装技能和可用服务器。所有全局技能的正文与资源都冻结为会话可用的参考库存；选中的技能
随正式输入提供正文，较长的正文和辅助资源通过同一个原生上下文资源读取接口按需读取。
技能中的脚本、hooks 和工具声明不会增加 Agent 权限。

创建请求使用顶层 `session_capabilities`（`skill_names`、`mcp_server_ids`）；省略时使用默认技能，
MCP 只保留 Agent 已明确保存的选择，不自动加入其它已启用服务器；显式空数组表示不选。
已有会话通过 `GET` / `PUT /api/agent-sessions/{id}/capability-selection`
读取和更新选择，更新必须携带 `expected_binding_version`。输入框在下次发送前等待配置保存成功；
失败时保留输入与选择并显示重试入口。

服务端把选择编译到同一不可变 Revision/Snapshot，并通过 canonical binding transition 应用。
运行中、暂停恢复、效果待核对、远程绑定和只读 Attempt 不能修改选择。切换 Agent 时保留会话选择；
工作区和其它已有资源授权不因扩展选择而改变。已有 accepted input 和 native checkpoint 不原地改写。

MCP 调用继续校验服务器启停、连接配置和工具 schema。连接身份按实际 transport 配置计算，
相同配置的重复测试、状态或描述修改不会使绑定变旧；transport 配置或工具 schema 的实际变化
仍需重新应用选择。OAuth grant 与 token 属于当前连接的凭据生命周期，不加入 Session Snapshot，
普通刷新和重新登录不改变冻结的 Agent 权限与 MCP 选择。

## MCP API

| 操作 | Endpoint |
| --- | --- |
| 列表 / 创建 | `GET`, `POST /api/mcp/servers` |
| 批量导入 | `POST /api/mcp/servers/import` |
| 获取 / 更新 / 删除 | `GET`, `PUT`, `DELETE /api/mcp/servers/{id}` |
| 启用切换 | `POST /api/mcp/servers/{id}/toggle` |
| 连接测试 | `POST /api/mcp/test-connection` |
| 探测 agent 配置 | `GET /api/mcp/agent-configs` |
| 认证状态 / 注销 | `GET`, `DELETE /api/mcp/servers/{id}/auth` |
| 创建 OAuth attempt | `POST /api/mcp/servers/{id}/auth/attempts` |
| 查询 / 取消 attempt | `GET`, `DELETE /api/mcp/servers/{id}/auth/attempts/{attempt_id}` |
| OAuth callback | `GET /api/mcp/oauth/callback`；由一次性 state 绑定原 attempt |

## 技能

打开“MCP & 技能 → 技能”（`/enhanced-tools?category=skills`），技能列表保留描述、来源标记、
详情查看与自定义技能删除操作。

技能可以是单个 markdown 文件，也可以是包含 `SKILL.md` 的目录。

| 来源 | 含义 |
| --- | --- |
| Builtin | 随应用发布；部分会自动注入。 |
| Custom | 用户导入或放入配置目录。 |

技能可打标签、导入、导出/符号链接和扫描外部目录。自定义技能与内置技能使用同一会话冻结和读取路径。

技能市场的“添加”由技能库服务直接完成：服务端校验市场条目身份，下载并安全解压归档，再导入用户技能目录。该流程不会创建 Agent 会话，也不执行市场返回的命令；技能安装与之后在会话中使用技能是两个独立阶段。

### 飞书 CLI 官方技能

内置 **飞书 CLI**（`lark-cli`）技能通过[官方 CLI](https://github.com/larksuite/cli/blob/v1.0.98/README.zh.md)
操作消息、文档、云空间、多维表格、电子表格、日历、任务、邮箱和知识库。
在会话技能列表中选择后按需使用，不自动注入。示例包括“打开这篇文档”“查询下周日程”
或“把这些数据写入指定多维表格”。

每个 Session 发现并复用现有程序的绝对路径，通过 canonical 进程工具传递字面参数。
工作台选择的程序路径和 profile 不会自动注入 Agent 调用。缺失组件时进入 `/office`，
点击右上角**连接设置**，在三步弹窗的**准备环境**中准备。
技能不再自行启动另一条 npm/npx 安装，也不获取 trust token 调用受保护的宿主 API。

用户明确启用后才按需供应 CLI，不打入 NomiFun 安装包。宿主先验证和复用现有兼容安装，
确证缺失后才准备固定版本的官方 npm 包。共享 prefix 独立于项目、运行时缓存和 Agent 数据。
`NOMIFUN_USER_CLI_ROOT` 是宿主提供的位置提示：

| 平台 | 共享 npm 前缀 | 启动器 |
| --- | --- | --- |
| Windows | `%LOCALAPPDATA%\nomifun-user-tools\npm` | `lark-cli.cmd` |
| macOS | `~/Library/Application Support/nomifun-user-tools/npm` | `bin/lark-cli` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/nomifun-user-tools/npm` | `bin/lark-cli` |

宿主识别官方 npm 包后使用已存在的原生程序。wrapper 缺少该程序时报告损坏，不运行它触发下载。
运行时升级和 Agent 数据清理保留共享安装。宿主供应器有共享 prefix 准备锁；技能文字不是锁，
不协调用户在外部终端自行执行的 npm 操作。

[办公工作台](office-hub.zh.md)提供三步连接、原生普通段落编辑与单次业务保存；
技能仍通过既有 canonical 进程执行 Agent 任务。业务命令通常使用 `ok`／`data` 包装，
配置、profile 和授权采用各自的原始 JSON 结构，以安装版本的真实命令合同为准。

凭据留在 CLI 自己的存储中。应用权限、用户授权和资源成员权限分别决定实际范围。
选择技能不会增加工具或飞书权限，停用工作台也不撤销服务端授权。
办公活动只展示工作台自己的操作，渠道机器人仍分别管理。

工作台提供完整官方向导命令 `npx @larksuite/cli@latest install` 供用户在自己终端显式执行。
该向导可能全局安装上游 Skills 并进入账号配置，不是静默前置。
工作台版本检查不安装或同步 Skills。共享组件的准备和兼容版本策略统一由工作台负责，
外部安装继续由原工具链与已核实 prefix 维护；明确更新请求可在原位置更新确定版本并验证同一程序。
没有所请求的升级路径时应说明限制，不能另装副本。内置技能内容仍随 NomiFun 应用更新。

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
