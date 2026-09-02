# Capability 目录与 Agent Preset 产品/领域设计（经 05 修订）

> 文档性质：这是经 2026-09-02 止损修订后的**产品与领域设计**，用于说明
> Capability Catalog、Agent Preset、typed resource binding、Compiler、Snapshot
> 与 AgentSession 的长期语义；它不是实施状态、TODO、Gate、Evidence 或发布台账。
>
> 修订依据：`05-system-capability-replacement-foundation.zh.md`。两份文档发生冲突时，
> 以 05 的止损规则和 Browser/Computer Role Provider 合同为准。
>
> 机器事实：落地后的 canonical Rust types、SQL、API schema 和行为测试是字段级事实。
> 本文不复制第二套需要逐字段同步的机器合同。

## 0. 修订边界

本次修订保留以下长期产品设计：

- Package、Capability、Skill、MCP Tool Mapping 四层领域边界；
- 版本化 Agent Preset 与不可变 Revision；
- initial/on-demand Capability 组合；
- typed resource binding、principal 与业务 ownership；
- 单一 canonical Compiler；
- 只包含实际执行闭包的 Resolved Snapshot；
- 单一 AgentSession、语义 SessionEvent、cursor 和 UI Projection；
- 固定 Codex-derived Agent Runtime 与 FullAuto 执行语义；
- Browser/Computer canonical façade 与精确 Provider binding；
- 第一方 Package 与未来扩展使用同一注册、物化和调用主链。

以下旧约束不再属于本文：

- 不冻结 Capability 总数，也不把某个历史目录数量作为产品合同；
- 不冻结官方模板数量，也不要求所有候选模板在一期同时可运行；
- 不以 action-bearing 条目全部拥有 production owner 作为一期完成条件；
- Wave 3/4 非核心领域只保留候选目录，不默认注册、不 seed 到默认 Agent，也不以
  metadata-only handler、占位 DTO 或模拟成功冒充可用；
- 不定义在线 canary drain、祖先 deadline、shadow comparison 或多维 outstanding
  证明体系；
- 不定义固定五格平台矩阵、cohort tuple、whole-candidate recheck 或 C8/C10 证明体系；
- 不要求调用者填写计数为零的对象来证明 Runtime 已释放；
- 不建立通用 Effect coordinator、通用 receipt 状态机或所有写操作共用的 replay
  matrix；
- 不让未选择的全局 inventory、模板全集、决策文档 digest 或全局 schema ledger
  决定某个既有 Session 是否还能执行。

Catalog、模板和测试范围以后按真实产品闭环演进。新增条目必须有明确业务 owner、
真实消费者和代表性行为测试；没有这些条件的设计只属于候选，不进入生产默认路径。

## 1. 产品目标

用户创建和选择的是一份用途明确、可版本化的 Agent 设定，而不是手工拼装执行引擎。
产品入口提交：

```text
scene
principal
preset revision
typed resource bindings
user-facing overrides
```

系统使用唯一 Compiler 把已保存的 Revision 编译为不可变 Snapshot。AgentSession 创建后
只执行该 Snapshot 已冻结的能力、模型、资源和实现，不在运行中扫描 Catalog、追随最新
模板或自动扩大权限。

产品入口不直接提交：

```text
runtime family
package implementation
service locator
raw MCP tool
provider credential
approval mode
legacy engine fallback
```

v2 使用一个 Codex-derived Agent Runtime。Preset 不提供 Runtime 选择器，FullAuto 也不是
可持久化的模式字段。不同 Agent 的差异来自 Snapshot 中实际选择的 Capability、Skill、
模型路由、typed resources 和 Context，而不是来自多套执行循环。

系统只保留几项直接影响正确性的同步边界：

1. 入口建立 principal；
2. 领域服务校验 principal、owner 和业务资源；
3. Runtime 校验 Snapshot capability allowlist；
4. 调用按 typed resource binding 路由；
5. Remote ingress 先认证，再进入产品 Session；
6. Provider credential 由平台集中存储，Preset 只保存连接引用。

这些检查位于现有请求、Runtime dispatch 和领域服务边界，不为其增加新的持久审批
状态机。

## 2. 统一术语

### 2.1 Codex-derived Agent Runtime 与 FullAuto

v2 只有一个生产执行内核：Codex-derived Agent Runtime。它复用并按 NomiFun 产品边界改造
Codex 的 Thread/Turn、模型流、Tool loop、steer/cancel、compaction、恢复和运行时事件，
但不表示所有 Agent 都加载 Codex Coding Prompt、Workspace、Shell 或完整 Coding Tool。

Runtime 是平台部署组件，不是 Package、Capability、Preset 或用户可选择的 runtime
family。Preset/Snapshot 只声明当前执行闭包需要的 Runtime protocol/features；某次 Session
实际绑定的 build identity 由该 Session 的 runtime binding/Event 记录，不复制进 Preset
authoring 内容。

FullAuto 是唯一执行语义，不是可持久化的 mode。Snapshot 内且通过 principal、ownership、
resource binding、schema 与领域约束的调用直接执行；Snapshot 外、资源不匹配或实现不可用
时返回 typed failure。系统不为此增加 approval/confirmation 状态、legacy mode alias 或
另一套执行循环。

### 2.2 Capability Package

Package 是代码分发、mount、配置和 contribution 物化单元。一个 Package 可以贡献
Capability、Skill、MCP Tool Mapping、authoring template、host-only contribution，以及
少量已定义的系统能力 Provider。

Agent Preset 不选择 Package。Package 通过 `PluginHost` 注册和物化后，Preset 才能选择
其中已发布的 Capability 或 Skill。

普通 Package 采用 trusted in-process 模型。Snapshot allowlist、resource binding 和
ownership 约束的是正式 Agent 调用路径，不宣称隔离已经被用户安装到同一进程中的恶意
代码。Codex sidecar 是固定 Runtime 的部署边界，不是普通 Package 类型。

### 2.3 Capability

