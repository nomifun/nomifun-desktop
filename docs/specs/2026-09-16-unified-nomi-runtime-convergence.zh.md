# 统一 Nomi Agent Runtime 与能力组合架构实施方案

> 日期：2026-09-16
> 状态：实施提案，尚未切换生产 Runtime
> 范围：Agent Runtime、AgentPreset、Capability、Skill、Resource、长程执行和旧 Runtime 退役
> 不包含：JavaScript Runtime（Node.js/Bun）、PluginRuntime 进程、模型提供商配置重构
> 实施入口：[UARC 长程实施总计划](2026-09-16-unified-agent-overhaul/README.zh.md)

> 当前方案的全部产品裁决已经确认，统一登记在
> [能力模型重构方案 §12](2026-09-16-agent-capability-product-redesign.zh.md) 的“产品裁决清单”。
> Agent 会话日志和数据代际的完整审计见
> [Agent Session 与执行日志存储重构方案](2026-09-16-agent-session-storage-redesign.zh.md)。

## 1. 最终结论

NomiFun 最终只保留一个官方 Agent Runtime：`nomifun.nomi`。

不继续建设以下产品能力：

- 多 Runtime Family；
- 第三方 Runtime 注册；
- Agent 选择 Runtime；
- Runtime Marketplace、安装、热替换或自动 fallback；
- 为不同 Runtime 重复验证所有 Capability、Preset、Session 和恢复组合。

新的 Nomi Runtime 以当前 Coding Runtime 的执行内核为基础，吸收旧 Nomi Runtime 已有的通用
能力和产品场景支持。不同 Agent 的差异由 Capability、Skill、资源、模型和指令组合
表达，不再由第二套 Runtime 表达。

```text
一个官方 Nomi Runtime
  × Capability 组合
  × Skill / Resource / Model / Middleware
= 不同工作场景的 Agent
```

代码中保留一个窄的内部 Runtime Driver 切面。它只用于测试和未来官方整体重写：如果现有
Runtime 质量不足，可以实现一个新的官方 Driver，通过完整门禁后在组合根一次性替换旧实现。
这个切面不是多 Runtime 架构，不向用户、Plugin 或社区开放，也不设计灰度双运行流程。

## 2. 设计目标

### 2.1 必须达到

- 官方只维护一套模型循环、上下文生命周期、工具调度、取消和恢复实现。
- Coding 能力成为官方 Runtime 的一等能力，而不是一个可选的第二引擎。
- 长程任务在压缩、续接、用户中途追加要求、Patch、Process、崩溃和未知副作用下保持可靠。
- `chat.minimal` 等轻量 Agent 不承担完整 Coding 计划、历史和完成证明开销。
- 通用助理、Coding、陪伴、客服和创作继续通过不同能力集合构造。
- Runtime 不拥有文件、进程、VCS、MCP、知识、记忆、Channel、创作任务或凭据。
- 删除旧循环、旧恢复路径、双份能力投影和 Runtime 选择相关代码。
- 保留足够清晰的内部 Port，使未来官方 Runtime 可整套替换而不改 Domain owner。

### 2.2 明确不做

- 不把所有概念都转换成 Capability。
- 不提供第三方 Runtime SDK 或任意 Family 注册。
- 不建立 Stable/Canary 双 Runtime、Shadow Runtime 或多 Build 灰度系统。
- 不在同一 Session 中切换 Runtime。
- 不在 Runtime 失败后自动改用另一个实现。
- 不原地修改历史 Agent Revision、ResolvedSnapshot 或 Session 事件。
- 不复制第二套 SessionStore、Capability Kernel、模型 Broker 或 Domain owner。

## 3. 当前架构检查结论

### 3.1 两个 Runtime 分别拥有一半优势

| 项目 | 旧 Nomi Runtime | 当前 Coding Runtime |
| --- | --- | --- |
| Family | `nomifun.nomi` | `nomifun.coding` |
| 执行循环 | `nomi-agent` 通用循环 | `nomifun-coding-engine` 专用循环 |
| 长程机制 | 通用压缩、ToolSearch、私有 Session | 自动压缩、需求账本、任务续接、完成证据、工具历史、Patch 恢复 |
| 平台能力 | Skills、MCP、知识、记忆、浏览器、Computer Use、子 Agent 较完整 | 文件、VCS、Process、部分 MCP/Plugin/Robot 已接入 |
| 恢复 | Nomi 私有 Session 恢复 | 从平台 Conversation 历史重建并独立审计 |
| 能力投影 | Nomi 专有 mapping | Coding 专有手写支持清单、schema 和 ToolPlan |

运行时注册位于 `crates/backend/nomifun-app/src/router/runtime_engines.rs`。Coding 主要实现位于
`nomifun-coding-engine` 和 `coding_runtime_host.rs`；旧 Nomi 工厂和执行循环主要位于
`nomifun-ai-agent/src/factory/nomi.rs`、`nomifun-ai-agent/src/manager/nomi/` 和
`crates/agent/nomi-agent/`。

