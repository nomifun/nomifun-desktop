# NomiFun Agent 能力模型全面检查与重构方案

> 日期：2026-09-16
> 状态：目标设计与实施输入
> 审查基线：`target-first-party-contributions.v1.json` 的 28 个 Package、136 个 Capability
> 关联方案：[统一 Nomi Agent Runtime 与能力组合架构](2026-09-16-unified-nomi-runtime-convergence.zh.md)
> 实施入口：[UARC 长程实施总计划](2026-09-16-unified-agent-overhaul/README.zh.md)

## 1. 结论

当前能力设计存在明显碎片化：大量完整产品能力被拆成“一个操作一个 Capability”，资源、连接、
中间件和后台服务又与用户权限放在同一层。工作台随后把这些条目作为同级选择项展示，导致：

- 官方 Agent 需要维护很长的 ID 清单；
- 用户难以理解一个 Agent 实际获得了什么；
- Runtime 为每个 ID 手写映射；
- 同一领域的权限、资源和实现机制彼此混淆；
- 每次增加操作都扩大 Preset、Compiler、Runtime 和测试矩阵。

目标不是取消 Capability，而是恢复它合适的粒度：

```text
Runtime invariant
  负责所有 Agent 的执行正确性

Capability Module
  表示一个稳定的产品能力领域，包含多个 Action/Context contribution

Action Grant
  表示该 Agent 在模块内被允许的具体操作

Resource Binding
  表示操作针对的 Workspace、知识库、设备、Server 等对象

Skill / MCP / Plugin
  向 Agent 绑定行为或发布新的 Capability Module/Action
```

当前合同已经具备 `CapabilityManifest.contributions.actions[]` 和
`CapabilitySelection.action_allowlist`。后者在本文统一称为 **Action 授权集合**：它记录一个
Agent 在某个能力模块内被允许调用的精确 Action ID。利用这个字段即可合并碎片，无需继续创建
操作级 Capability ID。

本文不把 Runtime 手写“支持哪些 Capability ID”的列表称为授权集合。那只是重复的实现清单，
目标设计会删除它；真正的权限来源只有编译后的 Snapshot、Action 授权集合和 Resource Binding。

### 1.1 对需求假设的批评性判断

| 可能的假设 | 判断 | 原因与修正 |
| --- | --- | --- |
| 超级 Agent 应默认拥有所有能力 | 不成立 | “可扩展到所有能力”不等于“默认全部授权”；默认全开会扩大本地数据、外部发送和物理控制风险 |
| 最简 Agent 也应拥有文件能力 | 不成立 | 最简 Agent 应是零外部 Tool 的安全基线；本机 Workspace 是敏感资源，必须显式绑定和授权 |
| 最简 Agent 不能处理附件 | 不成立 | 用户在当前消息主动提交的附件是 session-scoped input，不等于持续访问宿主文件系统 |
| 一个 Runtime 应实现所有领域能力 | 不成立 | Runtime 只实现执行机制；File、Browser、Memory、Robot 等必须保留独立 owner 和 Port |
| 一切都能力化最开放 | 不成立 | 取消、恢复、压缩、Transport、OAuth、Scheduler 等能力化后只会制造错误开关和组合爆炸 |
| Capability 越细权限越精确 | 不完全成立 | 权限精度应由 Module 内 Action 授权集合提供；每个 Action 建一个顶层 ID 会造成碎片化 |
| 通用 Agent 应默认启用 Skill/MCP/Schedule/Browser/Computer | 已由产品确认 | 官方通用 Preset 默认勾选这些模块；没有实际 Skill、MCP Server 或设备/Browser Provider 绑定时不产生对应 Tool |
| Coding 需要独立 Runtime 才能强大 | 不成立 | Coding 长程机制应沉淀到统一 Runtime；文件、VCS、Process 仍作为可选能力模块 |
| 历史兼容优先于模型清晰 | 不成立 | 本轮启用新 Agent 数据代际且不迁移旧日志；不能保留旧 ID、双 projection 和兼容 alias |

### 1.2 Runtime、Capability 与平台服务的判定法

| 判断问题 | 归属 |
| --- | --- |
| 无论 Agent 做什么都必须保证，否则执行不正确？ | Runtime invariant |
| 组织模型步骤、上下文和证据，但不产生新外部权限？ | Runtime internal mechanism |
| 读取敏感数据、调用外部服务或产生可观察效果？ | Capability Module / Action |
| 指向具体 Workspace、设备、Server、知识库或账户？ | Resource Binding |
| 建立连接、OAuth、消息循环、后台同步或定时唤醒？ | Platform/Domain service |
| 只决定底层使用哪个模型或 Provider 实现？ | Model Route / Role Provider |

#### 必须沉淀进 Runtime

- 模型 step/tool continuation 主循环；
- 流式事件和有界 Tool argument/result；
- context assembly、预算、compaction；
- cancel、interrupt、steering inbox 和 cleanup；
- plan/requirement/completion evidence；
- 通用 Tool 调度、只读并行/效果串行；
- Capability discovery/deferred exposure；
- effect/observation ledger、未知结果处理；
- history projection、显式 task continuation；
- checkpoint identity 和崩溃恢复；
- 文字、图片等已授权 Session input 的模型格式化。

这些机制可以向模型暴露内部控制 Tool，但它们没有外部 authority，不进入 AgentPreset。

#### 必须抽成 Capability