Capability 是 Agent 可以被授予的最小系统能力。它可以形成模型 Tool，也可以提供
Context、受控资源、事件入口、middleware、transport、scheduler 或 UI contribution。

Capability 必须有稳定 namespaced ID、版本、来源 Package 和明确执行语义。只有目录
metadata、没有实际 handler/factory 或真实消费者的条目，不算生产可用 Capability。

### 2.4 Skill

Skill 是任务方法、工作流说明、示例和领域知识，不是执行器。

Skill 可以声明 `requires_capabilities`。Compiler 只校验这些 Capability 已被 Revision
直接选中；Skill 不得自动加入 Capability、安装 Package、绑定 MCP 或扩大 Snapshot。

### 2.5 MCP

MCP 是外部工具和资源协议。每个对 Agent 可见的 MCP Tool 必须通过：

```text
server_id + canonical_tool_key + schema_hash -> CapabilityId
```

映射为一个 canonical Capability。Preset 选择 Capability，并用 typed resource binding
指向 MCP server/connection；Preset 不直接选择裸 MCP Tool。MCP discovery 变化不会修改
运行中的 Snapshot。

### 2.6 Agent Preset

Agent Preset 是用户可理解的、可版本化的组合配方。中文产品文案统一使用“Agent 设定”。

Preset 本身保存 metadata 和 Revision 关系；真正的 authoring 内容位于不可变
`AgentPresetRevision`。

### 2.7 Resolved Agent Snapshot

Resolved Snapshot 是一个 Revision 在特定可用 inventory、模型路由、Runtime contract
和资源绑定下的不可变编译结果。它是 AgentSession 的执行授权和恢复依据。

Snapshot 只锁定该 Session 实际选择和可能按需激活的闭包，不锁定无关的全局目录。

### 2.8 AgentSession

`AgentSession` 是产品历史与执行生命周期的唯一 aggregate，使用唯一
`AgentSessionId(UUIDv7)`。Chat、Coding、Remote、自动化和 Agent Editor 的“试用 Agent”
都创建同一种 Session，不建立第二个 Conversation 容器或测试专用 Session。

### 2.9 Typed Resource Binding

Typed resource binding 把抽象能力绑定到当前 principal 可使用的具体业务对象，例如
Workspace、Knowledge Base、MCP Server、Browser profile、Computer target、Channel 或
Robot。

它是 Capability 能否安全、确定地路由到真实资源的产品合同，不是 Prompt 文案，也不是
无 schema 的任意 JSON。

### 2.10 Execution Role 与 Provider

Execution Role 只用于 Browser/Computer 这类“canonical 能力稳定，但底层实现需要可替换”
的系统能力。它不是 Agent 人设，不进入四层 Catalog，也不是第五类用户选择对象。

Provider 是 Package 对某个 versioned Role Contract 的实现 contribution。Preset 仍然只
选择 `browser.*`、`computer.*` Capability；Compiler 在 Snapshot 中额外冻结其精确
Provider。

## 3. 四层领域边界

四层关系固定为：

```text
Package --materialize--> Capability
        --materialize--> Skill --requires subset of--> Capability
        --materialize--> MCP Tool Mapping --maps exactly to--> Capability

AgentPresetRevision --selects directly--> Capability[]
                    --selects directly--> Skill[]
                    --binds--> TypedResourceBinding[]
```

规则：

1. Package 负责分发、配置和物化，不进入 Preset selection；
2. Capability 是 Agent 功能授权的唯一原子单位；
3. Skill 只提供方法和知识，不提供隐藏执行能力；
4. MCP Tool 完成 canonical Capability 映射后才能进入 Agent；
5. `ServiceKey<T>` 只用于 Package 间 typed wiring，不进入 Catalog、Preset 或数据库选择；
6. authoring template 只用于创建 Revision，保存后展开为直接选择项；
7. Role Provider 只解释 canonical Browser/Computer Capability 由谁实现，不扩大
   Capability ceiling；
8. 不建立独立 Runtime contribution、Service Catalog、Provider DAG 或二次全局求解层。

## 4. Capability 类型与注册

### 4.1 Capability Kind

支持的 Kind 按真实执行需要增加，当前可使用：

| Kind | 作用 | 示例 |
|---|---|---|
| `tool` | 模型可调用动作 | `fs.patch`、`knowledge.search` |
| `context_contributor` | 组装模型上下文 | Persona、Memory 摘要 |
| `resource_provider` | 延迟取得受控资源 | Workspace、Browser lane |
| `event_source` | 把领域事件送入 Agent 流程 | Channel inbound、Cron trigger |
| `event_consumer` | 消费已提交领域事件 | Webhook notification |
| `turn_middleware` | 明确命名的 turn 前后行为 | Retrieval、IDMM observation |
| `transport` | 外部或本地协议接入 | MCP、IM、Remote |
| `scheduler` | 时间或队列触发 | Cron、AutoWork |
| `background_service` | Session 外长驻业务服务 | Browser Host、Robot link |
| `ui_contribution` | 声明式配置或状态展示 | Package 设置、能力状态 |

Kind 不是要求一次性实现的封闭数量表。新增 Kind 必须由现有 Kind 无法表达的真实产品
场景驱动，不能为未来可能性预建通用 Hook 或 Policy Engine。

### 4.2 Package 与 Contribution

下列 shape 只表达领域关系，精确字段由 canonical contract 定义：

```rust
struct PackageManifest {
    package_id: PackageId,
    package_version: Version,
    host_contract_version: Version,
    display: LocalizedMetadata,
    package_dependencies: Vec<ExactPackageDependency>,
    requires_services: Vec<ExactServiceRequirement>,
    config_schema: JsonSchema,
    contributions: PackageContributions,
}

struct PackageContributions {
    capabilities: Vec<CapabilityManifest>,
    skills: Vec<SkillDefinition>,
    mcp_tools: Vec<McpToolCapabilityMapping>,
    preset_templates: Vec<AgentPresetTemplate>,
    role_providers: Vec<RoleProviderContribution>,
    host: Vec<HostContributionDescriptor>,
}
```

