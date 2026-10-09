# 后端 Crates

[`crates/backend/`](../../crates/backend/) 下的 `nomifun-*` crate 共同构成 HTTP/WS 服务器。它们一起编译进 `nomifun-app` 库 crate，并通过 `nomifun-app/src/main.rs` 生成 **`nomicore`** 二进制。两个宿主应用（`nomifun-desktop` 与 `nomifun-web`）直接链接 `nomifun-app`，并自行调用 `run_embedded_server` 或组合 `create_router`。本文不再把已删除的 `nomifun-preset` 列为当前 crate。

下方分组反映了 crate 在工作区清单（[`Cargo.toml`](../../Cargo.toml)）中相互依赖的方式。这并非严格的分层 DAG —— 部分功能 crate 之间存在依赖 —— 但它提供了一张与请求穿越服务器的路径相吻合的认知地图。

> Agent 与会话的当前实现边界见 [Agent Session 架构](agent-session.zh.md)。

## Agent 层依赖规则

正常的产品接缝是 [`nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent/)。需要 agent 概念的功能 crate 应尽量通过 `nomifun_ai_agent::{nomi_config, nomi_types}` 来消费它们。

存在有意为之、由 feature 控制的直接依赖例外：

- Browser/Computer 具体实现由 `nomifun-app` 的 canonical `AgentPlatform`
  Role host 持有；Platform Gateway 不依赖或注册具体桌面控制工具。

不要在未说明“为何无法走正常接缝或上述桥接面”的情况下，新增其他直接的 `nomi-*` 依赖。

## 核心、数据、实时、运行时

### v3 数据与 ID 契约

贡献者改动必须遵守
[数据与标识符规范](../contributing/data-and-identifier-standards.zh.md)。
所有后端 crate 遵循统一的 v3 数据契约：

- 每张 NomiFun 产品表都有 `id INTEGER PRIMARY KEY AUTOINCREMENT`，
  canonical Agent Store 表与 Unified Plugin 表两组例外按基线冻结使用
  TEXT/复合主键；
- 需要跨数据集稳定识别的实体使用 `user_id`、`conversation_id`、
  `message_id` 等具名裸标准 UUIDv7；
- 纯内部行只把表 `id` 作为 repository 实现细节，关系通过 owner UUIDv7、
  sequence、自然键或复合条件表达；
- 一条关系只保留一个引用字段，不存在 `*_row_id` 双轨字段；
- Repository/Service 维护带索引的逻辑关联；上述两组例外之外的产品 DDL
  不包含物理 `FOREIGN KEY`、`REFERENCES`、`ON DELETE` 或 `ON UPDATE`，
  除基线已注册的 guard trigger 外不新增数据库触发器；
- v3 启动时发现不兼容的受管数据集，会整体 reset/quarantine，而不是迁移
  历史行。

技术 `id` 只属于当前数据集，不得当作 API 或跨表身份。稳定 UUIDv7 在边界上
是字符串，外部协议标识保持不透明。

| Crate | 职责 |
| --- | --- |
| [`nomifun-common`](../../crates/backend/nomifun-common/) | `AppError`、错误链、各类枚举（`AgentType`、`ConversationStatus`、`MessageType`、`McpServerStatus` 等）、稳定业务 ID 的裸 UUIDv7 生成/校验、数据集 reset 辅助、AES-GCM `encrypt_string` / `decrypt_string`、`TimestampMs`、分页辅助、`constants::DEFAULT_HOST/DEFAULT_PORT/BODY_LIMIT/CSRF_*`。 |
| [`nomifun-api-types`](../../crates/backend/nomifun-api-types/) | HTTP 请求 / 响应 DTO、`WebSocketMessage` 信封，以及 AgentPreset、Capability Catalog、Agent Session、`agent_snapshot` 和 Nomi build-extras。前端 TypeScript 类型镜像该 crate。 |
| [`nomifun-db`](../../crates/backend/nomifun-db/) | 通过 `sqlx` 操作 canonical SQLite 数据集，维护 schema contract 与逻辑关联 registry，并为用户、会话、MCP、需求、cron、Agent execution、AgentPreset revision、安装访问令牌、知识库、渠道、连接器凭据、IDMM 介入、webhook 等提供仓储 trait 与 Sqlite 实现。Agent Session、Turn、Event、Effect 与 Message projection 由唯一 canonical Store 管理。 |
| [`nomifun-realtime`](../../crates/backend/nomifun-realtime/) | `WebSocketManager`、`BroadcastEventBus`，带 token 校验的 `/ws` 升级处理器，消息路由 trait，心跳计时，每连接缓冲常量。 |
| [`nomifun-runtime`](../../crates/backend/nomifun-runtime/) | 内嵌 Bun 的解压、缓存、命令发现与启动期 `PATH` 增强。子进程所有权统一属于 shared 层的 `nomi-process-runtime`。 |
| [`nomifun-assets`](../../crates/backend/nomifun-assets/) | 随服务器一同发布的内嵌静态资源（`include_dir!`）。 |

## 认证与会话

| Crate | 职责 |
| --- | --- |
| [`nomifun-auth`](../../crates/backend/nomifun-auth/) | JWT HS256（`JwtService`）、bcrypt 密码哈希、登录 / 登出 / 刷新 / 修改密码 / 初始化路由、`auth_middleware`、**CSRF 双提交 cookie** 中间件（cookie `nomifun-csrf-token`、header `x-csrf-token`）、安全响应头中间件、**限流**（auth / api / authenticated-action 等变体）、二维码登录 token 存储、`validate_username` / `validate_password`。为 handler 暴露 `CurrentUser`。 |

## Agent 接缝

| Crate | 职责 |
| --- | --- |
| [`nomifun-ai-agent`](../../crates/backend/nomifun-ai-agent/) | **通往 `crates/agent/` 的运行时桥梁。** 构建内置 `nomi` Agent runtime，由 `AgentRuntimeSessions` 管理进程内 runtime handles，广播 `AgentStreamEvent`，暴露运行时信息与工具接缝。再导出 `nomi_config` 和 `nomi_types` 供其余后端使用。 |
| [`nomifun-agent-contracts`](../../crates/backend/nomifun-agent-contracts/) | AgentPreset、Revision、ContributionLock、Snapshot、Role 和平台 Capability Catalog 的 canonical Rust/schema 合同；只依赖基础类型，不持有产品 service。 |
| [`nomifun-agent-control-plane`](../../crates/backend/nomifun-agent-control-plane/) | Agent 工作台的 owner-scoped application service：官方 seed、Draft、Preview、Save、Revision、Catalog 查询和 AgentBinding；调用 canonical Compiler，不安装或管理 Plugin。 |
| [`nomifun-agent-kernel`](../../crates/backend/nomifun-agent-kernel/) | 冻结执行闭包、typed resource binding 与调用准入。 |
| [`nomifun-agent-session`](../../crates/backend/nomifun-agent-session/) | 唯一 canonical Session、Turn、Event、Payload、Effect 存储及派生展示。 |
| [`nomifun-agent-runtime`](../../crates/backend/nomifun-agent-runtime/) | 唯一官方 Nomi Runtime 的模型、工具、计划、压缩与 native 恢复循环。 |
| [`nomifun-agent-domain-support`](../../crates/backend/nomifun-agent-domain-support/) | 打包 domain wave crate 共用的构造与校验辅助。 |
| [`nomifun-agent-domain-wave1`](../../crates/backend/nomifun-agent-domain-wave1/) | 目标代际 Web Research、Knowledge、Memory 模块注册（Wave 1）。 |
| [`nomifun-agent-domain-wave2`](../../crates/backend/nomifun-agent-domain-wave2/) | 打包的编码扩展能力注册（Wave 2）。 |
| [`nomifun-agent-domain-wave3`](../../crates/backend/nomifun-agent-domain-wave3/) | 打包的创作与多模态能力注册（C7 Wave 3）。 |
| [`nomifun-agent-domain-wave4`](../../crates/backend/nomifun-agent-domain-wave4/) | 打包的身份/渠道/设备能力注册（C7 Wave 4）。 |
| [`nomifun-agent-domain-wave5`](../../crates/backend/nomifun-agent-domain-wave5/) | 打包的自动化、监督与 Remote 域注册（C7 Wave 5）。 |
| [`nomifun-engine-core`](../../crates/backend/nomifun-engine-core/) | 源内集成执行引擎共享的 port；不提供策略、Session 库、插件加载或隐式授权。 |
| [`nomifun-chat-model-broker`](../../crates/backend/nomifun-chat-model-broker/) | Provider 中立的 Agent 会话 broker 与无状态本地 Responses 桥；唯一的模型重试/故障转移权威。 |
| [`nomifun-voice`](../../crates/backend/nomifun-voice/) | 应用侧 voice owner；canonical 工作经 port 注入。 |
| [`nomifun-voice-core`](../../crates/backend/nomifun-voice-core/) | Provider 无关的 voice 媒体/会话机制。 |
| [`nomifun-voice-contracts`](../../crates/backend/nomifun-voice-contracts/) | 可选 voice 合同；只引用 Agent 工作标识，不重复定义。 |

## 功能 crate（产品的主体）

| Crate | 职责 |
| --- | --- |
| [`nomifun-conversation`](../../crates/backend/nomifun-conversation/) | Canonical AgentSession 产品适配层与共享投影类型：session-owner port（`CanonicalAgentSessionOwner`）、创作入口、Creative Studio 画布 Agent session 绑定、product-agent snapshot 解析、turn 投递与模型故障转移队列服务。会话 HTTP 路由在 `nomifun-app` 的 router 中挂载。 |
| [`nomifun-agent-execution`](../../crates/backend/nomifun-agent-execution/) | 持久化 Agent 协作：`AgentExecutionEngine` 门面统一负责规划、依赖调度、Attempt、恢复、决策、事件和显式 Conversation 关联；单 Agent 与多 Agent 共用同一聚合。详见[统一执行架构](agent-execution.zh.md)。 |
| [`nomifun-mcp`](../../crates/backend/nomifun-mcp/) | MCP 服务器 CRUD、**OAuth 流程**、多 CLI 同步（`adapters/` 下的 `Claude`、`Codex`、`CodeBuddy`、`Gemini`、`Qwen`、`OpenCode`、`Nomi`、`Nomifun` 适配器）、连接测试、向会话注入 MCP 能力（含内置图像生成）。 |
| [`nomifun-skill-library`](../../crates/backend/nomifun-skill-library/) | Skill 库产品：内置与用户 Skill、市场发现、路由、启动物化与运行时解析。 |
| [`nomifun-channel`](../../crates/backend/nomifun-channel/) | 外部聊天渠道适配器（Telegram、Lark、DingTalk、WeChat）——通过 feature 控制。将入站消息映射到共享的 Agent / Conversation runtime，解析按机器人或平台配置的伙伴归属，并应用渠道 Agent 上下文。它是接入边界，不是额外的 Agent 类型或模式。 |
| [`nomifun-gateway`](../../crates/backend/nomifun-gateway/) | **平台 Gateway MCP** —— `nomi_*` 兼容工具（会话、定时任务、伙伴记忆、需求平台等）的进程内能力注册表与传输层。Browser/Computer 走 canonical `AgentPlatform` Role host，不由 Gateway 持有。内部子进程经 `nomicore mcp-gateway-stdio` 接入，只接收服务端派生、带作用域、有效期和签名的能力声明；Conversation 或 build-extra 字段都不能授权。公开入口只投影其鉴权边界允许的能力子集。 |
| [`nomifun-cron`](../../crates/backend/nomifun-cron/) | 定时任务：cron 表达式、时区修复、cron 守护进程、由斜杠命令驱动的创建。 |
| [`nomifun-requirement`](../../crates/backend/nomifun-requirement/) | **AutoWork 持久执行器** —— 后端驱动、支持 boot-resume 的持久循环。`RequirementServiceSink` 实现 `nomifun_common::RequirementCreator`，由 `nomifun-app` 注入按需启用的渠道消息转需求链路。 |
| [`nomifun-idmm`](../../crates/backend/nomifun-idmm/) | 智能决策模式（IDMM）：canonical AgentSession 的可选监督器，在提供商故障、模型静默与决策停滞时通过规则层和受限旁路模型恢复任务；不拥有第二套 Session/Runtime。详见[智能决策](../guides/intelligent-decision.zh.md)。 |
| [`nomifun-webhook`](../../crates/backend/nomifun-webhook/) | 外发飞书消息发送器，以及 Agent 工作完成时的 `CompletionNotifier`。 |
| [`nomifun-companion`](../../crates/backend/nomifun-companion/) | 桌面伙伴状态、形象 / 图片资源、记忆 / 人格数据、伙伴公开图片服务，以及机器人 / 设备绑定集成。 |
| [`nomifun-knowledge`](../../crates/backend/nomifun-knowledge/) | 知识库、来源摄取、绑定库挂载状态，以及作用域只读的知识 MCP 服务器。 |
| [`nomifun-workshop`](../../crates/backend/nomifun-workshop/) | Canonical 创作域：持有带版本项目文档、素材、提示词、模板/运行、严格 one-shot 模板草稿、Canvas ZIP 归档，以及大部分 owner-only `/api/creative-studio/*` 路由。项目文档、模板状态与素材 metadata 在 SQLite；二进制原件/缩略图在 `{data_dir}/workshop/assets/`。唯一公开面是供浏览器媒体元素使用的只读 `GET /api/creative-studio/files/{asset_id}`。 |
| [`nomifun-plugin-platform`](../../crates/backend/nomifun-plugin-platform/) | Unified Plugin Core：一种 Manifest/Package/Plugin 身份、内容寻址 Artifact、Draft、generation DataRoot、Action/Binding registry、独立 Service 进程、Surface Bridge、Package/Backup、回退，以及唯一 `/api/plugins` + `/api/plugin-drafts` HTTP 面。 |
| [`nomifun-plugin-development`](../../crates/backend/nomifun-plugin-development/) | 可选 Agent 模块：通过普通会话工具开发一个 Unified Plugin；Plugin 实例与运行时仍归 Plugin Core 所有。 |
| [`nomifun-js-runtime`](../../crates/backend/nomifun-js-runtime/) | Unified Plugin Core 唯一不可变的进程级 JavaScript Runtime 权威。 |
| [`nomifun-robot`](../../crates/backend/nomifun-robot/) | 机器人网关：LAN 接入的实体机器人（小智固件）作为桌面伙伴的物理化身。 |
| [`nomifun-creation`](../../crates/backend/nomifun-creation/) | 创作画布节点与模板 step 背后的媒体生成引擎。持有 canonical owner-only `/api/creative-studio/tasks*` 队列（`queued → running → succeeded/failed/canceled`）、exact Provider/model/task/输入身份、Provider 级与全局并发、取消与启动对账；模型执行委托 `nomifun-model-invoke`，产物字节交给 Workshop `AssetSink`。 |
| [`nomifun-customer-service`](../../crates/backend/nomifun-customer-service/) | 客服独立域：面向 IM 渠道陌生人的独立服务域，与伙伴 / 会话体系不共享概念——对话是本域自己的聚合，回复由一次性引擎会话产出，工具注册表固定为只读三件套。 |
| [`nomifun-public`](../../crates/backend/nomifun-public/) | 安装令牌鉴权的 canonical Remote MCP adapter：挂载于 `/mcp`，只通过 `AgentPlatform`/`AgentSession` 暴露 `open/turn/observe/cancel`。 |
## 基础设施特性

| Crate | 职责 |
| --- | --- |
| [`nomifun-terminal`](../../crates/backend/nomifun-terminal/) | 基于 `portable-pty` 的终端会话，支持 resize，通过 WS 进行输入 / 输出流式传输。 |
| [`nomifun-ssh`](../../crates/backend/nomifun-ssh/) | SSH 远程会话：加密的 owner 作用域主机簿（`ssh_hosts`）、连接池/provider、`/api/ssh-hosts` 路由，以及给 SSH 绑定会话的 Agent 提供远程工具族的 `SshBackend` sink。传输层在隔离的共享 crate `nomi-ssh`（`russh`/`russh-sftp`）。 |
| [`nomifun-browser-macos`](../../crates/backend/nomifun-browser-macos/) | Browser Workspace 的 macOS 14+ 系统 WKWebView 适配器；Windows 保留独立 WebView2 宿主。 |
| [`nomifun-browser-platform`](../../crates/backend/nomifun-browser-platform/) | 会话持有的原生 Browser Workspace 类型契约：run/输入所有权、Tab 与 runtime generation、snapshot、上传/下载、Profile 清理，以及 Attached Chrome Provider 的授权模型。桌面宿主提供原生 WebView；隔离搜索/渲染与附着个人浏览器保持为不同消费者。 |
| [`nomifun-model-invoke`](../../crates/backend/nomifun-model-invoke/) | 统一多模态模型调用层：类型化任务请求 / 结果、声明式鉴权方案、共享 HTTP 传输、协议适配器接缝 + 注册表与模型目录解析管线；被 `nomifun-shell` STT/TTS、`nomifun-creation` 等模型调用方消费。 |
| [`nomifun-shell`](../../crates/backend/nomifun-shell/) | 操作系统外壳辅助：用系统应用打开文件，针对 Deepgram 或 OpenAI 的语音转文字，剪贴板 / 粘贴集成。 |
| [`nomifun-file`](../../crates/backend/nomifun-file/) | 在会话工作目录下的沙箱化文件系统（`browse`、`path_safety`、`watch_service`、`snapshot_service`），zip 辅助。 |
| [`nomifun-office`](../../crates/backend/nomifun-office/) | LibreOffice 转换 / 预览管线（Office 文档 → 预览）。 |
| [`nomifun-system`](../../crates/backend/nomifun-system/) | LLM provider / 模型查询、应用级设置、sysinfo、应用版本检查 / 自更新框架。 |

## 组合根：`nomifun-app`

[`nomifun-app`](../../crates/backend/nomifun-app/) 是两个宿主二进制所链接的 crate。其结构如下：

| 模块 | 角色 |
| --- | --- |
| `cli.rs` | 顶层 `nomicore` clap 解析器：`--host/--port/--data-dir/--work-dir/--app-version/--local/--log-dir/--log-level`，加上子命令 `mcp-requirement-stdio`、`mcp-knowledge-stdio`、`mcp-gateway-stdio`、`mcp-open-stdio`、`terminal-hook`、`doctor`、`remote`（`open`/`turn`/`observe`/`cancel`）、`backup`、`restore`。Web 宿主调用 `Cli::parse_from(["nomifun-web"])` 取得带默认值的实例，然后覆盖自身关心的项。 |
| `bootstrap/` | 分层初始化：`tracing_init`（文件 + 控制台层）、`work_dir` 解析、`builtin_skills` 物化、`environment::{init_environment,init_data_layer}`、`admin::ensure_admin_credentials`（认证模式下的首次运行预置）。 |
| `services.rs` | `AppServices` 大杂烩：每个功能 crate 的服务带着对应仓储一并接好。通过 `AppServices::from_config(database, &config)` 一次构建。 |
| `router/` | `create_router(&services)` 以及类型化的 `routes`、`state`、`health`、`trace` 辅助；`build_module_states` / `build_skill_state` / `build_ws_state`。Agent platform/control-plane、conversation 和其它模块分别注册自己的路由。 |
| `commands/` | CLI 子命令的实现体：服务器、各 stdio MCP bridge、终端生命周期 hook、诊断，以及公开能力客户端命令。 |
| `lib.rs` | 公共门面：`run_embedded_server`、`AppServices`、`create_router`、`bootstrap` 再导出。这是宿主二进制唯一引入的 API。 |

## 在哪里检查依赖规则

如果你想自行检查直接的 `nomi-*` 依赖，可以扫描每个后端 crate 的清单：

```sh
# from the repo root, on a Unix shell
rg -l 'nomi-[a-z-]+\s*=' crates/backend/*/Cargo.toml
```

预期会看到主接缝（`nomifun-ai-agent`）以及上文描述的、由 feature 控制的桥接例外。