- Workspace 文件、VCS、Process；
- Web、Knowledge、Memory；
- Browser、Computer、Robot、SSH；
- Channel 发送、客服、通知发送；
- Creation、Workshop、Office、Plugin；
- Agent delegate/fork；
- Schedule 的创建、修改和删除；
- 每个具体 MCP/Connector Tool。

#### 必须由 Runtime 与 Capability 各持一半

| 场景 | Runtime 部分 | Capability/Owner 部分 |
| --- | --- | --- |
| 文件 Patch | 观察新鲜度、证据失效、重读门禁 | 路径、版本、原子写和真实文件效果 |
| Process | 调用链、完成证据、取消协调 | spawn、PTY、stdin、进程树和退出证明 |
| Skill | 选择已绑定 Skill、注入指令和资源 | Skill 所需的真实 Action 权限仍由 Capability 授予 |
| MCP | 编译已冻结 ToolPlan、调度调用 | 连接、OAuth、schema、资源和远端效果归 MCP owner |
| Browser/Computer | 模型循环、截图/结果上下文预算 | 会话、页面、系统输入和真实物理效果归对应 owner |
| AgentExecution | 当前模型回合理解和发起命令 | DAG、Attempt、人工决策和持久状态归 AgentExecution owner |

Runtime 不应硬编码 `fs.patch`、`process.exec` 等具体 ID 才能获得可靠性。Domain Tool result 应提供
标准 effect receipt、resource version、observation scope 和 cleanup evidence，Runtime 基于这些通用
事实维护证据账本；Domain owner 保留具体业务语义。

## 2. 从目标 Agent 反推能力

### 2.1 最简 Agent

最简 Agent 只包含：

- 模型路由；
- Persona、系统指令和 Starter prompts；
- Conversation 历史；
- Runtime 强制提供的流式输出、取消、恢复、上下文预算和压缩。

它没有 Skill、MCP、Workspace、Web、定时任务、Browser、Computer 或其他 Tool。ToolPlan 为空，
不会因为系统支持很多能力而构造重型上下文。

用户在当前消息中主动提交的文本和附件属于 Session input。附件权限只作用于该消息/Session，
不要求 AgentPreset 永久启用一个 `session.attachments.read` 能力。

### 2.2 通用 Agent

通用 Agent 默认授权并接入：

- Skill；
- MCP/Connector 具体 Tool；
- Web research；
- Knowledge 和 Project Memory；
- Schedule；
- Browser；
- Computer；
- Creation/Office；
- Agent collaboration。

“能够接入”不等于默认拥有所有高风险权限。官方通用 Preset 提供安全的推荐组合；用户可继续
增加模块或调整具体 Action。Skill/MCP/Schedule/Browser/Computer 模块默认勾选；实际能力仍取决于
已安装/绑定的 Skill、MCP Server、Browser Provider 和 Computer device。

### 2.3 Coding Agent

Coding Agent 的核心模块是：

- Workspace Files；
- VCS；
- Process；
- Web Research；
- Agent Collaboration；
- 可选 Skill、MCP、Browser 和 Computer。

Coding 的计划、需求账本、Patch recovery、工具历史、完成证据和任务续接属于统一 Runtime 的
自适应执行机制，不再作为 Agent Capability。

### 2.4 伙伴与物联网 Agent

伙伴 Agent 由以下模块组成：

- Companion；
- Companion Memory；
- Channel Messaging；
- Robot；
- Knowledge；
- 可选 Creation/Audio、Schedule 和 Connector。

Channel 配对、消息接收循环、Robot 连接和 Audio 后台服务属于资源/Domain 生命周期；Agent 只获得
回复、发送、观察、记忆和设备动作等权限。

### 2.5 泛娱乐创作 Agent

创作 Agent 由以下模块组成：

- Media Creation；
- Creative Workshop；
- Office；
- Asset/Artifact；
- 可选 Web、Knowledge、Browser 和 Connector。

底层具体模型、Embedding、Rerank、Realtime transport 不作为 Agent 能力，由 Model Route 和 Role
Provider 解析。

### 2.6 客服 Agent

现有客服场景保留为一个明确组合：

- Customer Service；
- Channel Messaging；
- Knowledge；
- 可选 Project Memory、Schedule 和 Agent Collaboration。

Dialogue middleware 由客服场景自动装配；用户配置的是 notes、handoff、send/reply 等真实权限。

### 2.7 自定义 Agent

自定义 Agent 从最简 Agent 开始，逐个增加能力模块和 Action 权限。工作台不要求用户理解
Runtime、Transport、ContextContributor 或 ResourceProvider 等内部术语。

## 3. 新能力模型

### 3.1 Capability Module

一个 Capability Module 表示稳定产品领域，可以同时贡献 Action、Context、Event 和 Host Port：

```text
CapabilityModuleManifest
  id / version / display
  actions[]
    id / schemas / effect_class / presentation
  context_contributions[]
  event_contributions[]
  required_resource_kinds[]
  required_host_ports[]
  dependencies[]
  conflicts[]
  supported_surfaces[]
```

当前单值 `CapabilityKind` 不应继续限制一个模块只能是 Tool、Context 或 ResourceProvider 之一。
`CapabilityContributions` 已经支持多种贡献，目标 schema 应删除或降级 `kind` 为 Catalog 展示摘要。

### 3.2 Agent Grant

```text
AgentCapabilityGrant
  module: ExactCapabilityRef
  allowed_actions[]
```

Context、middleware 和 event contribution 是模块合同的一部分。真正敏感且需要独立开关的行为必须
建成 Action；工作台以“权限项”展示，Compiler 冻结精确 Action 授权集合。

