# CAR 阶段总纲与执行规则

## 1. 阶段目标

CAR 阶段只解决一个问题：

> 将成熟 Coding Agent 的核心执行能力内化为 NomiFun 的一个可选执行引擎，并让它
> 使用 NomiFun 的模型、Session、Capability 和领域所有权。

最终生产结构：

```text
NomiFun AgentSession Application Service
  → Agent Execution Engine Registry
  → selected Engine Build
  → Runtime Instance（进程内）
  → NomiFun ChatModelBroker
  → NomiFun Capability Kernel
  → NomiFun File / Process / VCS / MCP / Plugin / MiniApp owners
```

明确不是：

```text
NomiFun
  → 外部 codex-app-server
  → Sidecar JSONL
  → Codex 自己的模型、认证、历史和权限产品
```

## 2. 与上一阶段的关系

上一阶段已经形成或正在收尾的 Agent 工作台、AgentPreset、Revision、Snapshot、
Capability Catalog、AgentSession、SessionEvent、Plugin/MiniApp 设计，是 CAR 的
输入边界。

CAR 不重新设计以下内容：

- Agent 工作台的产品入口；
- AgentPreset 的用户模型；
- `/api/presets` clean cut；
- Capability Catalog 的多消费者模型；
- Plugin/MiniApp 的产品身份和生命周期；
- Browser/Computer Role/Provider 的平台合同；
- fresh-v4 数据根和一期发布状态。

如果 CAR 实施中发现上一阶段合同存在冲突，不得直接修改上一阶段文档。应在
`DECISIONS.zh.md`（本目录，如后续需要创建）记录冲突、影响和建议，并暂停依赖
该冲突的任务，直到形成明确的新阶段决策。

## 3. 本阶段硬边界

### 3.1 必须内化

- Agent Execution Engine descriptor、Build、Channel 和 Resolver；
- 多个 Engine Build 并存及独立 Session Binding；
- Provider-neutral turn loop；
- 单 AgentSession Runtime actor（每个 Session 绑定一个精确 Engine Build）；
- 多轮 model step 和 Tool Call continuation；
- Tool registry/router/argument delta assembly；
- read-only parallel dispatch 和 state-changing serial dispatch；
- reasoning、Tool Call、Tool result、usage 和 terminal event；
- Unified Exec 的输出、stdin、PTY、超时、取消和进程树清理语义；
- Patch parser/validation；
- Workspace 与 `AGENTS.md` context；
- Context window、Compaction、Resume、Steer 和 Cancel；
- review/diff/test Coding workflow。

其中 Build 必须 immutable；Stable/Canary 是 Catalog 指向 Build 的可变别名，不是
Build 自身属性。Channel 调整只影响之后创建或 Fork 的 Session，不修改既有 Binding。

### 3.2 明确排除

- `codex-app-server`、app-server daemon/client/protocol；
- Codex API、Codex Auth、Codex ModelProvider、ModelsManager 和 provider endpoint；
- Codex rollout/thread store、state DB 和私有历史格式；
- Guardian、approval、permission profile、grant、permit 和交互审批状态机；
- voice-host、realtime-webrtc、audio、视频和生图；
- TUI、CLI、桌面产品 UI；
- Codex Plugin Manager、Core Plugins 和 Extension Host；
- Web Search、OpenAI 专属文件传输和其他未通过 NomiFun 产品决策的专项能力；
- Code Mode、Node REPL、完整 Subagent workflow，除非后续任务有明确真实消费者。

### 3.3 子进程边界

“进程内 Runtime”只约束 Agent 核心 loop、模型 Port、Tool orchestration 和
Session actor 不得放在外部 Codex 进程中。

`process.exec` 仍然可以创建用户要求的 shell/command 子进程；这些子进程必须由
NomiFun Process owner 管理，包含取消、超时、stdin、输出截断和 descendant cleanup。

### 3.4 多引擎并存与切换边界

CAR 支持多个执行引擎和多个 Engine Build 同时存在：