结论不是选择其中一套原样保留，而是：

- 采用 Coding Runtime 更适合长程任务的执行内核；
- 迁移旧 Nomi 的平台能力适配；
- 删除两边重复或错误归属的能力和生命周期代码。

### 3.2 当前 Coding Preset 与 Coding Runtime 并不兼容

`coding.codex` 当前列出 35 项能力，其中只有 14 项进入 Coding Runtime 的内置准入范围。
预设默认也没有绑定 Coding Runtime。详细检查见
[Codex Coding 预设与 Coding Runtime 配合检查](../reviews/2026-09-16-coding-agent-runtime-compatibility.zh.md)。

这说明当前边界确实存在问题，不能通过以下方式修复：

- 只把默认 Runtime 改成 Coding；
- 只给 Coding 手写支持清单增加 21 个 ID；
- 只把 Coding Runtime 改名为 Nomi Runtime；
- 让每个 Runtime 各自重新实现所有平台能力。

### 3.3 当前概念有重复表达

1. 执行模式同时由 Runtime Family、descriptor profile、`RuntimeProfileKind` 和
   `required_runtime_features` 表达。
2. `agent.execution.plan` 与 Runtime 内部 `update_plan` 重叠；`agent.execution.steer` 与
   `turn.steer` 名称相近但作用域不同。
3. AgentPreset 已有 `skill_bindings`，同时又平铺 `skill.catalog/describe/invoke/hooks`。
4. MCP 的连接、OAuth、全量代理、资源和逐工具授权同时进入 Agent 能力集合。
5. `workspace.bind`、`process.session`、`terminal.pty` 等基础设施被当成普通用户能力。
6. Nomi 和 Coding 分别维护能力到模型 Tool 的映射，必然发生漂移。

## 4. 最终概念边界

| 概念 | 负责什么 | 是否权限 | 是否属于 Agent 组合 |
| --- | --- | --- | --- |
| Nomi Runtime | 模型循环、上下文、调度、取消、恢复 | 否 | 否，系统唯一 |
| Capability | 可观察、可调用或可产生的效果 | 是 | 是 |
| Resource Binding | 权限作用的具体 Workspace、知识库、MCP Server 等 | 只收窄权限 | 是/场景注入 |
| Skill Binding | 冻结的指令和参考资源 | 否；其效果需 Capability | 是 |
| AgentPreset | Agent 的版本化配置配方 | 聚合权限和行为 | 是 |
| ResolvedSnapshot | 编译后的精确能力、依赖和 contribution lock | 是，执行上限 | 编译产物 |
| Conversation | Session、Turn、消息和效果回执事实 | 否 | 产品运行态 |
| AgentExecution | 跨 Agent/Step/Attempt 的持久长任务 | 不新增权限 | 产品运行态 |

判断规则：

- “Agent 能不能做这件事”属于 Capability。
- “Runtime 怎样组织这项工作”属于 Runtime 内部自适应机制。
- “对哪个具体对象做”属于 Resource Binding。
- “怎样连接、鉴权、监督进程”属于平台 Domain service。
- “把哪些能力方便地一起选中”属于 Capability Bundle，只是工作台展示。

Runtime 本身不能拆成 Capability。否则用户可以通过勾选能力改变恢复、压缩和安全语义，也会产生
大量不可验证的执行机制组合。

## 5. 目标架构

```text
AgentPreset v2
  ├─ enabled_capabilities[]
  ├─ skill_bindings[]
  ├─ resource intents
  ├─ model routes
  └─ persona / instructions / starter prompts
                 │
                 ▼
Agent Control Plane + Capability Kernel
  ├─ 展开 capability dependencies
  ├─ 冻结 action/schema/contribution/resource policy
  ├─ 校验资源、Host Port 和模型 route requirements
  └─ 生成 ResolvedSnapshot
                 │
                 ▼
Single Nomi Runtime Provider
  ├─ 唯一 Runtime factory
  ├─ 唯一 family: nomifun.nomi
  ├─ 当前 build identity
  └─ internal Driver seam
                 │
                 ▼
Unified Nomi Runtime
  ├─ 强制底座：取消、恢复、有界上下文和压缩
  ├─ 根据当前回合按需运行计划、证据和续接机制
  └─ 根据 Snapshot 装配 Capability adapters
                 │
                 ▼
EngineSessionHost → Capability Kernel → Domain owners
```

### 5.1 保留 Build 身份，不保留多 Runtime Catalog

Session 仍需记录 `family_id/build_id/build_digest/profile`，用于：

- Checkpoint 和恢复身份核对；
- 判断新版本能否重建旧 Session；
- 日志、诊断和问题定位。