序列化 Manifest 使用 strict JSON 和 canonical schema：拒绝未知字段、legacy alias、
注释与宽松语法。本文不冻结字段 exact-set，但下列语义必须由 canonical contract 表达：

- Package：schema/host contract、exact Package 与 Service dependencies、Runtime feature
  requirements、config schema 和 contributions；
- Capability：稳定 ID/version、来源 Package、Kind、exact dependencies/conflicts、surface、
  platform、Runtime feature 与执行 schema；
- Skill：稳定 ID/version、来源 Package、body/resources、surface 和
  `requires_capabilities`；
- MCP Mapping：Package、server、canonical tool key、schema hash、CapabilityId 和
  materialization revision。

Skill resource 可以包含 references、templates、examples 和 scripts，但 script 仍只是
模型可读取或引用的资源；只有 Snapshot 已授权 Shell/Process 或专用 Capability 时才能
执行。MCP schema 变化会使旧 mapping 不再适用于新 Session，必须重新 materialize 并生成
新的 Revision/Snapshot；运行时 discovery 不能改写已有 Snapshot。

Manifest 是声明事实源。Registration builder 从实际注册的 handlers、factories 和 services
派生运行 metadata；Package 不再手写第二份 `declared_*`、`allowed_operations` 或计数表
来证明 Registration 与 Manifest 相同。

物化时必须检查：

- Package、Capability、Skill 和 MCP identity 唯一；
- exact dependency 与 service dependency 可满足；
- Capability dependency 无环且 conflict 可判定；
- handler/factory 与声明的 Kind 匹配；
- Role Provider 的 contract、member 和 mount identity 有效；
- config 通过 Package schema；
- cleanup 可由 mount 或 Session 生命周期触发。

普通 Package 使用同一 `PluginRegistration` 和 materializer。第一方来源可以影响默认分发，
但不能形成 built-in-only parser、Catalog 分支或 Runtime shortcut。

### 4.3 Mount、配置与受信任状态边界

Package、Mount 与 Source 是三个不同事实：

- `PackageRef` 标识代码与 Manifest 的 exact version；
- `PluginMountId` 标识当前 Host 中已启用、已配置的实例；
- Source metadata 只记录 provenance，不参与 Capability 权限、Provider 优先级或
  Runtime fallback。

Package config 先通过该 Package 的 schema 校验，再按 `(package_id, mount_id)` 隔离。
Package 自有的小型持久状态使用
`(package_id, mount_id, scope_key, state_key)` namespace，并通过 bounded
`get/set/delete/compare-and-swap` Host state port 访问。普通 Package 不携带 SQL/DDL
migration，不能直接打开 root database、跨 Package namespace 或取得全局 service
locator。

注册上下文只暴露已声明的 typed service dependencies、窄 Host ports、取消信号和受管任务。
缺失、重复或 version 不匹配的 `ServiceKey<T>` 在当前 Host generation 建立时失败；
`ServiceKey<T>` 不持久化，也不进入 Catalog、Preset 或 Snapshot selection。

该边界约束正式 Package 接入方式，不宣称进程内 trusted code 已被 sandbox。若未来需要
运行不可信代码，应作为独立架构需求处理，不能把权限枚举或空隔离接口预埋到当前合同。

### 4.4 激活与资源取得

注册只发布 factory，不在应用启动时创建所有 Agent Tool 或外部连接。

Compiler 只校验并冻结所需 factory、schema 和 resource refs。Browser、MCP、SSH、PTY 等
外部资源在 Capability 第一次真实调用或 Context 组装确实需要时 lazy acquire，并登记到
Session 生命周期。Session dispose 时释放这些真实 handle。

## 5. Capability Catalog 治理

### 5.1 Catalog 不是固定清单

Catalog 是随已安装、已 materialize Package 演进的目录，不具有固定条目总数。

本节名称分为三种含义：

- **一期核心**：直接支撑当前发布所需用户闭环；
- **可选能力**：已有真实 owner 时可以发布，但不自动进入所有 Agent；
- **候选目录**：保留领域命名和边界，尚未形成 production owner/consumer/test 时不注册。

这三个标签是本文的阅读分类，不要求新增数据库状态枚举。运行时只认实际 materialized
manifest 和 Compiler 结果。

生产目录新增 Capability 的最低条件：

1. 有明确 owning Package 和业务服务；
2. 有至少一个真实产品消费者；
3. 输入、输出、资源和错误语义已确定；
4. 有代表性正常与失败测试；
5. 不需要 fake handler、metadata-only success 或旧主链 fallback。

### 5.2 一期核心目录

下列 ID 表达已经确认的核心能力边界。精确版本、schema 和当期是否 materialized 仍由
Package manifest 决定。

#### Workspace、Files、Process、VCS 与 SSH

```text
workspace.bind
workspace.artifacts

fs.read
fs.search
fs.write
fs.patch
fs.delete

process.exec
process.session
terminal.pty

vcs.status
vcs.diff
vcs.stage
vcs.commit
vcs.push

ssh.connect
ssh.fs.read
ssh.fs.write
ssh.exec
ssh.sudo
```

Local 与 SSH 使用不同 namespace 和 typed resource。系统不通过“替代 Provider”在本地与
远程文件实现之间自动求解。

文件能力只承诺基本正确性：用户选择 root、canonicalize、root containment、拒绝明显
越界、输入上限，以及同目录临时文件加 rename 的写入方式。首版不扩张为对同权限恶意
本地进程的逐 syscall TOCTOU 防御平台。

#### Knowledge

```text
knowledge.search
knowledge.read
knowledge.write
```

一期发布闭环优先保证选择 Knowledge Base、search 和 read。`knowledge.write` 只有在真实
写入产品入口、基本 containment 和原子替换完成后才 materialize；高级 autogen、
embedding、rerank 和全量 source sync 不因目录中有候选名而成为一期门槛。

#### MCP 与连接器

