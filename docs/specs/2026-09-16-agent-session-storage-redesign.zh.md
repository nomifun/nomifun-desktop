# Agent Session 与执行日志存储重构方案

> 日期：2026-09-16
> 状态：目标设计已确认；DEC-13 采用方案 A（仅 Agent 数据 clean cut）
> 原则：不迁移历史 Agent 会话日志，不建立永久兼容读取器
> 实施入口：[UARC 长程实施总计划](2026-09-16-unified-agent-overhaul/README.zh.md)

## 1. 当前问题

Agent/Conversation 运行事实目前分散在多套存储中：

| 当前存储 | 内容 |
| --- | --- |
| `conversations` | 会话身份、模型、Preset 快照、状态、Cron、Channel 和大量 `extra` JSON |
| `messages` | UI 消息投影，同时承载多种历史类型和状态 |
| `conversation_delivery_receipts` | Turn/steer 等幂等请求、准入和终态回执 |
| `conversation_runtime_events` | Runtime 私有语义事件和模型操作 claim |
| `conversation_mcp_effects` | MCP 未知效果与结算 |
| `conversation_hosted_effects` | Plugin/Robot/Git 等效果回执 |
| `agent_execution_*` | Execution、Participant、Step、Attempt、Event、Link |
| `{data_dir}/nomi-sessions/` | 旧 Nomi 私有 Session transcript、checkpoint、deferred tools 和 index.json |
| Fresh-v4 `agent_sessions/session_events/...` | 另一套隔离 AgentSession event store，当前 Nomi-core 产品主链不使用 |
| 其他关系表 | Creative Studio、Channel、Cron、Remote、Plugin surface 等各自的 Session 绑定 |

结果是：

- Conversation、AgentSession、Runtime Session 的身份边界不清；
- 同一 Turn 同时出现在 messages、delivery receipt、Runtime event 和私有 transcript；
- 不同工具域分别新建 effect 表；
- UI projection 与 canonical fact 混在一起；
- `extra` JSON 承担运行 authority；
- 删除、reset、fork、恢复需要跨多套存储协调；
- Fresh-v4 和当前产品链同时存在但互不兼容；
- 每次演进只能继续增加迁移、trigger 和兼容分支。

这不是通过再增加一张映射表或一个兼容 adapter 可以解决的问题。

## 2. 路径选择的批评性结论

### 2.1 可以完全不迁移历史 Agent 日志

可以直接建立新的数据代际，旧 Session/Message/Runtime 日志不导入。这样能删除历史 ID、状态和
恢复语义，而不是把不一致数据翻译进新模型。

代价是旧 Session 和旧自定义 Agent 不再出现在新产品中；官方 Agent 从新 schema 重新 seed。

### 2.2 不建议只新增一个并行 Agent SQLite 文件

Requirements/AutoWork、Cron、Channel、AgentExecution 与 Agent Turn 之间存在原子认领、准入、回执和
效果结算。若主业务 DB 与 Agent DB 分离，将需要跨库 outbox/saga、重放和补偿，增加的复杂度大于
删除历史的收益。

因此只有两种合理选择：

1. **完整新数据代际（最干净）**：新 canonical data root + 新单一 SQLite；不读取旧 root。
2. **仅 Agent 数据清空（影响较小）**：继续使用同一 SQLite，删除旧 Agent 运行表并创建新表；保留
   Provider、Plugin、MCP、Knowledge 等非 Agent 配置。这不是“新 DB 路径”，但仍然没有会话迁移。

不采用“旧主库 + 新 Agent DB 长期并存”。

### 2.3 已确认选择

采用同库 Agent 表 clean cut：只丢弃 Agent Session、Execution、Preset 和日志，保留 Provider、Model、
Plugin、MCP、Knowledge 及其他非 Agent 配置。新版本使用新的 Agent 表集合和 object/checkpoint 路径，
不迁移旧 Agent 数据。

同时生成新的压缩 baseline，不能继续从 001～当前迁移重放旧 Agent 表再删除。完整 data root 重置
方案不采用，长期并行 Agent SQLite 方案也不采用。

## 3. 目标数据模型

一份 canonical SQLite 是所有 Agent 执行事实的唯一数据库；大 payload/checkpoint 使用同一代际下的
content-addressed object 目录。

```text
{canonical_data_root}/
  nomifun.sqlite
  objects/
    <sha256>
  runtime-checkpoints/
    <session-id>/<digest>   # 可丢弃派生数据
```