但不再需要任意 Family Catalog、Channel selector 或 Agent Runtime 选择。目标可以是一个简单的
`NomiRuntimeProvider`：组合根安装一个 descriptor 和一个 factory，应用启动后不可替换。

`family_id` 固定为 `nomifun.nomi`。Build 变化来自应用升级，而不是用户配置。

### 5.2 内部 Driver seam

统一 Runtime 通过窄的内部 Driver 接口消费 `EngineSessionHost`：

```text
NomiRuntimeDriver
  open_session(admitted session, exact binding, host ports)
  start_turn(accepted input)
  steer / cancel
  teardown with cleanup result
```

要求：

- 不公开稳定 ABI；
- 不允许运行期注册；
- 不接受非官方 Family；
- 不允许 Driver 直接访问 root DB、凭据或 Domain handler；
- fake Driver 只用于合同测试；
- 未来官方重写 Runtime 时，实现同一接口并在组合根替换旧 factory。

## 6. 单一自适应执行循环

Runtime 不预先把 Agent 分成轻量、标准或重型三档。每个回合都从同一条最小执行路径开始，
根据真实发生的工作按需激活内部模块：

```text
收到输入
→ 构造最小上下文
→ 如果模型直接回答，则结束
→ 如果产生 Tool Call，进入通用 Tool loop
→ 如果出现多步或效果操作，建立需求/证据账本
→ 如果上下文接近上限，执行 compaction
→ 如果发生 Patch/Process，启用对应恢复与证明
→ 如果用户明确继续旧任务，装载 continuation candidate
```

这样 `chat.minimal` 因为没有工具和复杂上下文而自然轻量；Coding Agent 即使能力很多，面对简单
解释问题也不会预先建立完整计划和历史账本。执行重量由本回合实际行为决定，不由静态 Agent
类型或用户可选档位决定。

### 6.1 强制 Runtime invariants

以下是所有 Agent 的基本通用要求，不是 Capability、Profile、Feature flag 或可选模块：

| Invariant | 始终保证 | 何时实际运行 |
| --- | --- | --- |
| Cancellation | 每个 active turn 和在途任务都有可达的取消与清理边界 | 用户/平台取消、超时或关闭时 |
| Recovery | Runtime 状态可由 canonical facts 重建，未知效果不自动重放 | 启动、崩溃或中断恢复时 |
| Context bounds/compaction | 所有模型请求都受统一预算约束，超限前有安全压缩路径 | 历史、Tool 结果或媒体达到阈值时 |

“始终保证”不表示每个简单回合都执行压缩或恢复算法，而是这些路径始终存在且不能被 Agent 配置
关闭。它们也不进入 Capability Catalog，因为它们不授予外部权限。

现有 `context.compact.*`、`turn.cancel/interrupt` 等 feature ID 随旧 Snapshot 一同退出新数据代际；
新 Preset/Snapshot 不再把这些基础要求逐项列成可协商 feature。统一 Runtime 的合同版本和强制测试
直接保证这些行为，不增加旧 Snapshot 兼容读取。

其余共同底座还包括：

- exact Session/Snapshot/Build binding；
- one-active-turn；
- bounded context、输出、Tool 参数和 Tool 结果；
- Capability Kernel admission；
- typed cancel、cleanup 和终态；
- effect receipt 与未知结果；
- canonical Conversation history；
- 不自动重放不确定副作用；
- Runtime 不接触 credential value。

### 6.2 按事件和 Capability 激活内部模块

| 模块 | 激活条件 |
| --- | --- |
| Workspace/AGENTS scope | 有 Workspace 或文件/VCS 能力 |
| Patch recovery | 有 `fs.patch` |
| Process provenance | 有 `process.exec` |
| VCS evidence | 有 `vcs.*` |
| Image context | 有图像读取权限且模型支持 image input |
| Skill context/resources | Snapshot 有 `skill_locks` |
| MCP ToolPlan | Snapshot 有 `mcp_tool_locks` |
| Middleware/hooks | Snapshot 有准入的 middleware contribution |
| Requirement/plan ledger | 当前回合形成多步工作、外部效果或模型显式规划 |
| Task continuation | 用户明确要求继续，且平台提供可续接的 closed task |
| Completion evidence | 当前回合执行过效果操作或形成了多步计划 |

## 7. Capability 边界清理

全部 136 个 first-party Capability 的产品模块归并、目标 Agent 组合和逐 Package 映射见
[NomiFun Agent 能力模型全面检查与重构方案](2026-09-16-agent-capability-product-redesign.zh.md)。
本节只保留 Runtime 收敛所需的边界结论。

### 7.1 Authoring 分类

Catalog 增加 authoring 选择策略：

| 类型 | 含义 |
| --- | --- |
| `direct` | 用户或模板可直接选择，形成权限根 |
| `dependency_only` | 只能由 direct capability 依赖引入 |
| `platform_managed` | 连接、OAuth、宿主基础设施，不能成为 Agent 权限根 |
| `internal` | Runtime 执行机制，不进入 Agent Capability Catalog |

