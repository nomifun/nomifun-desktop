# Agent 工作台

> 本页按 05 §15 的当前合同更新。用户可见的产品名称是 **Agent 工作台**，
> 公共 UI 路由是 `/agent`；`AgentPreset` 只是后端 authoring aggregate 的内部名称。
> AP-0～AP-7 尚未全部通过，因此本页描述的是 canonical 目标和当前实现边界，
> 不是“旧设定”兼容说明。

## 入口与最短路径

从首页侧边栏进入 **Agent**（`/agent`），然后：

1. 新建 Agent；
2. 选择轻量、通用、全面或自定义创建种子；
3. 编辑身份、指令、模型路由、能力、技能和 typed resources；
4. 查看来源、可用性与影响提示；
5. 保存并试用；
6. 在 `/agent-sessions/:agentSessionId` 中继续使用生成的 Session。

Runtime、网络、系统和提供商管理仍属于全局设置，尤其是
`/settings/execution-engines`；它们不是 Agent 工作台的编辑内容。

## 领域边界

| 对象 | 所属方 | Agent 工作台能做什么 |
| --- | --- | --- |
| Package / Plugin | 平台扩展域 | 只读取已正式物化的来源和 provenance |
| MiniApp Active Release | MiniApp 产品域 | 只读取已发布、可供消费者使用的贡献 |
| Capability Catalog | 平台能力目录域 | 查询能力、来源、合同、支持的消费者和 availability |
| Skill Catalog | 平台技能目录域 | 选择 instruction/workflow；Skill 本身不是执行器 |
| AgentPreset | Agent authoring 域 | 保存用户意图和当前 Revision 引用 |
| AgentPresetRevision | Agent authoring 域 | 保存不可变 payload、ContributionLock 和 revision digest |
| Agent Session | Agent 运行域 | 消费冻结的 Snapshot，不反向改写 Revision |

Plugin/MiniApp 是平台能力供给层，不是 Agent 子系统。安装、启停、配置、Credential、
KV、`dataDir`、发布和 Service 生命周期由所属平台域负责；Agent 只能绑定正式的
typed resource 或已物化 Capability。

## 四种创建种子

四种模式是创建时的 seed，不是四种持久化类型：

| 种子 | 初始内容 | 不会自动做什么 |
| --- | --- | --- |
| 轻量 | 身份、指令和模型路由 | 不加入工具、Workspace、MCP、Plugin 或 MiniApp |
| 通用 | 官方常用 Capability/Skill | 不吸收用户安装的全部扩展 |
| 全面 | 官方 Coding 基线 | 不安装或纳入全部 Plugin/MiniApp |
| 自定义 | 空白或可选种子 | 不允许填写内部 ID、Digest 或 Runtime 参数 |

所有创建结果都进入同一条主链：

```text
创建种子
  → AgentPreset Draft
  → canonical Compiler
  → AgentPresetRevision + ContributionLock[]
  → ResolvedSnapshot
  → AgentSession / AgentBinding
```

模板更新只影响以后创建的 Draft；已有 Revision 不会静默漂移。

## 能力、技能与来源

工作台从平台 Capability Catalog 查询能力。Catalog 条目至少包含稳定能力 ID、合同
版本/digest、owner、来源 provenance、支持的消费者、typed contribution、所需资源和
按消费者区分的 availability。

- 只有已发布、已启用并完成 materialization 的贡献可以进入正式 Catalog；
- Ready Candidate、未发布 Release、Project Source、测试 Host 和 Plugin 私有
  `dataDir` 不会进入 Agent Snapshot；
- 标记为 non-Agent-only 的能力不会出现在 Agent picker；
- 能力不可用、合同不匹配或资源缺失时显示可解释状态，并 fail closed；
- Skill 只提供 instruction/workflow 和资源说明，不自动扩张 Snapshot。

如果同一 canonical 能力存在多个合法实现，用户只能在能力详情中显式选择来源。
选择结果由服务端生成 ContributionLock；前端不提交 Mount、Artifact 或内部 digest
来驱动执行。

## Revision 与 Snapshot

用户编辑内容先存在于不可执行 Draft。Save 成功后，服务端原子创建不可变
`AgentPresetRevision`，并推进当前 Revision 指针。Preview、Save 和 Test 使用同一个
canonical Compiler；Test 创建普通 AgentSession，不创建 test-only Session。

Revision digest 覆盖规范化的：

```text
payload + contribution_locks
```

Snapshot 进一步冻结本次实际执行所需的 Capability、Provider/Model route、Tool schema、
typed resource binding、Runtime feature、initial/on-demand 分组和 Snapshot digest。
Session Open 读取已保存 Snapshot，不在每个 Turn 中重新选择 latest 来源，也不静默
切换 Provider 或 fallback。

数据库迁移记录如下：

- `061_agent_snapshot_naming.sql` 将 `conversations`、
  `agent_execution_participants`、`agent_execution_template_participants` 和
  `cron_jobs` 的物理列统一改名为 `agent_snapshot`，不保留 alias 或双写；
- `062_agent_preset_contribution_locks.sql` 为
  `nomi_agent_preset_revisions` 增加 `contribution_locks_json`，并要求其为合法 JSON
  数组；
- 旧 baseline 中的 `preset_snapshot` 只是 migration source，不是当前运行时字段；
- `preset_id`/`preset_revision` 若仍出现，只表示 AgentPreset provenance，不恢复旧
  resolver 或旧兼容模型。

## Canonical API（机器资源名）

工作台使用以下 canonical API；API 的稳定机器名称不代表 UI 必须显示“Preset”：

| 用途 | Endpoint |
| --- | --- |
| 官方创建种子 | `GET /api/agent-preset-templates?source=official` |
| 创建 AgentPreset | `POST /api/agent-presets` |
| 从官方种子创建 Draft | `POST /api/agent-presets/from-template/{template_id}` |
| 读取编辑器 | `GET /api/agent-presets/{preset_id}/editor` |
| Preview / Save / Revision | `/api/agent-presets/{preset_id}/...` |
| 平台能力 Catalog | `GET /api/capabilities` |
| Agent Session | `/api/agent-sessions/*` |
| Agent Binding | `/api/agent-bindings/*` |

客户端不得提交 Snapshot digest、Mount ID、内部 Revision ID、完整 Binding 或裸
canonical JSON。服务端负责 owner 检查、Catalog resolve、ContributionLock、Revision/
Snapshot digest 和 typed failure。

## 旧路径处理

旧 `/presets`、`/settings/agent-presets`、`/settings/agent` 和旧 `/api/presets` 不再
是产品 API。迁移窗口内的 UI 深层链接只能一次性跳转到 `/agent`；它们不能继续加载
旧编辑器、旧服务或双读写链。当前工作树仍有部分迁移 redirect、旧 consumer 和
generated inventory residual，详见：

- [`GLOBAL-CLOSURE-TODO.zh.md`](../specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md)
- [`DECISIONS.zh.md`](../specs/2026-08-28-agent-capability-platform-v2/DECISIONS.zh.md)
- [`05-system-capability-replacement-foundation.zh.md`](../specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md)

在 AP-7 admission 通过前，不得开始 06 的 Plugin/MiniApp 代码实施。