### 3.1 核心表

| 表 | 唯一职责 |
| --- | --- |
| `agent_sessions` | Session 身份、owner、标题、Agent Revision/Snapshot、状态和当前 head |
| `agent_turns` | accepted input、idempotency、admission、状态、结果和终态错误 |
| `agent_events` | Session 内按 sequence 追加的规范语义事件 |
| `agent_messages` | 从 events 生成的 UI/read projection，可重建 |
| `agent_payloads` | 附件、媒体和大型事件 payload 的 digest/metadata/object ref |
| `agent_effects` | 所有 Domain effect 的 pending/terminal receipt 和资源归属 |
| `agent_session_resources` | 本 Session 冻结的 typed Resource Binding |
| `agent_presets` / `agent_preset_revisions` | 新 Module/Action schema 的 Agent 配置和 immutable Revision |

AgentExecution 保持独立业务聚合，但 Attempt 直接引用 `agent_session_id`，删除多余的
Conversation/AgentSession 双重 link 语义。Requirements、AutoWork、Cron、Channel 等通过 typed ID 和
同库事务引用 Session/Turn，不读取 Runtime 私有状态。

### 3.2 一个身份

产品 UI 可以继续使用“对话”一词，但底层只有 `agent_session_id`：

- 不再分别创建 Conversation 和 AgentSession；
- Message 是 Session event 的 projection；
- Fork 创建新 AgentSession 并记录 parent/fork cursor；
- AgentExecution Attempt 引用 AgentSession；
- Channel、Cron、AutoWork 和 Creative Studio 绑定同一个 ID。

### 3.3 Turn 是准入和幂等边界

`agent_turns` 合并当前 delivery receipt 和 active turn authority：

```text
turn_id / session_id / operation_id / idempotency_key
source_message_id / admission_epoch
state: accepted | running | completed | failed | cancelled | interrupted
result/error/created/started/finished
```

Steer 等输入可以使用同一 operation/receipt 合同，但不会伪造成新的主 Turn。

### 3.4 Event 与 Message

- `agent_events` 是唯一 append-only Runtime/Domain 语义日志；
- Runtime private event 必须版本化、有限大小且不含凭据；
- `agent_messages` 只做查询优化和 UI 展示，丢失后可从 events 重建；
- Streaming delta 不必全部永久写入，只保存可恢复语义边界和最终投影；
- Compaction 不删除原始 accepted input/Tool receipt，只改变派生模型上下文。

### 3.5 统一 Effect Ledger

用一张 `agent_effects` 替代 MCP、Plugin、Robot、Git 等不断增加的专用表：

```text
effect_id / session_id / turn_id / operation_id
owner_domain / capability_module / action_id
resource_binding_id / resource_key
input_digest
state: pending | returned | rejected | cancelled | unknown
bounded_observation
created_at / settled_at
```

Domain owner 决定其结果语义；Runtime 只读取通用状态、freshness、cleanup/effect evidence。涉及跨
Session 全局互斥的效果（例如 Git push）通过 owner 的 resource_key 唯一约束表达。

### 3.6 Runtime Checkpoint

- Checkpoint 是可丢弃派生数据，不是会话事实；
- 绑定 exact build、Snapshot、Session、event cursor 和 digest；
- 不匹配则删除并从 DB 事件重建；
- 不再保留 `{data_dir}/nomi-sessions/index.json` 和第二份 transcript；
- checkpoint 文件不参与用户备份语义，canonical DB 足以恢复安全状态。

## 4. 明确删除

- `{data_dir}/nomi-sessions/`、Nomi Session index/transcript；
- `NomiSessionPersistence`；
- 旧 `conversations/messages` 作为 Agent 主存储；
- `conversation_delivery_receipts`；
- `conversation_runtime_events`；
- `conversation_mcp_effects`；
- `conversation_hosted_effects`；
- 当前产品未使用的隔离 Fresh-v4 Session root/双主链；
- `extra` 中的 Runtime/AutoWork/IDMM authority；
- Conversation 与 AgentSession 双创建/双 ID；
- 为每个新 Domain effect 单独建表的模式。

如果部分旧表仍服务非 Agent 历史页面，先改名为明确的 projection/archive；不能继续成为新 Session
写入目标。

## 5. 与平台领域的关系

### Requirements / AutoWork

- Requirement claim 与创建 Agent Turn 可在同一 DB 事务或同一 owner command 中闭合；
- AutoWork 只读取 canonical turn/effect receipt；
- 不从 messages 文本猜测成功；
- AutoWork 不保存第二份 Runtime overlay。