### 3.3 Resource Binding

Workspace、KnowledgeBase、MCP Server、Channel、Robot、SSH Host 等不再伪装成 Agent 能力。
Capability 声明需要哪类 Resource，Session/场景绑定具体实例和允许的操作。

```text
Capability Grant：允许 workspace.files/read
Resource Binding：只允许读取 workspace 019...
```

两者必须同时满足，Resource 只能收窄 Capability，不能升权。

### 3.4 Provider 与内部机制

以下内容不进入 AgentPreset：

- Runtime cancel/recovery/compaction；
- Transport、OAuth、pairing、ingress；
- Scheduler runner、message receive loop、后台同步；
- Embedding、Rerank、模型协议和 Provider 选择；
- Skill catalog/describe/hook 基础设施；
- MCP connect 和 generic proxy；
- 场景固定 middleware。

### 3.5 Extension 开放方式

开源扩展通过下列方式增加系统能力：

- Plugin 发布新的 Capability Module 和 Action schema；
- MCP Tool 物化为命名空间隔离的 Module/Action；
- Skill 绑定指令、资源并声明所需 Module Action；
- Domain package 提供 Host Port/handler；
- UI 从 Catalog metadata 自动展示模块和权限。

不需要新增 Runtime，也不能绕过 Kernel。

## 4. 目标产品能力模块

| 模块 | 主要 Action/Context | 典型资源 | 风险提示 |
| --- | --- | --- | --- |
| `agent.collaboration` | delegate、fork；Execution observe/steer 按 Session Role 派生 | process/execution | 创建或控制其他执行 |
| `automation.schedule` | create、list、update、delete | scheduler owner | 持久自动执行 |
| `web.research` | search、fetch；citation 为结果 helper | 网络 | 外部传输 |
| `knowledge` | search、read、write、autogen | knowledge_base | 敏感读取/持久写入 |
| `project.memory` | read、write；citation/distill 内部派生 | project_memory | 跨 Session 持久化 |
| `companion` | persona context、learn、evolve | companion | 角色状态变更 |
| `companion.memory` | recall、write；merge/evolve 内部维护 | companion_memory | 私密长期记忆 |
| `channel.messaging` | reply、send | channel | 对外发送 |
| `customer.service` | notes.read、notes.write、handoff | customer | 敏感数据/外部转接 |
| `workspace.files` | read、search、write、patch、delete、watch、snapshot | workspace | 文件修改/删除 |
| `workspace.vcs` | status、diff、stage、commit、push | workspace | 持久提交/外部推送 |
| `workspace.process` | exec/start、poll、input、resize、cancel | workspace/process session | 本地执行 |
| `workspace.artifacts` | read、publish | workspace/artifact store | 发布需显式授权 |
| `browser` | observe、navigate、act、render、download、upload、evaluate | browser session | 外部传输/执行脚本 |
| `computer` | observe、a11y、input、launch | computer session | 物理输入 |
| `robot` | vision、display、motion、device action | robot | 物理效果 |
| `creation.media` | text、image、image_edit、video、audio、music | model/task owner | 成本与持久任务 |
| `creative.workshop` | canvas/asset read/write、template run | canvas/asset library | 素材修改 |
| `office` | preview、document/sheet/slides edit | asset library | 文档修改 |
| `requirements` | read、write、status、claim | requirements owner | 项目事实变更 |
| `plugin.development` | read、edit、publish、serve | plugin | 发布/服务暴露 |
| `ssh` | fs.read、fs.write、exec、sudo | ssh_host | 远程执行/提权 |
| `connector.<source>` | 具体 read/write/invoke actions | connector/MCP server | 由每个连接精确物化 |

Notification、Remote ingress、IDMM、Channel receive、Knowledge sync 等继续作为 Domain/Automation
配置。如果未来需要 Agent 主动调用，新增明确的产品 Action，例如 `notification.send`，而不是把
后台 consumer 直接暴露给模型。

## 5. 官方 Agent 组合

| Agent | 默认模块 | 默认不开放的高风险 Action | 可选扩展 |
| --- | --- | --- | --- |
| 最简 Agent | 无 | 全部外部 Tool | 任意模块 |
| 通用 Agent | web.research、knowledge、project.memory、Skill、Connector/MCP、automation.schedule、browser、computer | 未绑定资源对应的 Action 不出现；外部发送和破坏性操作保持明确可见 | Creation、Collaboration、Office、SSH 等其他模块 |
| Coding Agent | workspace.files、workspace.vcs、workspace.process、web.research | delete、vcs.push、长期外部发布 | Skill、Connector、Browser、Computer、Collaboration |
| 伙伴 IoT Agent | companion、companion.memory、channel.messaging、robot observe | robot.motion/device、任意群发 | Knowledge、Creation/Audio、Schedule、Connector |
| 泛娱乐创作 Agent | creation.media、creative.workshop、office preview | publish、任意外部发送 | Web、Knowledge、Browser、Connector |
| 客服 Agent | customer.service、channel.messaging reply、knowledge read | 主动群发、无约束 notes 写入 | Project Memory、Schedule、Collaboration |
| 自定义 Agent | 无，从最简开始 | 全部 | 用户逐模块和 Action 开启 |

上述官方 Agent 名单和默认模块已经产品确认。模块内不可逆/发布/物理动作继续按表中默认关闭，
用户可在能力配置中明确开启。

## 6. 需求平台、AutoWork、IDMM 与 Runtime 的定位