Compiler 已能区分 Contribution 与 Dependency，应利用依赖图生成 Snapshot，而不是让官方模板
平铺所有实现依赖。

权限型依赖不能静默授权。若某个 Skill 需要 `fs.write`，工作台必须明确展示并让用户确认；
纯基础设施依赖才可以自动展开。

### 7.2 模糊能力的目标归属

| 当前条目 | 目标归属 | 处理 |
| --- | --- | --- |
| 文件/VCS/Web/知识读写/创作/Channel 发送/Robot 动作等真实操作 | direct Capability | 保留并接入统一 adapter |
| `agent.delegate`、`agent.fork` | direct Capability | 调用 AgentExecution/Session owner |
| `agent.execution.observe/steer` | session/role-derived grant | 只在当前 AgentExecution 身份允许时注入，不作为 Preset 全局勾选项 |
| `agent.execution.plan` | Runtime internal | 从能力选择移除；Runtime 在真实多步任务中按需建立 plan |
| `workspace.bind` | dependency/resource | 由 File/VCS/Process 要求，工作台不直接选择 |
| `workspace.artifacts` | 拆分 | 资源绑定内部化；如存在读取/发布效果，改为明确的 artifact read/publish Capability |
| `process.exec` | direct Capability | 统一命令和交互动作 |
| `process.session`、`terminal.pty` | dependency/resource | 从 process action、平台支持和绑定推导 |
| `session.attachments.read` | session-derived grant | 用户实际附加资源后按消息/资源身份授予，不在 Preset 长期勾选 |
| `skill.catalog/describe` | binding-derived context | 存在 Skill binding 时生成，不单独选择 |
| `skill.invoke` | frozen Skill invocation | 只能调用已绑定 Skill，效果权限仍显式授权 |
| `skill.hooks` | middleware contribution | 由 Skill/Product lock 导出 |
| `mcp.connect`、`mcp.oauth` | platform-managed | 属于连接和账户授权，不进入 Preset |
| `mcp.tool_proxy` | 删除 | 使用冻结的逐工具 Capability/mapping |
| `mcp.resource` | binding-derived resource grant | 由明确 Server/resource binding 导出，不作为通用开关 |
| `citation.render` | result-derived helper | 有合法搜索结果时自动可用，只接受本 Session 的结果身份 |
| `llm.vision` | derived authorization + model trait | 从具体图像资源权限和模型 image input 推导，不作为孤立勾选项 |
| `fs.watch` | direct EventSource | Runtime 只消费有界事件，不拥有 watcher |

### 7.3 Skill、MCP 和 Bundle

- Skill binding 冻结正文、资源和依赖；Skill 本身不能绕过 Capability 授权。
- MCP Tool 必须物化为逐工具 Capability 和 schema lock；连接/OAuth 留在 MCP owner。
- Capability Bundle 只用于 UI 展开和批量选择，不进入 Kernel，不拥有 handler。
- 官方模板与工作台使用同一 Bundle/依赖解析器，不能维护两份隐藏清单。

### 7.4 其余过度配置化候选

当前根因不是 `CapabilityKind` 太丰富，而是 Catalog 中 Tool、Context、Resource、Transport、
Middleware、Scheduler 和 BackgroundService 都可能被当作 AgentPreset 的同级直接选择项。

#### 确定不应直接配置

| 类别 | 当前 ID 示例 | 目标 |
| --- | --- | --- |
| Runtime 通用机制 | compaction、cancel、recovery、Tool history control | Runtime invariant/internal |
| 资源容器 | `workspace.bind`、`process.session`、`terminal.pty`、`knowledge.mount`、`memory.session.scratch`、`robot.link`、`ssh.connect` | Resource Binding / dependency |
| 连接与入口 | `mcp.connect`、`mcp.oauth`、`channel.pairing`、`remote.*`、`ingress.*`、`llm.realtime` | Platform-managed transport |
| 调度与后台服务 | `autowork.runner`、`schedule.timer`、`schedule.agent_trigger`、`knowledge.source.sync`、`robot.audio` | Domain service lifecycle |
| 事件投递 | `notification.desktop`、`notification.webhook` | 通知/自动化配置，不是模型权限 |
| 入站路由 | `channel.receive` | Channel binding 决定消息进入哪个 Agent，不是 Agent 自己调用的能力 |
| 场景中间件 | `channel.group_policy`、`customer_service.dialogue`、`idmm.*` | 由场景/Role/绑定导出 |
| 场景固定上下文 | `companion.persona`、`companion.roster` | 由 Companion binding 注入，并以资源权限收窄 |
| 记忆维护机制 | `memory.project.distill`、底层 merge/evolve 操作 | 由明确的 Memory 写权限和领域策略驱动 |
| Skill 基础设施 | `skill.catalog`、`skill.describe`、`skill.invoke`、`skill.hooks` | 从 `skill_bindings` 和冻结 locks 导出 |
| MCP 基础设施 | `mcp.tool_proxy`、`mcp.resource` | 删除全量代理；资源和逐工具权限由绑定导出 |
| Runtime 计划工具 | `agent.execution.plan` | Runtime internal |