```text
Legacy Nomi Engine Build
NomiFun Coding Engine Stable Build
NomiFun Coding Engine Canary Build
```

每个 `AgentSession` 在创建或显式 Fork 时解析一个 exact `EngineBinding`，之后
该 Session/Turn 不得更换 Engine。用户选择、替换和灰度只能作用于：

- 新建 AgentSession；
- 从旧 Session 显式 Fork 的新 Session；
- 尚未开始执行的 Agent Binding。

禁止同一 Session 中途切换、自动 fallback、双引擎重复执行真实 Effect，或后台
改写既有 Binding。Engine failure 必须保持 typed failure；是否 Fork 到另一个
Engine 由用户显式决定。

## 4. 决策层级

发生冲突时使用以下顺序：

```text
canonical Rust / SQL / generated schema / behavior tests
  > 本目录已确认的阶段设计
  > 本目录任务 Manifest 与 Prompt
  > 上一阶段文档的背景描述
  > Codex 上游源码的原有产品假设
```

上一阶段文档在 CAR 中是只读背景，不是新的 Coding 实施合同。Codex 源码只能说明
可借鉴的行为，不能覆盖 NomiFun 的所有权和安全边界。

## 5. 原子任务标准

每个 `CAR-*` 任务必须具备：

| 项目 | 要求 |
|---|---|
| 单一目标 | 只闭合一个可观察的 Runtime 能力或删除边界 |
| 输入 | 明确列出依赖的 Port、类型、测试 fixture 或源码快照 |
| 写集 | 列出允许修改的文件/目录 |
| 禁区 | 列出不得修改的中央文件、旧主链和其他任务写集 |
| 交付物 | 代码、测试、文档或删除结果必须可单独审查 |
| 验收 | 至少一个行为测试和一个依赖/边界检查 |
| 停止条件 | 发现需要第二套事实、Sidecar、兼容 alias 或大范围越界时立即停止 |
| 回滚 | 只允许普通 commit/revert；不 reset、不 force-push、不覆盖他人 WIP |

任务完成不能以“类型已经存在”“编译通过”或“metadata success”作为唯一依据。
必须证明真实 Runtime 行为和错误边界。

## 6. 设计简单性规则

新增设计前必须回答：

1. 它是否直接支持 Coding 用户闭环？
2. 是否能复用现有 NomiFun owner，而不复制 File、Process、Model、Session 或 Catalog？
3. 是否增加第二个持久事实源、状态机、Coordinator、Provider 配置或兼容路径？
4. 是否可以在没有外部 Codex 进程的情况下运行？
5. 是否有真实消费者和代表性测试？

如果第 1、2、4、5 项不能同时回答“是”，默认后置或删除。

## 7. 默认锁定的范围决定

以下决定无需在每个任务中重复讨论：

1. 生产中不运行 `codex-app-server`。
2. 不复制整个 `codex-core` crate graph。
3. 平台允许多个 Engine Build 并存，但每个 Session 只绑定一个 exact Build。
4. 模型只通过 `nomifun-chat-model-broker`。
5. Tool 只通过 `nomifun-agent-kernel` 和 NomiFun owner。
6. SessionEvent 是产品事实；Runtime cache 可丢弃。
7. FullAuto 由 NomiFun Snapshot/ThinAuthority 实现。
8. voice/realtime/TUI/CLI/Guardian 永久排除。
9. Plugin、MiniApp、MCP、Skill 是平台供给或消费者适配，不是 Coding Runtime 私有插件。
10. 子 Agent 默认后置，未来映射为 AgentSession fork，不复制 Codex graph store。

## 8. 阶段状态规则

本阶段状态只记录在 `STATUS.zh.md` 和 `TASK-MANIFEST.json`。旧阶段
`GLOBAL-CLOSURE-TODO.zh.md` 不记录 CAR 状态。

允许状态：

```text
planned
ready
in_progress
blocked
pending_validation
completed
cancelled
```

状态更新必须附带任务提交、验证命令和未运行项；设计文档不复制临时进度。