```text
Requirements Platform
  保存用户/产品的需求事实、状态、标签、附件和审计
            │
            ▼
AutoWork
  认领待办需求、选择绑定 Agent、触发并等待执行、回写结果
            │
            ▼
AgentExecution / Conversation
  保存持久任务、Step、Attempt、Turn 和效果回执
            │
            ▼
Unified Nomi Runtime
  执行一个已准入 Agent 的当前 Turn
            │
            ▼
Capability Kernel → Requirements / File / Browser / ... owners
```

### 6.1 Requirements Platform：业务事实与工作入口

需求平台是独立 Domain Platform，不是 Runtime，也不是一个整体 Agent Capability。

它拥有：

- Requirement 实体、正文、标签、优先级和状态；
- 附件、认领代际、完成/失败/复核事实；
- Kanban、查询、通知和用户操作；
- Requirement 与自动执行之间的持久关系和审计。

它向 Agent 暴露一个产品级 `requirements` Capability Module，包含 read、write、status、claim 等
Action。只有获授权的 Agent 才能操作需求；普通 Agent 和 Runtime 不自动获得这些权限。

Runtime 内部的 requirement/plan ledger 只服务当前任务的执行完整性，是派生状态；Requirements
Platform 保存的是用户可管理的长期业务需求。两者名称相似，但不能共用数据模型或相互隐式写入。

### 6.2 AutoWork：自动化控制器

AutoWork 不是 Agent Runtime，也不是模型 Tool。它是 Requirements Platform 上的队列消费与调度
策略：

1. 按标签、状态和配置寻找可执行 Requirement；
2. 原子认领并建立执行代际；
3. 解析用户绑定的 exact AgentPreset；
4. 创建/继续 AgentExecution 或 Conversation Turn；
5. 等待 canonical receipt；
6. 将结果归约回 Requirement；
7. 根据策略继续下一项、暂停或请求人工处理。

因此：

- `autowork.runner`、`schedule.timer`、`schedule.agent_trigger` 是平台 Scheduler/Service，不进入
  AgentPreset；
- Agent 若需要创建或管理自动化，只获得 `automation.schedule` 和必要的 `requirements` Actions；
- AutoWork 不能增加被绑定 Agent 的 Capability，也不能向 Runtime 注入任意工具；
- Runtime 不知道本回合是用户手动触发还是 AutoWork 触发，只处理相同的 accepted turn 合同；
- AutoWork 的持久重试、队列和认领不应复制到 Runtime。

长远目标应让 AutoWork 复用 AgentExecution 的 Attempt、receipt、retry 和恢复能力，而不是维护
第二套任务执行状态机。AutoWork 只保留“选择下一条 Requirement 和应用自动化策略”的领域价值。

### 6.3 IDMM：当前是旁路监督器，目标应拆解

当前 IDMM（Intelligent Decision-Making Mode）在 Conversation/Terminal 外部轮询会话，检测 Provider
故障和决策停滞，通过规则或旁路模型进行干预。它与 AutoWork 配合时，AutoWork 提供推进，IDMM
尝试保证单轮存活。

在新的统一 Runtime 架构中，继续保留完整 IDMM 层会造成第二套故障判断、重试、决策和会话控制。
建议按职责拆解：

| 当前 IDMM 职责 | 目标 owner |
| --- | --- |
| Provider fault 分类、retry/failover | ChatModelBroker + Runtime model loop |
| Turn deadline、卡死检测、cancel、cleanup | Runtime invariant |
| 用户中途 steering | Runtime steering inbox |
| 缺少信息、关键决策、ask-user | Runtime 当前任务状态 + AgentExecution decision policy |
| 无人值守任务的 retry/pause/replan | AutoWork / AgentExecution adaptation policy |
| 介入记录和运行审计 | Conversation/AgentExecution canonical events |
| Terminal/第三方 CLI 卡死监督 | Terminal domain 的独立 Supervisor，不进入 Agent Runtime |

目标状态：

- `idmm.observe`、`idmm.intervene`、`idmm.fallback_policy` 不再是 Agent Capability；
- Agent Session 不再需要旁路模型偷偷形成第二个决策者；
- Runtime 内的恢复和决策逻辑遵守同一 Snapshot、输入来源、效果账本和终态；
- AutoWork 使用同一执行回执决定继续或暂停；
- Terminal 若仍需要守护，使用 Terminal 专用配置和实现。

职责迁移完成后，删除 Agent 路径中的独立 `nomifun-idmm` 监督循环、对应 UI 状态和 IDMM 产品名称；
Terminal 若仍需要守护，使用 Terminal Supervisor 的领域名称，不保留横跨所有会话的 IDMM 执行层。

### 6.4 三者与 AgentPreset 的关系

| 对象 | AgentPreset 中保存什么 |
| --- | --- |
| Requirements Platform | 可选 `requirements` Module 及精确 Actions |
| AutoWork | 不保存 runner；AutoWork 配置单独引用 exact AgentPreset Revision |
| IDMM | 不保存 IDMM Capability；需要的可靠性由 Runtime/Execution policy 强制提供 |
| AgentExecution | 可由 `agent.collaboration` Actions 发起，但持久 Execution 不属于 Preset |

## 7. Browser 产品模型纠正

### 7.1 当前问题

当前实现错误地把浏览器能力塑造成了一种特殊会话入口：

- `GuidPage.tsx` 在输入框旁直接显示“会话浏览器”；
- `useGuidSend.ts` 的 `launch('browser')` 创建一个标题为“会话浏览器”的新 AgentSession，且不发送
  初始消息；