#### 应合并或改为派生关系

| 重叠 | 问题 | 目标 |
| --- | --- | --- |
| `creation.*` 与 `llm.image/audio/video/embedding/rerank.*` | 产品能力和底层模型调用同时可选 | Agent 选择 `creation.*`；底层模型由 Role/Model route 提供 |
| `knowledge.search/read` 与 `knowledge.embedding/rerank` | 用户意图和检索实现细节同时可选 | 保留 search/read/write；embedding/rerank 作为 provider 实现 |
| `companion.learn/evolve` 与 `memory.companion.write/merge/evolve` | 场景行为和存储操作重复暴露 | Agent 选择高层 Companion 行为或明确 Memory 权限，内部操作作为依赖 |
| `memory.project.read` 与 `memory.project.citation` | 同一读取结果被拆成两个长期权限 | citation 从合法 memory result 派生 |
| `session.attachments.read` 与实际附件绑定 | Preset 权限脱离当前消息事实 | 用户附加资源时生成 session-scoped grant |
| `llm.vision` 与图像资源/模型 route | 全局开关重复表达资源权限和模型兼容性 | 同时从具体资源授权与 `image_input` route 推导 |
| `citation.render` 与搜索结果 | 格式化帮助被当成长期权限 | 有合法结果时按 session 派生 |
| `agent.execution.observe/steer` 与 Execution 身份 | Preset 能力范围过大，缺少当前 Execution scope | 根据 lead/attempt/owner Role 生成 session-scoped grant |
| `workspace.artifacts` | ResourceProvider 同时被当作 workspace write 权限 | 分离资源绑定与 artifact read/publish 操作 |

#### 仍应保留为 Capability

以下能力代表真实数据访问或外部效果，不能因为“统一 Runtime”而自动开放：

- `fs.read/write/patch/delete`、`vcs.*`、`process.exec`；
- `web.search/fetch`、`knowledge.search/read/write`、项目/陪伴记忆的真实读写；
- `channel.send/reply`、客服 notes/handoff、通知发送类 Tool；
- Browser/Computer 的观察与输入、Robot vision/motion/display/device tools；
- Creation、Office、Workshop、Plugin、SSH 的真实操作；
- `agent.delegate`、`agent.fork`；
- 每个物化的 MCP/Connector 具体 Tool。

ContextContributor 也不天然属于内部机制。屏幕、摄像头、知识库和记忆等敏感上下文仍需要明确
Capability；只有附件、引用格式化、场景 Persona 等由当前 Session/Binding 已经明确授权的内容，
才适合改为派生 grant。

## 8. AgentPreset 目标模型

```text
AgentPresetDocumentV2
  enabled_capabilities[]
  skill_bindings[]
  resource_intents[]
  model_route_refs / chat_route_records
  context_order / middleware_order
  persona / instructions / starter_prompts
```

删除 `runtime_engine`。AgentPreset 不选择 Family、Build、Channel 或 Profile。

编译流程：

```text
direct capability selections
→ authoring/permission validation
→ Capability/Skill/Role dependency closure
→ Resource requirements
→ Resource、Host Port 与模型 route requirements
→ model route traits
→ exact contribution/action/schema locks
→ ResolvedSnapshot
```

工作台保存前调用服务端权威 preview compile，不能只根据“能力已物化”显示可保存。

### 8.1 官方模板

| 模板 | 主要能力组合 |
| --- | --- |
| `chat.minimal` | 无工具或极少显式能力 |
| `assistant.general` | 附件、知识、Web、项目记忆、创作 |
| `coding.codex` | Workspace、File、VCS、Process、Web、MCP Tool、Skill、协作 |
| `companion.default` | Persona、记忆、知识、Channel、Robot |
| `customer-service.default` | 对话、知识、笔记、转接 |
| `creative-studio.default` | 素材、生成和编辑 |

## 9. Runtime 与长程任务事实边界

### 9.1 Runtime 可以持有

- active turn 的模型循环；
- 有界派生上下文；
- turn-local plan、需求和完成账目；
- 可丢弃 checkpoint；
- 当前 Tool 批次和清理句柄；
- 从 canonical history 派生的 continuation candidate。

### 9.2 Runtime 不能持有

- Conversation、消息和 accepted turn；
- AgentPreset Revision / ResolvedSnapshot；
- AgentExecution DAG、Participant、Attempt 和人工决策；
- 文件、进程、VCS、MCP、知识、记忆和创作任务事实；
- Provider credential；
- Cron、Channel 连接或长期调度。

