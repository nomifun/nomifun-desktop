# ID 体系

本文档是 NomiFun 标识符的权威契约，适用于数据库、Rust 领域模型、HTTP/WebSocket/MCP 协议、运行时注册表、文件系统、备份与导入。

## 目标

所有持久化且可被其他记录引用的实体统一使用：

```text
{已注册前缀}_{规范小写连字符 UUIDv7}
```

例如：

```text
conv_019bffff-ffff-7abc-8def-0123456789ab
```

实体 ID 在 JSON 中永远是字符串，在 SQLite 中永远是 `TEXT`。它不能是 SQLite `rowid`、自增整数、时间戳或本地序号。数据库重建、跨设备迁移、备份恢复和合并导入都不得改变实体 ID。后端使用 `Uuid::now_v7()` 的完整 128 bit UUID，不截断、不使用自定义短 ID。

## ID 分类

### 1. 实体 ID

实体 ID 标识持久化、可引用的对象。它必须使用带已注册前缀的 UUIDv7，并在 Rust 边界使用领域 newtype。普通恢复或合并导入保留原 ID；同一 ID 内容不同必须失败且原子不变。只有显式“克隆”操作可以生成新 ID，并通过完整 old → new remap 重写全部引用。

### 2. 外部 ID

ACP session ID、平台消息 ID、远程任务 ID、provider request ID 等由外部系统签发。它们保持不透明，并使用能说明签发方和用途的字段名；不得直接作为 NomiFun 实体主键。

### 3. 自然键

Skill 名称、extension slug、URL、模型名和配置 key 属于自然键，不是实体 ID。不要仅因为它们参与查询或唯一约束就命名为 `*_id`。

### 4. 操作键与幂等键

请求关联 ID、幂等键、capability nonce 和临时 operation token 标识一次动作而非持久化实体。无前缀的 `generate_id()` 仅允许用于这类场景。若操作最终创建实体，必须另行生成 typed entity ID。

### 5. 不是 ID 的数值

revision、sequence、分页 offset、计数、时间戳、进程 PID、端口以及 JSON-RPC 请求号继续使用数值，但应按真实含义命名。

## 严格规范格式

实体 ID 必须满足：

- 前缀长度为 1–32 个 ASCII 字符；
- 首字符为 `a-z`；
- 后续字符只能是 `a-z` 或 `0-9`；
- 前缀与 UUID 之间恰好一个 `_`；
- UUID 必须是小写、带连字符、RFC 4122 variant、version 7；
- 禁止空白、大写、花括号、紧凑 UUID、其他分隔符和 JSON number。

非法 ID 必须立即报错，绝不能降级为 `0`、空字符串或另一种实体 ID。

## Rust API

`nomifun-common` 统一负责生成、校验和 typed newtype：

```rust
let id = ConversationId::new();
let parsed: ConversationId = text.parse()?;
```

Typed ID 以透明 JSON 字符串序列化，但反序列化时严格校验前缀、UUID 格式、版本和 variant。因此 `ConversationId` 无法从 Terminal ID 或 JSON number 反序列化。

实现保持轻量：每种持久化实体一个小 newtype，由 `nomifun-common/src/id.rs` 内部宏生成，不引入复杂的泛型 ID 框架。

## 已注册前缀表

前缀是永久协议值。一旦使用，不得重命名、复用或分配给第二种实体。下表覆盖 ID-contract-v2 主数据库及参与备份/恢复的 companion 存储中全部持久化、可引用实体。

| 前缀 | Rust 类型 | 实体 |
| --- | --- | --- |
| `user` | `UserId` | 用户账户 |
| `conv` | `ConversationId` | 会话 |
| `term` | `TerminalId` | 终端会话 |
| `req` | `RequirementId` | Requirement / AutoWork 项 |
| `msg` | `MessageId` | 会话消息 |
| `artifact` | `ConversationArtifactId` | 会话 Artifact |
| `mcp` | `McpServerId` | 已配置 MCP Server |
| `ragent` | `RemoteAgentId` | Remote Agent 配置 |
| `webhook` | `WebhookId` | 出站 Webhook |
| `prov` | `ProviderId` | Provider 配置 |
| `agent` | `AgentId` | 用户/自定义 Agent |
| `preset` | `PresetId` | 用户创建的 Preset |
| `presettag` | `PresetTagId` | 用户创建的 Preset Tag |
| `kb` | `KnowledgeBaseId` | 知识库 |
| `kbind` | `KnowledgeBindingId` | 知识绑定 |
| `att` | `AttachmentId` | Requirement 附件 |
| `conn` | `ConnectorCredentialId` | Connector 凭据 |
| `cron` | `CronJobId` | 定时任务 |
| `cronrun` | `CronJobRunId` | 定时任务运行记录 |
| `idmmrec` | `IdmmInterventionId` | IDMM 干预记录 |
| `aext` | `AgentExecutionTemplateId` | Agent Execution 模板 |
| `aetp` | `AgentExecutionTemplateParticipantId` | 模板参与者 |
| `exec` | `AgentExecutionId` | Agent Execution |
| `execpart` | `AgentExecutionParticipantId` | Execution 参与者 |
| `execstep` | `AgentExecutionStepId` | Execution 步骤 |
| `eattempt` | `AgentExecutionAttemptId` | Execution 尝试 |
| `aevt` | `AgentExecutionEventId` | Execution 事件 |
| `execlink` | `ConversationExecutionLinkId` | 会话/Execution 关联 |
| `chn` | `ChannelId` | Channel Plugin |
| `chu` | `ChannelUserId` | Channel 用户 |
| `chs` | `ChannelSessionId` | Channel 会话 |
| `companion` | `CompanionId` | Companion 配置 |
| `mem` | `CompanionMemoryId` | Companion 记忆 |
| `sug` | `CompanionSuggestionId` | Companion 建议 |
| `plr` | `CompanionLearnRunId` | Companion 学习运行 |
| `csw` | `CompanionSessionWindowId` | Companion 会话窗口 |
| `figure` | `FigureId` | Companion 形象库条目 |
| `audit` | `PublicAgentAuditEntryId` | Public Agent 审计记录 |
| `evf` | `CompanionEvolutionFeedbackId` | Companion 进化反馈记录 |
| `pubagent` | `PublicAgentId` | Public Agent 配置 |
| `wsc` | `WorkshopCanvasId` | Workshop 画布 |
| `wsa` | `WorkshopAssetId` | Workshop 资产 |
| `wst` | `CreationTaskId` | Workshop 创建任务 |
| `wsn` | `WorkshopNodeId` | 持久化 Workshop 画布节点 |
| `wse` | `WorkshopEdgeId` | 持久化 Workshop 画布边 |