- `initial-browser-open` sessionStorage 标记让新会话进入后自动展开浏览器面板；
- API 使用 `/api/conversations/{conversation_id}/browser`；
- `BrowserWorkspaceKey` 固定由 user + ConversationId 构造；
- `owned_conversation` 明确拒绝 `execution_step_id` 非空的 delegated Agent；
- “会话浏览器”“已登录 Chrome”“本地网页搜索”又被设计成三个用户可选能力。

这使 Browser 看起来像一种 Conversation 类型，并阻止其他 AgentExecution participant 按自己的
Snapshot 使用浏览器，与“Browser 是平台 Capability”冲突。

Conversation/AgentSession 作为浏览器资源的隔离范围本身没有问题；错误的是把这种内部资源范围
提升成一个专属产品入口和 Agent 类型。

### 7.2 目标模型

```text
任意 AgentPreset
  + browser Module grant / Action 授权集合
  + Browser Resource Binding / Provider
              │
              ▼
任意 AgentSession（普通、Coding、通用、自定义或 delegated）
              │
              ▼
Runtime 暴露 browser actions
              │
              ▼
Browser owner 创建或复用该 Session 的隔离 Browser Resource
```

核心规则：

- 不存在“浏览器会话”产品类型；
- 首页不通过打开浏览器创建空 Conversation；
- Browser 只在 Agent Snapshot 授权后进入 ToolPlan；
- Browser Resource 在第一次 Tool 调用或用户打开能力面板时懒创建；
- 普通 Agent、Coding Agent、自定义 Agent 和 delegated Agent 使用同一合同；
- delegated Agent 是否可用只由其 exact Snapshot、Resource Binding 和父执行权限决定，不能按
  `execution_step_id` 硬编码拒绝；
- 用户手动浏览与 Agent 操作共享同一 Resource 时，继续使用 RunGuard/InputGate 仲裁，不能并发抢占；
- Browser 生命周期跟随 AgentSession/resource lease，不跟随一个人为创建的“浏览器会话类型”。

### 7.3 Browser Module 与 Provider

`browser` Module 保留以下权限：

- observe；
- navigate；
- act；
- render/read content；
- download；
- upload；
- evaluate。

Provider/Resource 决定实际使用哪种浏览器：

| 当前实现 | 目标定位 |
| --- | --- |
| 当前 BrowserWorkspace 内置隔离浏览器 | `browser` Module 的 managed/isolated Provider |
| 当前 `nomi_system_browser` 已登录 Chrome | `browser` Module 的 attached Chrome Provider，不再是独立 Capability |
| Knowledge headless renderer | Knowledge service 使用的 Browser Provider，不取得 Agent Session 权限 |
| `nomi_local_websearch` | `web.research/search` Provider，不属于 Browser Module |

Agent 选择 Browser 权限，平台/资源绑定选择 Provider。Provider 不得改变允许的 Action，也不允许
因为某个 Provider 可用就自动给 Agent 增加 Browser 权限。

Attached Chrome 采用简单的安装级连接：用户完成一次本地连接后，连接可被当前用户下具有
`browser` grant 的 AgentSession 复用，不再设计每 Session/每 Tab 的二次授权、复杂租约或重复确认。
只保留两个基础边界：当前安装用户拥有该连接，Agent Snapshot 具有 Browser Action 权限。用户主动
断开连接后全部 Session 立即失去该 Provider；不复制 Cookie、密码或 Profile 数据。

### 7.4 UI 目标

- 删除 Guid 首页“会话浏览器”按钮；
- 删除 `launch('browser')`、浏览器专用空 Session 创建和 `initial-browser-open`；
- Conversation 内可保留可视化 Browser Panel，但它是当前 Session 的 Capability Surface；
- Panel 只在当前 Agent 有 Browser grant、存在 Browser activity，或用户从统一能力栏打开时显示；
- `BrowserWorkspacePanel` 产品概念改为通用 `BrowserPanel`；
- 文案使用“浏览器”，Provider 详情可显示“内置隔离浏览器”或“已授权 Chrome”；
- Markdown/link 打开浏览器时使用当前 Session 的 Browser resource，不创建新会话；
- 没有 Browser grant 时，引导用户为当前 Agent 增加能力，而不是创建另一种会话。

### 7.5 Backend 目标

- 将 `BrowserWorkspaceService` 收敛为 Browser Resource/Provider owner；
- `BrowserWorkspaceKey` 改为 canonical AgentSession/resource binding 身份，不依赖旧 Conversation 产品类型；
- Browser API 通过已认证 AgentSession + exact Browser grant + binding 准入；
- 删除 `execution_step_id` 对 delegated Agent 的硬编码禁用；
- Kernel/Runtime 从统一 `browser` Module 生成 ToolPlan；
- 会话删除、Runtime teardown 和 resource lease 结束时走同一 cleanup；
- 当前进程内 BrowserWorkspace 状态无需迁移成新的持久业务对象。

### 7.6 需要删除或废弃

- Guid 浏览器入口和对应交互测试；
- 浏览器专用 Session 创建分支；
- `initial-browser-open` 存储合同；
- “会话浏览器”作为能力名和 Conversation 名称；
- `nomi_system_browser`、`nomi_local_websearch` 作为与 Browser 并列的 Agent 能力；
- 仅 Conversation 主 Agent 可拥有 Browser 的限制；
- 将 Browser resource existence 当成 Browser authority 的逻辑；
- 旧 `browser-workspace-v2` 文档中“浏览器是一种专属会话工作区/入口”的产品结论。