```text
mcp.connect
mcp.resource
mcp.oauth
```

业务 MCP Tool 各自映射到独立 canonical CapabilityId。`mcp.tool_proxy` 不能成为绕过
Snapshot allowlist 的万能全局调用入口。

#### Browser 与 Computer

```text
browser.observe
browser.navigate
browser.act
browser.render_content

computer.observe
computer.input
```

`browser.render_content` 是 Knowledge 等系统消费者使用的 hidden canonical member。
Browser 的 download/upload/evaluate/site memory/takeover 和 Computer launch/A11y 可以在
真实产品路径成熟后作为可选 member materialize。

Browser/Computer 的实现选择遵守第 14 章的 Role Provider 设计。平台可用性属于所选
Provider，不由某个第一方 build feature 提前定义整个 Role。

#### Agent 与自动化辅助

```text
session.attachments.read
agent.delegate
agent.fork
agent.execution.plan
agent.execution.steer
agent.execution.observe

schedule.store
schedule.timer
schedule.agent_trigger
```

基础 Session create、turn、stream、history、cancel 和 delete 是 AgentSession 协议，不是
Catalog Capability。因此 `chat.minimal` 可以正常对话，同时保持 Capability 集合为空。

一期只要求至少一个真实 scheduled/automation Session 走统一 Preset、Snapshot 和 Session
主链；不要求所有自动化候选域同时完成。

### 5.3 可选目录

下列能力在有真实 owner、资源 binding 和产品入口时可以发布，但不默认授予所有 Agent：

```text
web.search
web.fetch
citation.render

llm.realtime
llm.embedding
llm.rerank
llm.image.generate
llm.image.edit
llm.video.generate
llm.audio.tts
llm.audio.asr
llm.vision

memory.project.read
memory.project.write
memory.project.distill
memory.companion.recall
memory.companion.write
memory.session.scratch

browser.download
browser.upload
browser.evaluate
browser.site_memory
browser.takeover
computer.launch
a11y.observe
```

基础 Chat model route、Provider credential storage 和普通 reasoning model selection 属于
平台模型调用基础，不必包装成 Capability。媒体、Embedding、Rerank 等额外产品功能可以
由对应 Package 贡献 Capability。

### 5.4 Wave 3/4 候选目录

以下名称只保留领域划分和未来 authoring 参考。它们在没有真实 production repository、
owner、消费者和产品入口前：

- 不进入 production registration；
- 不进入默认 Agent Preset；
- 不生成批量 receipt/reconcile DTO；
- 不要求 fake handler 或 typed-unavailable placeholder；
- 不阻塞一期核心发布。

#### Requirement、AutoWork 与 IDMM 候选

```text
requirements.read
requirements.write
requirements.status
requirements.claim
autowork.runner
idmm.observe
idmm.intervene
```

Requirement 和 AutoWork 可以绑定任意兼容的 Agent Preset，不需要专用 Agent 类型。
`idmm.fallback_policy` 不作为运行时 fallback 开关保留；未来 IDMM 行为应由明确的
middleware 产品场景重新定义。

#### Companion、Channel、客服与 Robot 候选

```text
companion.persona
companion.roster
companion.summon
companion.learn
companion.evolve

channel.receive
channel.reply
channel.send
channel.pairing
channel.group_policy

customer_service.dialogue
customer_service.notes.read
customer_service.notes.write
customer_service.handoff

robot.link
robot.audio
robot.vision
robot.display
robot.motion
robot.device_tools
```

每个客户、群、伙伴和设备必须使用明确的业务 resource ID 与 owner。物理设备 Effect
属于 Robot 领域服务，不进入通用 Effect 平台。

#### Creation、Workshop、Office 与 MiniApp 候选

```text
creation.text
creation.image
creation.image_edit
creation.video
creation.audio

workshop.canvas.read
workshop.canvas.edit
workshop.asset.read
workshop.asset.write
workshop.template.run
workshop.director

office.preview
office.document.edit
office.sheet.edit
office.slides.edit

miniapp.read
miniapp.edit
miniapp.publish
miniapp.serve
```

Canvas revision、Asset ownership、文档格式、发布产物和 iframe policy 由各自 Package
拥有。只有真实业务模型和消费者存在后，才从候选目录提取最小 Capability 合同。

#### Notification 与其他 ingress 候选

```text
notification.webhook
notification.desktop
ingress.web
ingress.mobile
ingress.channel
```

Remote 的 `open/turn/observe/cancel` 是产品 ingress protocol，不需要为每个操作再创建一组
模型可见 Capability。

## 6. 业务 Package 边界

领域数据、repository、service、后台任务和 UI 由 owning Package 管理。Agent-facing
部分只通过 Capability contribution 暴露。

| Package 领域 | 业务 ownership | Agent-facing contribution |
|---|---|---|
| Workspace & Execution | 文件、VCS、进程、终端、SSH、artifact | Tool 与 Resource Provider |
| Knowledge | KB、source、retrieval、writeback | search/read/write/context |
| Memory | Project/Companion memory namespace | recall/write/context |
| Skills | Skill catalog 与 body | Skill/Context |
| MCP & Connectors | server、OAuth、connection | MCP mapping、Tool/Resource |
| Browser | engine、identity、lane、download | Browser Role Provider |
| Computer & A11y | observation、input、target ordering | Computer Role Provider |
| Automation | schedule、trigger、run | Scheduler/Event/Session consumer |
| Companion/Channel/Robot 等 | 各自业务事实与资源 | 仅已成熟的 typed contribution |

Thin Kernel 只能看到 ID、manifest、typed ports 和 contribution contract，不应持有
`KnowledgeService`、`BrowserSessionHub`、`ComputerRegistry` 等 concrete business type。

Composition Root 不得通过巨型 `AppServices`、`GatewayDeps` 或 Factory optional fields
向 Agent 逐项灌入业务服务。Gateway 如保留业务入口，也只能委托 canonical Agent
Platform/Capability 主链，不能形成第二执行路径。