内置 Agent 目录行（`agent_builtin_*`）和 Extension 提供的 Preset key 是稳定的**安装自然键**，不是 UUID 实体 ID，因此不会解析为 `AgentId`/`PresetId`；用户创建的记录使用 `agent_`/`preset_`。

`preset_tag_bindings.tag_key` 是明确的字符串联合类型，不是无类型的单一实体外键：

- 以保留命名空间 `presettag_` 开头的值是规范 `PresetTagId`，对应 `preset_tags` 中的用户标签行，且 `dimension` 必须与该行一致；
- 其余值是 `office`、`coding` 等稳定的内置 manifest 词表自然键，因此不存在对应的 `preset_tags` 数据库行。

因此该列使用 `TEXT` 但不能声明单一 SQLite 外键。消费方必须先按联合类型分支：不得把内置自然键强制解析为 `PresetTagId`，也不得仅因 `presettag_` 值是字符串就把它当成宽松自然键。

短生命周期的操作/传输标识目前包括 `browseroob`、`wso` 以及只返回给当前调用方的 evolution-run token。该 evolution token 由 `generate_id()` 生成为无前缀 UUIDv7；`evr` 明确不注册为持久前缀，也没有 typed entity newtype。`client` 值属于外部协议 locator，不是 NomiFun 实体。禁止将它们直接提升为数据库主键或备份实体引用；若未来变为持久实体，必须先注册前缀并增加 typed newtype。

“持久化”不限于主 SQLite：`figure` 同时写入 `figures/index.json` 和图片文件名，`audit` 写入保留期 JSONL，`evf` 是 companion store 主键，`wsn`/`wse` 写入并导出 `canvas.json`。因此它们都遵守同一 canonical entity-ID 契约，并必须参与 clone/import remap。

## 存储与协议规则

- 数据库实体 PK/FK 原样保存完整字符串。
- HTTP path、WebSocket 事件、MCP 参数、缓存 key 和 owner manifest 使用同一值，不得转为整数。
- 文件目录可以直接使用规范实体 ID；若同时带可读名称，ID 仍是权威部分。
- 多态引用使用 tagged union 或拆分后的 typed FK，禁止裸 `(kind, integer)`。
- 事件使用强类型 DTO，避免散落的 `json!` 让同一 ID 以不同 JSON 类型序列化。
- Workshop 画布的 payload 语义仍由前端拥有，但后端在每次读写时严格校验持久身份包络：`nodes[].id` 和 `groupId` 必须是规范 `wsn` ID，`edges[].id` 必须是规范 `wse` ID，边端点与 `node:<wsn>` mention 必须指向同一文档内的节点。已存储文档校验失败时按损坏数据处理并返回空默认文档；非法写入在替换文件前失败。
- 导入 Workshop archive 属于克隆：必须重新生成 `wsn`/`wse` ID，并通过一张完整 old → new remap 同步改写 group、边端点与 `node:<wsn>` mention 引用。
- 主 schema 的运行时 contract 校验全部已登记实体键/引用列为 `TEXT`，禁止 `AUTOINCREMENT`，并只允许 `system_settings.id` 作为显式 singleton 的单列 `INTEGER PRIMARY KEY`。

## Reset、备份与导入

删除或重建主数据库不代表旧 ID 可以复用。UUIDv7 实体 ID 永不主动回收。

Session、workspace、browser profile、attachment、knowledge inbox、companion DB 和事件文件等衍生存储必须：

1. 使用同一个规范实体 ID，并校验 owner manifest；或
2. 在 destructive reset 时与权威存储一起删除。

导入模式：

- **恢复/合并**：保留 ID；完全相同的数据幂等跳过；同 ID 不同内容报错且原子不变。
- **克隆**：生成新 typed ID，并使用显式 remap 表重写全部声明的引用，包括嵌套对象和数组中的实体引用。

任何模式都不得依赖 SQLite 自增值，也不得把 ID 冲突静默解释为旧实体失效。

## 新增实体前缀

1. 确认它是持久化、可引用实体，而非外部 ID、自然键、操作键或序号；
2. 选择未使用的规范前缀；
3. 更新中英文注册表；
4. 在 `nomifun-common` 增加 typed newtype 和校验测试；
5. 在存储及协议边界全链路使用该类型；
6. 增加导入导出与契约测试，证明 ID 始终为字符串且引用可完整往返。