底层经过验证的 Chromium runtime、Tab、下载、权限、Website dialog、RunGuard 和原生 Surface 可以
复用；整改重点是产品身份、授权入口和资源绑定，不应重写浏览器引擎。

### 7.7 验收

1. 首页没有 Browser 专属会话入口，打开 Browser 不会创建 AgentSession。
2. 任意包含 `browser` grant 的 Agent 可在普通 Session 使用允许的 Action。
3. 无 grant 的 Agent 即使 Browser Provider 已安装也不能使用。
4. delegated Agent 在 exact Snapshot/Binding 允许时可使用，在未允许时 fail closed。
5. managed browser 与 attached Chrome 通过同一 Module 权限工作。
6. Web search 不依赖 Browser grant，Browser 也不隐式获得 Web search。
7. 两个 AgentSession 的 Browser Resource、Profile、Tab 和权限严格隔离。
8. 用户/Agent input gate、下载、权限提示、对话框和 cleanup 回归通过。
9. 生产路由和 UI 不再使用“会话浏览器”作为产品类型。

## 8. 136 项当前能力的完整归并

以下 28 行覆盖当前 first-party inventory 的全部 136 个 ID。

| 当前 Package | 当前 Capability ID | 目标 |
| --- | --- | --- |
| `nomifun.agent-execution` | `agent.delegate`、`agent.fork`、`agent.execution.observe`、`agent.execution.steer`、`agent.execution.plan` | 前两项进入 `agent.collaboration`；observe/steer 按 Execution Role 派生；plan 进入 Runtime internal |
| `nomifun.autowork-scheduler` | `schedule.store`、`autowork.runner`、`schedule.timer`、`schedule.agent_trigger` | `schedule.store` 合并为 `automation.schedule` actions；其余为 Scheduler owner 内部服务 |
| `nomifun.browser` | `browser.observe`、`browser.navigate`、`browser.act`、`browser.render_content`、`browser.download`、`browser.upload`、`browser.evaluate` | 合并为 `browser` Module 和 Action 授权集合 |
| `nomifun.channel` | `channel.reply`、`channel.send`、`channel.receive`、`channel.pairing`、`channel.group_policy` | reply/send 进入 `channel.messaging`；receive/pairing/policy 属于 Binding/Transport/Middleware |
| `nomifun.chat` | `session.attachments.read` | 当前消息附件形成 session-scoped input grant，不进入 Preset |
| `nomifun.companion` | `companion.persona`、`companion.roster`、`companion.learn`、`companion.evolve` | 合并为 `companion`；persona/roster 由绑定注入，learn/evolve 为 actions |
| `nomifun.companion-memory` | `memory.companion.recall`、`memory.companion.write`、`memory.companion.merge`、`memory.companion.evolve` | 合并为 `companion.memory`；recall/write 可授权，merge/evolve 为领域维护 |
| `nomifun.computer-a11y` | `computer.observe`、`a11y.observe`、`computer.input`、`computer.launch` | 合并为 `computer` Module；观察和控制分组授权 |
| `nomifun.creation` | `creation.text`、`creation.image`、`creation.image_edit`、`creation.video`、`creation.audio`、`creation.music` | 合并为 `creation.media` actions |
| `nomifun.customer-service` | `customer_service.dialogue`、`customer_service.notes.read`、`customer_service.notes.write`、`customer_service.handoff` | notes/handoff 合并为 `customer.service`；dialogue 为场景 middleware |
| `nomifun.idmm` | `idmm.observe`、`idmm.intervene`、`idmm.fallback_policy` | 平台决策 middleware，不作为 Agent grant |
| `nomifun.knowledge` | `knowledge.search`、`knowledge.read`、`knowledge.write`、`knowledge.autogen`、`knowledge.mount`、`knowledge.source.sync`、`knowledge.embedding`、`knowledge.rerank` | search/read/write/autogen 合并为 `knowledge`；mount 为 Binding；sync/embedding/rerank 为实现服务 |
| `nomifun.local-websearch` | `nomi_local_websearch` | `web.research/search` 的一种 Provider，不再是独立产品能力 |
| `nomifun.mcp-connectors` | `connector.data.read`、`connector.data.write`、`mcp.connect`、`mcp.oauth`、`mcp.resource`、`mcp.tool_proxy` | 具体 connector Tool 物化为命名空间 actions；连接/OAuth/resource 为 Binding；删除 generic proxy |
| `nomifun.model-media` | `llm.embedding`、`llm.rerank`、`llm.image.generate`、`llm.image.edit`、`llm.video.generate`、`llm.audio.tts`、`llm.audio.asr`、`llm.realtime`、`llm.vision` | Model Route/Role Provider 内部能力；vision 从图像资源授权与模型 trait 推导 |
| `nomifun.notification` | `notification.desktop`、`notification.webhook` | Automation/Notification 配置；如需模型调用，另建明确 `notification.send` action |
| `nomifun.office` | `office.preview`、`office.document.edit`、`office.sheet.edit`、`office.slides.edit` | 合并为 `office` Module/actions |
| `nomifun.plugin` | `plugin.read`、`plugin.edit`、`plugin.publish`、`plugin.serve` | 合并为 `plugin.development` Module/actions |
| `nomifun.project-memory` | `memory.project.read`、`memory.project.write`、`memory.project.distill`、`memory.project.citation`、`memory.session.scratch` | read/write 进入 `project.memory`；distill/citation 为内部派生；scratch 为 Resource |
| `nomifun.remote-ingress` | `remote.mcp`、`remote.rest`、`ingress.web`、`ingress.mobile`、`ingress.channel` | 平台 Transport/Ingress 配置，不作为 Agent grant |
| `nomifun.requirements` | `requirements.read`、`requirements.write`、`requirements.status`、`requirements.claim` | 合并为 `requirements` Module/actions |
| `nomifun.robot` | `robot.vision`、`robot.display`、`robot.motion`、`robot.device_tools`、`robot.link`、`robot.audio` | 前四项进入 `robot`；link 为 Resource Binding；audio 为后台 owner |
| `nomifun.skills` | `skill.catalog`、`skill.describe`、`skill.invoke`、`skill.hooks` | 全部由 Skill binding/locks 和 Runtime Skill Port 提供，不作为直接 Module |
| `nomifun.ssh` | `ssh.fs.read`、`ssh.fs.write`、`ssh.exec`、`ssh.sudo`、`ssh.connect` | 前四项合并为 `ssh` actions；connect 为 Resource Binding |
| `nomifun.system-browser` | `nomi_system_browser` | `browser` Module 的一种 Provider，不再是独立产品能力 |
| `nomifun.web-research` | `web.search`、`web.fetch`、`citation.render` | search/fetch 合并为 `web.research`；citation 从合法结果派生 |
| `nomifun.workshop` | `workshop.canvas.read`、`workshop.canvas.edit`、`workshop.asset.read`、`workshop.asset.write`、`workshop.template.run` | 合并为 `creative.workshop` Module/actions |
| `nomifun.workspace-execution` | `fs.read`、`fs.search`、`fs.write`、`fs.patch`、`fs.delete`、`fs.watch`、`fs.snapshot`、`vcs.status`、`vcs.diff`、`vcs.stage`、`vcs.commit`、`vcs.push`、`process.exec`、`process.session`、`terminal.pty`、`workspace.artifacts`、`workspace.bind` | 拆为 `workspace.files`、`workspace.vcs`、`workspace.process`、`workspace.artifacts`；session/pty/bind 为 Resource/Host Port |