Host-only 页面和服务，例如账号管理、Provider 配置、Browser 登录、设备管理和系统设置，
可以由 Package 注册，但不进入 Agent Preset。

## 7. Agent Preset 与 Revision

### 7.1 关系模型

```text
AgentPreset
  id
  metadata
  owner / source
  current_revision_ref

AgentPresetRevision
  revision_id / revision_no
  schema_version
  surfaces[]
  model_routes[]
  initial_capabilities[]
  on_demand_capabilities[]
  skill_bindings[]
  typed_resource_bindings[]
  persona / instructions
  context_policy
  execution_constraints
  created_by / created_at / reason
  revision_digest
```

Revision 保存后不可原地修改。编辑器 Save 总是创建新 Revision；“试用 Agent”对 dirty
draft 先执行普通 Save，对 clean draft 复用当前 Revision，然后创建普通 AgentSession。
不存在 TestRevision、DraftSnapshot 或 ephemeral execution。

### 7.2 Capability Selection

首版 selection 只保留被真实执行路径消费的字段：

```text
capability_ref
action_allowlist[]
resource_binding_refs[]
```

Initial/on-demand 由 selection 所在集合表达，不再重复保存 `required`、`exposure`、
destination constraints、context/tool budget override 或未传给 handler 的 config。
未来只有出现真实执行语义和消费者时，才为 selection 增加字段。

同一 CapabilityId 只能出现在一个集合：

- `initial_capabilities`：Session 启动后立即可见；
- `on_demand_capabilities`：属于 frozen ceiling，可在 turn boundary 激活。

两个集合只保存 direct roots。Compiler 为实际 roots 计算最小 dependency closure，不把
整个 Catalog 复制进 Snapshot。

### 7.3 Skill Binding

Skill Binding 只保存 exact Skill ref。Compiler 校验其 `requires_capabilities` 是 Revision
直接 Capability selections 的子集。缺失时返回诊断，由用户补选；系统不自动修改 draft。

### 7.4 Model Route

Preset 可以按真实 task 保存 model route，例如：

```text
chat
reasoning
image
image_edit
video
asr
tts
embedding
rerank
```

Route 保存 provider/model/config reference 和必要预算策略，不复制明文 credential。

## 8. Agent Template

Template 是创建 Revision 的 authoring 输入，不是长期执行依赖。应用模板后，Capability、
Skill 和 resource defaults 被展开到普通 Revision；Snapshot 不保存 template 或 pack key。

官方模板数量不是固定产品合同。模板只有在其核心能力、资源 picker、真实 Session 路径和
主要失败行为可用时才进入默认产品目录。

当前基础模板语义：

| Key | 产品语义 |
|---|---|
| `chat.minimal` | 正常 Chat，但 Capability、Skill、MCP、Workspace 与 Coding Context 为空，最终模型请求 `tools=[]` |
| `coding.codex` | 覆盖当前正式声明的完整核心 Coding surface，不通过通用弱化 Tool 或 mock 补齐 |

以下名称可作为后续产品模板候选，但不因名称存在而默认注册或阻塞一期：

```text
assistant.general
companion.default
robot.default
customer-service.default
creative-studio.default
```

候选模板成为默认项前，必须满足：

- 对应 Capability 已有真实 owner 和消费者；
- 默认资源使用 picker 或 typed slot，不写入具体用户资源 ID；
- Preview 能解释缺失资源和平台不可用；
- 创建 Session 后走同一 Compiler、Snapshot、Runtime 和 SessionEvent 主链；
- 不以 placeholder、fake handler 或静默删减能力形成“可运行”。

Research 更适合作为编辑器中的 Capability bulk action。应用后只保存展开的 direct
selections，不创建 Research Agent 类型。

用户从任意模板 fork 后，可以从当前已安装且兼容的 Catalog 增加 Capability 或 Skill，
并保存新 Revision。运行中的 Agent 仍不能从 Catalog 扩大自己的 frozen ceiling。

## 9. Typed Resource Binding

每种 binding 必须有明确 schema。共同语义至少包括：

```text
resource_kind
resource_id
owner_ref
operations[]
optional connection_config_ref
```

典型资源：

| 领域 | Binding 内容 |
|---|---|
| Filesystem | Workspace root、read/write/execute |
| Knowledge | KB ID、search/read/write |
| Memory | namespace、owner resource ID、recall/write |
| MCP | server ID、Tool mapping、connection config ref |
| Browser | profile/lane/origin 等所选 Provider 需要的资源 |
| Computer | display/window/app/remote target |
| SSH | host connection、filesystem/exec/sudo 能力 |
| Channel/Robot | account/channel/device 和允许动作 |

Runtime 把 principal、Snapshot 和 exact binding ref 传给 owning domain。领域服务在调用时
校验 owner、资源存在性和 operation，不只依赖 Compiler 的早期检查。

Provider identity 不能塞进 `typed_parameters`。Browser/Computer Provider 由
`ResolvedRoleProviderLock` 冻结，resource binding 只描述该 Provider 实际需要的业务
资源。

Compiler 不因“可能会用”就打开资源。真实连接和 handle 由对应 factory lazy acquire，
并随 Session 或 non-Agent operation teardown。

## 10. 单一 Canonical Compiler

Preview、Save 和 Test 必须调用同一个纯 Compiler：

```text
Preview ─┐
Save ────┼─> one canonical Compiler
Test ────┘          │
                    └─> Snapshot + authority + diagnostics

Session Open ─> 读取已保存 Snapshot + 当前执行兼容检查
```

Control Plane 只把 Compiler diagnostics 映射为产品 DTO，不复制 dependency closure、
RuntimeProfile、Snapshot digest 或 Provider selection 算法。

Compiler 的确定性流程：