### 9.3 两类计划

- Runtime plan：单个 Agent 当前任务的派生账本。
- AgentExecution plan：跨 Agent/Step/Attempt 的持久 DAG。

`agent.execution.observe/steer` 操作第二类；Runtime 内部 `update_plan/report_completion` 属于第一类。

### 9.4 长程可靠性规则

- 原始 Conversation/AgentExecution 事实不因 compaction 删除。
- Compaction 只替换派生模型上下文并记录来源范围。
- 跨回合 continuation 必须由当前用户明确要求。
- 历史 Tool、权限、进程和完成证据不能自动导入新回合。
- 崩溃恢复先结算或隔离未知效果，不自动重放。
- Checkpoint 身份不匹配时丢弃并从 canonical facts 重建。
- 命令进程不承诺跨应用重启继续运行。
- 完成声明必须引用当前 workspace epoch 的有效观察。
- 退出码 0 不等于测试通过。

## 10. 代码布局与删除范围

### 10.1 保留

| 模块 | 职责 |
| --- | --- |
| `nomifun-agent-contracts` | Preset、Capability、Snapshot 和 Runtime invariant 版本合同 |
| `nomifun-agent-kernel` | 能力依赖、权限、资源和 contribution 编译 |
| `nomifun-agent-control-plane` | Preset/Revision/模板/预编译 |
| `nomifun-engine-core` | Runtime 内部 Driver/host ports |
| `nomifun-chat-model-broker` | 唯一模型 route/provider/credential/retry owner |
| `nomifun-conversation` | Session/Turn/Message/Receipt 事实 |
| `nomifun-agent-execution` | 持久长任务、多 Agent、DAG |

### 10.2 演进

为避免与 JavaScript/Bun 组件 `nomifun-runtime` 混淆，统一 Runtime crate 最终建议使用
`nomifun-agent-runtime`：

| 当前模块 | 目标 |
| --- | --- |
| `nomifun-coding-engine` | 演进为统一 `nomifun-agent-runtime` |
| `coding_runtime_host.rs` | `nomi_runtime_host.rs` |
| `coding_tool_surface.rs` | `agent_tool_surface.rs` |
| `coding_runtime_history.rs` | `agent_runtime_history.rs` |
| `coding_runtime_recovery.rs` | `agent_runtime_recovery.rs` |
| `nomifun-ai-agent/src/coding_runtime.rs` | 统一 Runtime adapter，或下沉到新 crate |

物理重命名在行为切换稳定后单独完成，不和 Capability 迁移混在同一提交。

### 10.3 删除候选

替代链路通过后删除：

- `nomifun-ai-agent/src/factory/nomi.rs` 旧 Runtime 工厂；
- `nomifun-ai-agent/src/manager/nomi/**` 旧执行循环；
- `nomi_session_persistence.rs` 和 Nomi-only rewind/reset；
- `AgentRuntimeHandle::Nomi` 和 Nomi-specific 下转型；
- `uses_nomi_session`、`uses_nomi_recovery`；
- 旧 `NomiAdmission` 和 Nomi-specific projection；
- `nomifun.coding` Family；
- 任意 Runtime Family Catalog、selector、channel 和注册 API；
- AgentPreset/UI 中的 Runtime 选择；
- Nomi/Coding 双份能力 mapping/支持清单。

`crates/agent/nomi-agent` 不能整 crate 直接删除。先将仍有价值的 Capability adapter、Tool schema、
Subagent、Skill、Knowledge、Browser 和 Computer Use 代码迁到 owning domain 或统一 Runtime Port。
`nomi-tools`、`nomi-mcp`、`nomi-skills`、`nomi-memory`、`nomi-compact`、`nomi-providers` 也要按真实
生产依赖逐项审计。

## 11. 实施任务拆分

```text
NUR-01 合同与概念清理
   ↓
NUR-02 统一 Runtime 公共内核
   ├─→ NUR-03 自适应模块激活
   └─→ NUR-04 长程 Coding 稳定性
              ↓
NUR-05 通用 Capability projection
   ├─→ NUR-06 平台能力迁移
   └─→ NUR-07 Preset / Workbench 收敛
              ↓
NUR-08 单 Runtime 切换与旧代码删除
              ↓
NUR-09 全量验证与发布
```

### NUR-01：合同与概念清理

主要写集：Agent contracts、API types、Kernel compiler、Control Plane。

交付：

- Capability direct/dependency/platform/internal 分类；
- 新 Preset schema 不再包含 `runtime_engine`，不读取旧 Agent 数据；
- Resource、Host Port 和模型 route requirements 自动推导；
- 服务端 preview compile；
- 单一 `NomiRuntimeProvider` 合同。

验收：Preset 不再选择 Runtime 或执行档位；非官方 Family 无生产入口。

### NUR-02：统一 Runtime 公共内核