### AgentExecution

- Execution/Step/Attempt 保持业务聚合；
- Attempt 创建 AgentSession；
- Session terminal receipt 推进 Attempt；
- 不再通过 Conversation link 猜测当前 Attempt；
- Runtime plan 不写入 AgentExecution DAG，除非显式 domain command。

### IDMM

- Agent Runtime 自己写 Turn deadline/failure/decision events；
- Agent 路径不再有旁路 IDMM 日志和 action reservation；
- Terminal Supervisor 数据归 Terminal domain。

### Browser 与其他 Resource

- Browser、Workspace、MCP、Robot 等都写入 `agent_session_resources`；
- 资源自身的大状态仍归 Domain owner；
- Session 只冻结 binding、权限和必要 generation/digest。

## 6. 不迁移时的切换行为

### 完整新数据代际

```text
旧 root：保留在原路径或一次性改名 archive，不再打开
新 root：初始化新 baseline，seed 官方 Agent，Session 列表为空
```

- 不读取旧 Session/Message/Preset/Execution；
- 不运行逐表迁移；
- 不提供旧会话查看器；
- 用户重新配置模型、Plugin、MCP、Knowledge 等所有本地数据。

### 仅 Agent 数据清空

- Provider、Model、Plugin、MCP、Knowledge 等配置保留；
- 删除旧 Agent Session、Message、Execution、Preset 和私有日志；
- seed 新官方 Agent；
- 自定义 Agent 和历史会话不迁移；
- 新表在同一 canonical DB 中创建，保持同库原子性。

两种方式都不建立 legacy reader。旧文件如需人工备份，由用户在升级前复制整个旧 data root，
不由新产品读取。

## 7. 实施任务

### STORE-01：冻结新事实模型

- Session/Turn/Event/Message/Payload/Effect/Resource schema；
- 状态机、ID、sequence、幂等和删除合同；
- AgentExecution/Requirements/AutoWork typed boundary。

### STORE-02：新 baseline

- 生成一份从零开始的 canonical schema；
- 不引用旧 Agent migrations；
- schema self-test、索引和大小预算；
- 新 object/checkpoint 路径。

### STORE-03：单一 Session owner

- `/api/agent-sessions` 直接创建唯一 Session；
- UI Conversation 投影读取同一数据；
- Turn/steer/cancel/fork/delete；
- 删除双创建和 compatibility bridge。

### STORE-04：Runtime/Event/Effect 接线

- Unified Runtime journal port；
- Tool/effect receipt；
- Message projection；
- checkpoint rebuild；
- 删除 Nomi 私有 Session。

### STORE-05：平台领域接线

- AgentExecution Attempt；
- Requirements/AutoWork；
- Cron/Channel/Remote/Creative Studio；
- Browser/Workspace/MCP/Robot resources。

### STORE-06：数据代际切换

- 执行已确认的 Agent-only reset，保留非 Agent 配置；
- 旧路径不再打开；
- 删除 legacy code/migrations/fixtures；
- 新安装、升级和崩溃中断测试。

## 8. 验收

1. 一个用户 Turn 只存在一个 canonical `agent_turns` 记录和一条有序事件流。
2. UI Message 可从 events 重建。
3. Runtime 无独立 transcript/index。
4. 每个 effect 使用统一 ledger，并仍由 Domain owner 解释结果。
5. AgentExecution/AutoWork 只消费 typed Session receipt。
6. Reset/Fork/Delete 不跨多套 Session 存储猜测状态。
7. 新代码不读取旧 root 或旧 Agent 表。
8. 新 baseline 从空 DB 初始化，不先创建历史表再删除。
9. 压缩、取消、恢复、未知效果和重启回归通过。
10. 不为保留旧日志增加兼容 alias、双写或后台迁移。

## 9. 已确认数据切换

- 清空：Agent Session、Message projection、Turn/Event/Effect、Execution、Preset/Revision/Snapshot、
  Nomi 私有 transcript 和旧 Agent bindings。
- 保留：用户、Provider、Model、Plugin、MCP Server、Knowledge、应用设置及其他非 Agent 领域配置。
- 初始化：新 Agent baseline、新官方 Preset、新 object/checkpoint 目录，空 Session/Execution 集合。
- 禁止：旧 Agent 数据迁移、legacy reader、双写、长期 alias、并行 Agent DB。