1. 读取 exact AgentPresetRevision；
2. 校验 initial/on-demand direct roots 不重复；
3. 只为这些 roots 计算最小 dependency closure 和 conflict；
4. 冻结 exact Skill，并校验 Skill requirement 子集；
5. 为已选择的 MCP-backed Capability 校验 mapping 和 schema hash；
6. 为实际选择的 Browser/Computer member解析 exact Role Provider；
7. 校验所选闭包需要的 platform、model route、typed resources 和 Runtime features；
8. 生成 initial plan、on-demand plan、authority 和 diagnostics；
9. 生成一个 Resolved Snapshot。

Compiler 不做：

- 扫描未绑定 MCP server 或 Package 目录来发现候选；
- 为 Skill、模板或 MCP 自动补选 Capability；
- 为未选择的 Capability 解析资源或 Provider；
- 生成多个 Runtime/Provider 候选并评分；
- 在 Session Open 再编译一份 Snapshot；
- 为 checkpoint 建立第二套 compatibility 算法。

## 11. 小闭包 Snapshot 与 RuntimeProfile

### 11.1 Snapshot 必须冻结

Snapshot 只锁定实际执行闭包：

- exact Preset Revision ref；
- 实际选中的 Capability 和最小 dependency closure；
- 对应 Package/Mount/contribution provenance；
- 实际 Tool schema、Context plan 和 action allowlist；
- exact Skill body/version；
- 已选 MCP Tool mapping 与 schema hash；
- model route 与 connection config revision；
- typed resource binding refs；
- 当前执行所需 Runtime protocol/features；
- initial/on-demand 分组与 activation plan；
- Browser/Computer exact Provider lock；
- Snapshot content digest。

### 11.2 Snapshot 不锁定

下列全局事实与当前 Session 无关，不得进入其执行兼容判断：

- 未选择的 Package 或 Capability；
- 整个 target inventory；
- 默认模板全集；
- 文档或决策 digest；
- 全局 schema ledger；
- 其他平台的 release evidence；
- 未使用 Provider 的版本和健康状态。

### 11.3 RuntimeProfile

`CompiledRuntimeProfile` 是 Compiler 的内部派生结果，不是用户字段。它把 Snapshot 翻译为
当前固定 Runtime 可消费的 instructions、feature flags、Tool/Context plan 和 compact
on-demand index。

`chat.minimal` 从空 Capability 集合正向构造，不先初始化 Coding、Workspace、Git、Shell、
Skill、MCP、Knowledge、Browser 或 Computer 再过滤。

`coding.codex` 保留当前正式声明的完整核心 Coding 能力。完整性由实际 manifest 和正常
功能测试确认，不通过固定 Capability 数量、统计 benchmark 或另一套 reference runner
证明。

### 11.4 On-demand Activation

On-demand 只在 frozen Snapshot ceiling 内工作：

1. Compiler 已预计算该 root 的最小 activation plan；
2. Runtime 在 turn boundary 把选中的 plan 合并进 active set；
3. 激活不再次调用 Compiler，也不重新选择 Provider、模型或资源；
4. 外部资源在第一次实际调用时 lazy acquire；
5. Snapshot 外 Capability 返回 `CAPABILITY_NOT_IN_PRESET`。

Active set 是 Session 执行状态，不是第二份 Snapshot。它只能在 Snapshot ceiling 内单调
增加。

### 11.5 Compatibility

兼容性在建立 Runtime binding、激活实际 Capability 或其执行实现变化时检查并缓存，不在
每个普通 Turn 对整个 ceiling 和全局 inventory 重算。

如果原 Snapshot 所需的 exact Capability、Provider、schema 或 Runtime feature 不再可用，
原 Session 历史保持可读，执行返回 `SNAPSHOT_EXECUTOR_UNAVAILABLE`。系统不改写原
Snapshot、不静默换 Provider、不降级 Coding。用户需要继续工作时创建使用新 Snapshot 的
新 Session 或显式 fork。

## 12. 统一 AgentSession、Event 与 Projection

### 12.1 唯一 Session

所有入口使用同一个 `AgentSessionId(UUIDv7)`：

- 本地 Chat；
- Coding；
- Agent Editor 试用；
- Remote；
- scheduled/automation run；
- Companion、Requirement 或其他成熟业务入口。

不存在第二个公开 Session ID、opaque handle、Conversation 映射或 Remote transcript。

### 12.2 SessionEvent

SessionEvent 保存恢复和产品历史真正需要的语义事实：

- Session/turn 生命周期；
- 最终用户消息；
- 最终 assistant message；
- 中断时最多一份 bounded partial；
- Capability activation；
- Tool call 和 bounded final result；
- 外部不确定 Effect 的 reservation/result reference；
- completed compaction；
- fork provenance；
- Runtime binding/checkpoint reference。

模型 token/delta、raw provider stream、typing heartbeat、重复 progress、中间 reasoning 和
完整 stdout/stderr 默认 transient。正常完成只持久化最终 assistant message。

Event append 使用单一写入口和 canonical cursor。commit 后再通知 UI/Remote；断线后按
cursor 补读，不让 stream 成为第二事实源。

### 12.3 Projection

Projection 只保存 UI 当前需要的派生状态：

- Session status 和最后 cursor；
- 最终用户/助手文本；
- Tool 摘要、状态和稳定引用；
- 当前 active capability 摘要；
- Runtime/checkpoint 可用性摘要。

Projection 不内嵌完整 `events[]`，也不复制 Event Log。它可以删除并从 Session fact 与
SessionEvent 重建；不能反向成为写入权威。

### 12.4 Checkpoint

Codex rollout/checkpoint 是绑定 exact Snapshot 和 event cursor 的可丢弃 Runtime cache。
缺失、损坏或不匹配时丢弃；只有当前执行环境仍兼容原 Snapshot 时，才从原 Snapshot 和
语义 Event 重建 Runtime binding。

不开发 checkpoint converter，也不把 checkpoint blob 当作产品历史。

### 12.5 删除

所有 Session 使用同一简化删除语义：

```text
live
-> deleting admission fence
-> 停止新写入
-> cooperative dispose
-> timeout 后 hard-kill descendant process tree
-> 幂等删除 Session 自有内容
-> minimal tombstone
```