主要写集：`nomifun-coding-engine`、`nomifun-engine-core`、Runtime adapter。

交付：

- 单 actor/turn/model/tool loop；
- journal、cancel、cleanup、bounded context；
- 从 `EngineSessionHost` 获取全部平台事实；
- 内部模块按事件装配；
- 统一错误和终态；
- 去除 `nomifun.coding` 假设。

验收：fake model 下 plain text 和 Tool continuation 通过；Runtime 不访问具体 Domain handler。

### NUR-03：自适应模块激活

交付：

- 普通文本回合只构造最小上下文和一次模型步骤；
- Tool、计划、证据、续接、Patch 和 Process 状态按真实事件懒加载，compaction 只在预算达到阈值时执行；
- 复杂机制触发条件确定、可测试且不能授予额外 Capability；
- 每回合记录有界的实际模块和预算信息。

验收：`chat.minimal` 不承担重型开销；同一个 Coding Agent 的简单问答同样保持最小路径。

### NUR-04：长程 Coding 稳定性

交付：

- 需求来源、计划和完成证据；
- 多次 compaction 后的强制状态保留；
- Tool history 和显式 task continuation；
- Patch recovery、Process provenance、workspace epoch；
- context overflow recovery；
- 保守 restart recovery 和未知效果隔离。

验收：不丢需求、不重复效果、不将未验证工作标记完成。

### NUR-05：通用 Capability projection

交付：

- 从 ResolvedSnapshot 生成 Tool/Context/Resource/Middleware plan；
- 标准 schema、effect、resource 和 action mapping；
- 删除 Nomi/Coding 双份 mapping；
- 不支持的 contribution 在保存时报告；
- Capability 新增不需要修改 Runtime Family 分支。

验收：所有调用仍经过 Kernel 和 Domain owner，不能仅把 ID 加进 Runtime 支持清单来假装能力可用。

### NUR-06：平台能力迁移

按 owner 分组施工：

- File / VCS / Process / Workspace；
- Attachment / Web / Citation；
- Skill / MCP / Plugin；
- Knowledge / Project Memory / Companion Memory；
- Channel / Companion / Customer Service；
- Creation / Workshop / Artifact；
- Browser / Computer Use / Robot；
- AgentExecution / Fork / Subagent。

每组必须交付真实 adapter、权限/资源校验、取消/未知结果处理和代表性产品链路。

Browser 子任务还必须删除“会话浏览器”专属产品入口：Browser 作为普通 Capability Module 供任意
获授权 AgentSession 使用，底层资源按 Session/Binding 隔离。详细任务和源码删除范围见
[能力模型重构方案第 7 节](2026-09-16-agent-capability-product-redesign.zh.md)“Browser 产品模型纠正”。

### NUR-07：Preset 与 Workbench 收敛

交付：

- 六个官方模板只列 direct capabilities；
- 能力选择器隐藏 dependency/platform/internal；
- Bundle 和权限依赖确认；
- 删除 Runtime selector；
- 删除 Guid“会话浏览器”按钮、浏览器专用空 Session 创建和自动打开状态；
- 将 Browser Panel 改成当前 Session 的通用 Capability Surface；
- 设置页只显示当前 Nomi Runtime Build/健康/恢复诊断，与 JavaScript Runtime 分开；
- 保存前展示权威兼容性错误。

验收：六个模板均由统一 Runtime 创建 Session，并执行各自代表性能力。

### NUR-08：单 Runtime 切换与删除

交付：

- 新 Agent Revision 去除 Runtime 字段；
- 从新 Module schema 重新 seed 官方 Preset；
- 组合根切换到统一 factory；
- 旧 Session transcript 不进入新数据代际；
- 新 Agent Store 从空 Session 集合启动，不加载第二 Runtime；
- 同一变更序列删除旧 Nomi loop、`nomifun.coding` 和多 Runtime 基础设施；
- production reachability audit。

验收：应用中只有一个可创建/运行 Session 的 Runtime 实现。

### NUR-09：全量验证与发布

覆盖 Windows、macOS、Linux 的构建、升级、六个模板、长程 Coding、取消、恢复、MCP/Plugin、
浏览器/Computer Use 可用平台、性能和秘密扫描。

## 12. 数据处理与未来替换

### 12.1 AgentPreset

- 不迁移旧 AgentPreset/Revision/Snapshot；
- 从新 Module/Action schema 重新 seed 官方 Agent；
- 用户自定义 Agent 从新最简 Agent 重新创建；
- 新代码不读取 `runtime_engine`、旧 Capability ID 或旧 Snapshot；
- 不增加 v1/v2 双读、alias 或后台转换器。

### 12.2 Session