## 9. 配置与按需加载

### 9.1 Authoring

工作台只展示 Capability Module：

```text
Workspace Files
  [x] Read/Search
  [x] Write/Patch
  [ ] Delete
  [ ] Watch

VCS
  [x] Status/Diff
  [ ] Stage/Commit
  [ ] Push
```

高级技术详情可以显示 canonical Action ID、effect class、资源要求和来源 Package，但用户不需要在
136 个实现条目间移动能力。

### 9.2 编译

```text
Module grants + Action 授权集合
→ dependencies / conflicts
→ resource requirements
→ exact action/schema/contribution locks
→ context/middleware/event plan
→ ResolvedSnapshot
```

### 9.3 Runtime 加载

```text
Snapshot 无 Action/Context
→ ToolPlan 为空，最简模型回合

Snapshot 有 Module
→ 只装配对应 schema、adapter 和绑定资源

存在 Skill/MCP/Plugin
→ 只装配已冻结的具体 contribution
```

Runtime 不扫描全局 Catalog，不把未选择能力暴露给模型，也不因安装了 Plugin/MCP 就自动增加权限。

## 10. 不迁移历史 Agent 数据的 clean cut

本次建立全新的 Preset/Snapshot/Session 数据代际，不翻译旧 Agent 数据：

1. 官方 Preset 从新 Module 定义重新 seed，不继续复制旧 0/16/35/24/9/8 清单。
2. 旧自定义 Agent、Revision、Snapshot、Session 和日志不进入新数据代际。
3. 不编写旧 ID → Module/Action 转换器，不增加 legacy reader。
4. Capability schema、Catalog、Compiler、Runtime、Store 和 UI 在同一版本切换。
5. 旧路径不再打开；按 DEC-13 只清空 Agent 数据，保留非 Agent 配置。
6. 新 baseline 不先创建历史表再通过迁移删除。

完整存储设计见
[Agent Session 与执行日志存储重构方案](2026-09-16-agent-session-storage-redesign.zh.md)。

## 11. 实施拆分

### CAP-01：Module 合同

- Capability Module 多 Action/Context/Event 合同；
- 去除单一 `CapabilityKind` 对模块的限制；
- Agent grant / Action 授权集合；
- Resource Binding 分离；
- Catalog UI metadata。

### CAP-02：完整映射和官方模块

- 将本文件 136 项映射固化为机器可校验 manifest；
- 建立目标模块和 Action schemas；
- Effect class、资源、依赖和冲突审计；
- 禁止平台内部条目进入 Agent authoring catalog。

### CAP-03：Compiler/Snapshot vNext

- 编译 Module/Action grant；
- 冻结 Context/Middleware/Event plan；
- 精确资源和 Contribution locks；
- 权威 preview compile；
- Snapshot 外调用 fail closed。

### CAP-04：Domain adapters

按目标模块逐个接入真实 owner。每个 Action 验证成功、拒绝、schema/resource mismatch、取消和未知
结果；禁止只登记 manifest 而没有 handler/port。

### CAP-05：Skill/MCP/Plugin 开放扩展

- Skill required actions；
- MCP Tool → namespaced Module/Action；
- Plugin Module publication；
- UI 自动展示；
- 所有执行经过 Kernel。

### CAP-06：AgentPreset 与工作台

- 六类目标 Agent + 客服组合；
- 最简 Agent 创建入口；
- Module/权限级 UI；
- Bundle 只做快捷选择；
- 保存前完整诊断。