Runtime 返回真实 `RuntimeDisposeReport`。Session Store 只删除自己拥有的数据，并在启动
时发现 `deleting` 后继续幂等清理。

调用者不填写 `ZeroOutstandingProof`、handle 计数表或发布级证明对象。清理是否完成由
Runtime/Store 的真实操作结果决定。

删除 Session 不撤销外部世界已经发生的业务 Effect。owning domain 可以保留最小
`agent_session_id` provenance，但不能依赖被删除的 Prompt、消息或 Runtime Context。

## 13. Effect 语义

一期只保留三种执行策略：

```text
read_only
managed_effect
external_uncertain_effect
```

### 13.1 Read-only

只读取本地或远程状态，不创建 Effect 状态机。SessionEvent 只记录模型需要看到的 bounded
Tool result。

### 13.2 Managed Effect

本地 DB、KV、文件和 VCS 使用 owning domain 已有的 transaction、revision/CAS 或原子文件
操作。成功或失败后记录一个最终 Tool result，不为每种本地写入复制通用
`started/succeeded/failed/reconciled` 流程。

### 13.3 External Uncertain Effect

外部发送、远程命令、设备控制等结果可能未知的操作，在 dispatch 前由 owning domain
保存必要的 idempotency identity/reservation。结果 unknown 时：

- 当前调用明确返回不确定；
- Runtime、resume、Remote redelivery 和 replay 不自动重试；
- 由 owning domain 使用原 identity 查询或 reconcile；
- SessionEvent 只保存 bounded 状态与领域记录引用。

`EffectClass` 可以作为展示和路由 metadata，但不建立全局 `EffectCoordinator`，也不要求
所有 Capability 生成通用 `EffectReceipt`。领域确有可靠投递或审计需求时，由领域自己的
表和 outbox 负责。

## 14. Browser/Computer 可替换实现

### 14.1 稳定 Façade

Agent、Automation、Remote 和 Knowledge 只依赖 canonical Capability：

```text
system.browser_use  -> browser.*
system.computer_use -> computer.*
```

第一方 Browser/Computer 是默认 Provider，不是唯一可被架构表达的实现。

### 14.2 Provider Contribution

Package 可以对 versioned Role Contract 贡献：

```text
Role contract
Package ref
Mount ID
Contribution digest
Member exports
Per-member platform/resource requirements
```

Provider 不注册或抢占 `browser.*` / `computer.*` ID。Role Provider 是四层模型内部的实现
贡献，不成为 Preset 可直接选择的新目录。

### 14.3 Binding 与 Snapshot Lock

选择顺序：

```text
Agent Revision exact override
-> installation default binding
-> typed failure
```

没有 override 表示继承 installation default，不增加 `latest/follow/auto` 状态。

Compiler 先选择 exact Provider，再按该 Provider 对实际 member 的 platform/resource
要求编译，并把 Package、Mount、contract 和 contribution digest 写入
`ResolvedRoleProviderLock`。

运行期只读取 Snapshot lock，不重新读取当前默认 Provider。Provider 缺失或不兼容时明确
失败，不静默回退第一方实现。

### 14.4 统一 Dispatch

Tool、ContextContributor 和 ResourceProvider 都必须读取同一个 lock：

1. Kernel 对 canonical Capability 做 authority 检查；
2. 从 Snapshot 读取 exact Provider；
3. 在当前 Registry generation 找到 Provider member；
4. 使用 Provider Mount 的 config、state、services 和 frozen resources；
5. 保持原 Capability/Action/idempotency identity，只调用一次；
6. 原样返回结果或 typed error。

不得先调用 façade handler，再回调 Kernel 进行第二次分发；也不得让第一方路径绕过
`role_provider_index`。

### 14.5 Browser 与 Knowledge

Knowledge 的网页渲染使用 `browser.render_content`，不得直接调用第一方
`BrowserSessionHub`。这样替换 Browser Provider 后，Chat 和 Knowledge 不会分裂成两条
实现路径。

Browser owner/lane/profile/close/cancel 和 process cleanup 继续由具体 Provider 保证，Role
抽象不能把它降级成无状态 Tool bag。

### 14.6 Computer Ordering

Computer 对同一 exact target resource 的物理 action 使用公共调用级 arbiter 串行化。
锁属于 target，不属于某个 Provider，也不跨 observe、模型思考和下一次 input 长时间持有。
observation ref 由 Provider 使用 generation 校验过期。

## 15. Agent Binding 与产品入口

持续业务对象统一引用：

```rust
struct AgentBindingValue {
    preset_revision_ref: PresetRevisionRef,
    resolved_snapshot_ref: ResolvedSnapshotRef,
    typed_resource_bindings: TypedResourceBindings,
    binding_version: u64,
}
```

Revision ref 表示用户保存的 authoring 内容，Snapshot ref 表示该内容的具体可执行编译
结果。两者不能混为一个 digest，也不能只保存 Revision 后在每次运行时重新解析到 latest。

适用入口：

- 新 Chat/Coding Session；
- scheduled/automation job；
- Requirement、Companion、Channel 等成熟 target；
- RemoteBinding；
- non-Agent operation 需要精确 Browser/Computer Provider 时的 operation context。

Binding 更新只影响之后创建的 Session/run。既有 Session 继续使用创建时冻结的 Snapshot。

### 15.1 Remote

RemoteBinding 只增加 Remote 自己的：

```text
binding_id
owner
name
AgentBindingValue
```

installation credential 与 Binding 分离。Remote REST/MCP 只适配：

```text
open
turn
observe
cancel
```

`open` 返回 canonical `agent_session_id`。后续操作显式提交同一个 ID；token、IP、HTTP
connection、MCP transport session 或“最近 Session”都不能隐式选择产品 Session。

Remote 不提供 Capability scope、Runtime mode、confirmation 或全局 Registry bypass。

## 16. 产品 UI

普通 Agent 编辑器默认展示用户能理解的产品信息：