- 新 Session 只使用当前 `nomifun.nomi` Build 和新 canonical Session Store；
- 不迁移或展示旧 Session transcript；
- 删除 Nomi 私有 transcript 和旧 Session compatibility path；
- 不携带旧 Nomi/Coding factory 作为 recovery-only 第二 Runtime；
- 按 DEC-13 只清空 Agent 数据、保留非 Agent 配置，见
  [Session 存储重构方案](2026-09-16-agent-session-storage-redesign.zh.md)。

### 12.3 未来官方 Runtime 整体替换

未来如果统一 Runtime 质量不足，替换流程只包含：

1. 新实现接入内部 `NomiRuntimeDriver` 和现有 Host Ports；
2. 跑 NUR-09 的完整合同、Preset、能力和长程任务门禁；
3. 在组合根将唯一 factory 指向新实现；
4. 更新 official build identity 和 checkpoint codec；
5. 对新数据代际内的旧 Build 明确可重建或需要新建；
6. 删除旧实现。

不建设灰度 Runtime、双执行、Shadow Tool effect、用户切换或自动 fallback。需要降低发布风险时，
使用普通版本渠道、测试数据和预发布制品，而不是把多 Runtime 复杂度带进产品架构。

## 13. 验证矩阵

### 13.1 合同

- 新 Preset/Snapshot schema 从空数据代际初始化，不读取旧版本；
- Runtime 自适应模块不能授予或扩大权限；
- Capability dependency 和冲突；
- exact build/checkpoint mismatch fail closed；
- 非官方 Runtime 注册不可达；
- 六个模板 preview compile。

### 13.2 Runtime

- text → Tool Call → Tool Result → continuation；
- read-only parallel / effectful serial；
- steering 与 Tool admission 原子边界；
- cancel、interrupt、dispose、panic cleanup；
- semantic output/effect 前后的 provider failure；
- bounded arguments/results/history/media；
- late event、重复 ID 和错误 stream framing。

### 13.3 长程 Coding

- 多轮、多文件重构并经历自动 compaction；
- 多次 compaction 后需求、约束和未解决项完整；
- Patch mismatch 后强制重读；
- 交互命令 start/input/poll/exit/cleanup；
- 用户 steering 后旧完成报告失效；
- 输出上限触发有界 continuation；
- 重启后不重放未知 Tool effect；
- 明确“继续”后导入旧需求，但不导入旧权限、进程和完成证据；
- 完成报告覆盖所有需求，未验证项明确标记。

### 13.4 六个官方场景

每个保留的 direct Capability 至少验证 Snapshot 内成功、Snapshot 外拒绝、resource/schema mismatch、
取消或未知结果。每个官方模板至少有一个真实代表能力闭环，不能只测试“Session 能创建”。

### 13.5 最小路径性能

- 直接回答的回合不注册或构造不需要的 plan/history/completion 状态；
- 简单回合不因 Agent 能力数量较多而自动进入重型路径；
- 无能力 Agent 不物化 ToolPlan；
- 上下文、历史和并行 Tool 全部有硬上限；
- 冷启动、首个模型请求、常驻内存和二进制体积对比旧基线，无未解释的大幅退化。

### 13.6 最终残留检查

生产代码必须不再存在可达的：

```text
nomifun.coding Session 创建
旧 NomiAgentManager factory
AgentRuntimeHandle::Nomi
uses_nomi_session / uses_nomi_recovery 分支
RuntimeEngineSelector / Agent runtime_engine 新写入
任意 Runtime Family 注册
Nomi/Coding 双份 Capability mapping
官方 mcp.tool_proxy
Guid 浏览器专用 Session 创建
initial-browser-open
仅 Conversation 主 Agent 可使用 Browser 的硬编码
```

## 14. 完成定义

1. 官方产品只有一个 Agent Runtime Family 和一个当前 factory。
2. 所有 AgentPreset 不再选择 Runtime 或执行档位，只选择 Capability、Skill、资源和模型。
3. 简单与复杂任务使用同一个自适应模型循环、安全底座和 Host Ports。
4. Coding/长程能力通过真实压缩、续接、取消、恢复和效果测试。
5. 六个官方 Agent 全部在统一 Runtime 上运行。
6. Capability Catalog 不再把连接基础设施和 Runtime 内部机制暴露为普通能力。
7. 新 Session 不再出现 `nomifun.coding` 或旧 Nomi Manager。
8. 旧 Agent 数据不加载，不通过第二 Runtime 或 legacy reader 兼容。
9. 旧循环、私有恢复、多 Runtime Catalog/selector 和双份能力映射已物理删除。
10. 内部 Driver seam 足以让未来官方实现通过一次代码替换接管，而不改变 Kernel、Conversation
    和 Domain owner。
11. Browser 不再创建专属 Conversation；任意 Agent 只能依据自己的 Snapshot 和 Resource Binding
    使用统一 Browser Module。

最终架构追求的是“一套足够强、可按负载减重的官方 Runtime”，不是一个长期维护任意 Runtime
实现的框架。