### CAP-07：新数据代际与旧结构删除

- 新 Preset/Snapshot/Session baseline；
- 官方 Agent 重新 seed；
- 删除旧官方 seeds、projection、Runtime 手写支持清单和能力迁移 UI；
- 删除旧 Agent Store、私有日志和兼容读取；
- 不创建永久 alias 或迁移工具。

### CAP-08：产品验收

- 最简 Agent 无 ToolPlan；
- 通用 Agent 可按需加载 Skill/MCP/Schedule/Browser/Computer；
- Coding 长程任务；
- 伙伴 Channel/Memory/Robot；
- 创作 Media/Workshop/Office；
- 客服 Knowledge/Notes/Handoff；
- 自定义 Agent 从空模块开始逐项增加；
- Plugin/MCP 新模块无需修改 Runtime family 分支。

### CAP-09：Browser 产品模型纠正

- 删除 Browser 专属 Session 入口和 UI 状态；
- 将 BrowserWorkspace 改成 AgentSession-scoped Browser Resource；
- 合并 managed browser / attached Chrome 的 Module 权限；
- 允许获授权的 delegated Agent 使用；
- 迁移 API、Runtime projection、资源清理和 Panel；
- 废弃与目标模型冲突的 BrowserWorkspace 产品文档结论；
- 完成本节 7.7 的九项验收。

## 12. 产品裁决清单

### 12.1 已由当前会话明确裁决

| ID | 决定 | 状态 |
| --- | --- | --- |
| DEC-01 | 官方只维护一个 Nomi Runtime，不建设第三方/多 Runtime 产品能力 | 已确认 |
| DEC-02 | 未来官方 Runtime 重写通过内部切面直接替换，不建设灰度双 Runtime | 已确认 |
| DEC-03 | cancel、recovery、compaction 是强制 Runtime invariant，不是 Capability | 已确认 |
| DEC-04 | 不设置 light/standard/durable 静态档位，Runtime 按回合事件自适应 | 已确认 |
| DEC-05 | Browser 是任意 Agent 可授权使用的平台能力，删除“会话浏览器”专属入口 | 已确认 |
| DEC-06 | 能力模型应按长期最优重构，不永久保留历史 ID、双 projection 和兼容债务 | 已确认原则 |
| DEC-07 | 最简 Agent 无外部 Tool/Workspace，但可读取用户当前消息明确提交的附件 | 已确认 |
| DEC-08 | 通用 Agent 默认勾选 Skill/MCP/Schedule/Browser/Computer；实际 Tool 由绑定资源物化 | 已确认 |
| DEC-09 | Agent 路径拆解并删除独立 IDMM；Terminal 需要时保留专用 Supervisor | 已确认 |
| DEC-10 | AutoWork 保留需求队列策略，但执行/Attempt/retry/receipt 复用 AgentExecution | 已确认 |
| DEC-11 | Attached Chrome 一次安装级连接即可被 Browser 授权 Agent 使用，不做每 Session/Tab 二次授权 | 已确认 |
| DEC-12 | 官方 Agent 名单及默认模块/Action 按 §5 推荐表执行 | 已确认 |
| DEC-13 | Agent 数据采用 clean cut 方案 A：重置 Session/Execution/Preset/日志，保留非 Agent 配置 | 已确认 |

### 12.2 裁决状态

当前方案中的产品方向均已确认，没有待裁决项。后续新增会改变默认权限、删除其他非 Agent 数据、
恢复多 Runtime 或重新建立独立产品入口的内容，必须另行登记新决定。

### 12.3 不再需要产品裁决的 UI 实施原则

原 DEC-P06 不是架构或权限决定，降为研发实现原则：工作台显示用户能理解的能力模块复选框；只有
模块内确实存在独立权限意义时才显示 read/write/control 等子项。canonical Module/Action ID 仅放在
技术详情中，不设计一套“普通/高级模式”产品概念。

## 13. 完成定义

1. 136 个旧 ID 全部有明确的合并、派生、平台归属或删除结果。
2. 工作台展示稳定产品模块和 Action 权限，不展示资源容器、Transport 或 Runtime 内部机制。
3. 最简 Agent 的 ResolvedSnapshot 没有 Tool/Context/Middleware contribution。
4. 所有官方场景都由同一个 Runtime 和 Module grants 构造。
5. Skill、MCP、Plugin 可以增加能力，但不能增加 Runtime 或绕过 Kernel。
6. 新增一个 Action 不需要创建新的顶层 Capability ID。
7. 每个外部效果和敏感读取仍有明确权限，不因合并模块而扩大授权。
8. 旧 ID、旧 projection、旧 seeds 和长期兼容代码从生产路径删除。
9. Requirements 只通过明确 Module Actions 向 Agent 开放，业务事实不进入 Runtime。
10. AutoWork 只负责队列/触发策略并复用 AgentExecution，不形成第二套 Runtime 或任务状态机。
11. Agent 路径不再依赖独立 IDMM 监督循环；相关职责已归还 Runtime、Broker、AgentExecution 和
    Terminal owner。
12. Browser 是任意 Agent 可授权加载的平台能力，不再存在“会话浏览器”专属入口或主 Agent 限制。

最终模型必须同时满足两个目标：对用户足够简单，对 Kernel 足够精确。简单来自领域模块，精确来自
Action 授权集合、Resource Binding、effect class 和 exact Snapshot，而不是把每个实现细节都变成
一个用户能力。