- 名称与用途；
- 模型选择；
- 按任务分组的能力开关；
- Workspace、Knowledge、MCP/连接器等 picker；
- 保存；
- 试用 Agent。

Initial/on-demand 可由模板和 Capability metadata 提供默认值；高级覆盖放在开发者视图。
普通用户不需要手填 CapabilityId、Snapshot digest、ResourceId、owner 或 canonical JSON。

Revision、Snapshot、Provider provenance、protocol 和 raw diagnostic 放入折叠的技术详情，
不成为主流程。

“试用 Agent”执行：

```text
dirty draft -> 普通 Save Revision
clean draft -> 复用当前 Revision
-> canonical Compiler
-> 普通 AgentSession
```

它使用真实 resource binding 和真实 Effect 语义，不增加 mock/suppressed Effect 模式。

模板页展示当前真正可用的产品模板，不承诺固定卡片数量。候选模板在后端能力和资源流程
未完成前不出现在默认入口。

## 17. API 与持久化边界

本文只规定领域边界，不冻结一份庞大的 route/table exact-set。

API 至少需要覆盖：

- materialized Package、Capability、Skill 和 MCP mapping 查询；
- Agent Preset 创建、Revision 保存和 Preview；
- AgentSession create/read/turn/events/messages/fork/delete；
- Agent binding 和 RemoteBinding；
- Remote open/turn/observe/cancel。

所有 Session API 使用同一个 `AgentSessionId`。产品不提供 test-only Session route、
Runtime selector 或 public capability activation mutation。

持久化保留真正被产品读取的事实：

- `agent_presets`；
- immutable Revision payload；
- Snapshot envelope；
- Agent/Remote binding；
- 必须独立索引的 model route；
- AgentSession fact、SessionEvent 和轻量 Projection。

应删除或合并没有真实查询者的重复数据：

- 与 Snapshot JSON 重复的 capability projection；
- 单独复制的 RuntimeProfile projection；
- 未使用的 preset audit event；
- 没有消费者的 capability pack 子表；
- 同时保存 content JSON 和包含同一 content 的 envelope JSON；
- `message_projection` 中复制的完整 Event 数组。

未来正式升级使用 data generation、migration lineage/checksum 和 schema compatibility；
不能要求应用 build、文档 digest 或无关全局 inventory 完全相同才允许打开用户数据。

## 18. 产品不变量

1. AgentPreset Revision 只直接选择 Capability 与 Skill，并绑定 typed resources；
2. Package、ServiceKey、裸 MCP Tool 和 template key 不进入已保存 Snapshot selection；
3. Skill 不自动扩张 Capability；
4. MCP Tool 未完成 canonical mapping 时不能进入 Agent；
5. 一个 canonical Compiler 同时服务 Preview、Save 和 Test；
6. Session Open 读取已保存 Snapshot，不重新编译另一份结果；
7. Snapshot 只冻结实际选择闭包和 exact Provider，不冻结无关全局目录；
8. Snapshot 外调用统一失败，运行中的 Agent 不能修改自己的 ceiling；
9. typed resource binding 与领域 ownership 在真实调用时再次校验；
10. Browser/Computer 第一方与 alternate Provider 走同一 materializer、index 和 dispatcher；
11. Provider 缺失时明确失败，不 fallback；
12. AgentSession 是唯一产品与执行 aggregate；
13. Projection 只保存最终消息和 UI 摘要，不复制 Event Log；
14. checkpoint 是可丢弃 cache，不是第二事实源；
15. 删除使用真实 dispose/cleanup 结果，不使用调用者填零证明；
16. 本地写入使用领域已有事务或原子操作，不进入通用 Effect 状态机；
17. 外部结果未知时不自动重试，由 owning domain reconcile；
18. Candidate Capability 不默认注册、不 seed、不阻塞一期；
19. Catalog 数量、模板数量和 action owner 覆盖率不是一期完成标准；
20. 发布与平台 Evidence 不进入 Preset、Snapshot、Session、API 或产品 UI。

## 19. 最小产品验收

本文要求行为测试围绕真实闭环，而不是围绕固定数量或源码字符串：

| 场景 | 必须证明 |
|---|---|
| Minimal Chat | 空 Capability/Skill/MCP/Workspace 正向编译，最终 `tools=[]`，正常 turn/stream/cancel |
| Coding | 当前正式核心 Coding surface 真实可用，不以 mock 或弱化 fallback 补齐 |
| Package | first-party 与 test alternate 使用同一 registration/materializer |
| Compiler | Preview、Save、Test 对同一输入得到同一 Snapshot content |
| Resource | owner mismatch、missing binding 和 operation mismatch 明确失败 |
| On-demand | 只能激活 frozen ceiling，资源按首次真实使用 lazy acquire |
| Browser/Computer | first-party 与 alternate Provider 可替换，消费者代码不变，无具体实现旁路 |
| Projection | 删除 Projection 后可从 Session facts/Event 重建最终消息与 UI 摘要 |
| Effect | managed write 得到最终结果；external unknown 不自动重试 |
| Remote | open/turn/observe/cancel 复用同一 AgentSession、cursor 和 resource authority |
| Delete | cooperative dispose、超时 kill process tree、幂等清理和 minimal tombstone |

一期核心完成以真实用户闭环为准，包括 Chat、Coding、Workspace/File/Process/VCS、至少一个
MCP Tool、Browser、平台可用时的 Computer、Knowledge search/read、AgentSession
create/continue/cancel/delete、一个 scheduled/automation Session、Remote
open/turn/observe/cancel，以及代表性的 Runtime crash/process-tree cleanup。

Wave 3/4 候选、所有高级 Knowledge/Memory 组合、每种 Channel/Robot/Creative 场景、固定
模板全集和未声明交付平台的 native evidence，不属于本文的一期阻断条件。

平台支持以实际发布声明、Provider availability 和目标原生验证为准；本文不维护固定平台
笛卡尔积或发布 Evidence 编排。具体收口与发布策略由 05 和当前发布计划管理。
