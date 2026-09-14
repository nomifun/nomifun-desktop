# Agent 全栈插件平台评估：当前基线与开放式组装架构

日期：2026-09-13\
分支：`rf/agent-capability-platform-v2`\
基线提交：`08caa20d7b278423b09a56d126e465378eff6403`\
状态：用户已接受架构方向；已补充分期计划、暂不新增 Rust 插件情景及原始需求覆盖核对；本次文档任务未实施或验收所列新增运行能力

本次修订目标：**用户可替换任意 Agent 主机部件的全栈插件平台**。这里的替换是用户选择不同实现，不是抢占系统 ID；“任意部件”包括 UI、Prompt、模型、Agent Loop、资源、存储和宿主服务，但不同部件适合在不同生命周期边界替换。

快速阅读：

| 本次问题 | 判断与详细章节 |
|---|---|
| 1. 多个同类 capability 由用户选择 | 认可；独立实现 ID + 共同契约 + 显式绑定，见 §14 |
| 2. 替换系统 UI | 可实施；从页面/Agent UI 到完整 Shell，见 §15 |
| 3. 全环节开放及 Rust/JS 方案 | 大部分可开放；逐环节矩阵见 §16，语言与隔离方案见 §17 |
| 4. Skill 装载未闭环的影响 | ID 已投影，精确制品装载与执行语义未闭环，见 §18 |
| 5. 简单、稳定、开放的组装模型 | 复用 Role/Provider，统一编译结果与执行入口，见 §19 |
| 6. Rust 是否另建一套 runtime、如何安排开发 | 层级与复用见 §22；版本范围见 §23；工作包见 §24；预算/排期见 §25；启动与验收见 §26 |
| 7. 暂不增加 Rust 插件后，是否仍解决原始五项需求 | 首期只部分覆盖；逐项对应、29 个环节的工作包/验收、JS-only 工期及不可承诺的边界见 §27 |

§1～10、§12～13 描述原始基线；§11 总结架构判断；§14～21 为目标设计与研究证据；§22～26 将已接受的方向细化为开发计划；§27 核对原始需求并补齐遗漏的任务范围。暂不新增 Rust 插件的安排与覆盖判断以 §27 为准；§23～26 保留含 native 的对照计划。§20 的 A～G 只是主题路线，不是另一套需要重复执行的工作包。后文“建议/应当”不代表当前已有对应产品能力。本次仅更新报告，不修改运行代码，也不把工作区并行代码改动当作已经验收。

附录 A 保留工作区另行补充的统一实施基线，不代表本次研究已执行其中的代码、协议或数据库变更。§20 的数据迁移建议适用于需要保留真实用户数据的情况；开发数据重建等实施选择须按具体任务确认的范围执行。

## 1. 阅读范围与判断口径

本报告先回答两个现状问题，再研究上述五项目标架构问题：

1. 用户开发的插件目前可以替换系统什么级别、什么范围的能力；
2. 用户插件目前可以注入 Agent 的哪些环节。

报告区分以下四种状态：

- **已接通**：当前 NomiCore 产品链路已经能够完成安装/发布、组装、运行时调用；
- **部分接通**：部分链路已存在，但仍有运行时限制或缺少完整桥接；
- **合同/SPI 已存在**：代码和协议已经定义，但不能据此推断普通用户插件当前可用；
- **未开放**：由宿主、控制面或内核保留，用户插件当前不能替换。

本报告以当前产品实际启动的 **NomiCore** 为准。Fresh-v4 的 `AgentPlatform` 属于独立的未来迁移路径，不能直接当作当前用户插件能力。

## 2. 总体结论

当前重构已经兑现：

> **能力级插件化 + Agent Preset 显式组装 + Session 级精确锁定。**

最完整的用户扩展路径是：

- N1 JS Plugin 的 `FunctionTool`；
- M1 发布型插件的无资源 `Service Tool`。

因此，用户现在可以：

- 发布新的业务 capability；
- 实现具体的 Tool/action；
- 在 Agent Preset 中显式选择这些能力；
- 把能力放入初始 Tool 集或 on-demand Tool 集；
- 让 Agent 通过 Kernel 调用 JS Host 或 MiniApp Service Host；
- 以新的 capability 实现承担某个业务动作。

但当前还没有兑现：

- 整个 Agent Loop 的替换；
- 模型路由和决策循环的替换；
- Session authority、Snapshot authority 的替换；
- Kernel Registry 或 runtime supervisor 的替换；
- 面向普通用户插件的通用 TurnMiddleware、Event、Transport、Scheduler、BackgroundService；
- NomiFun Shell、系统主界面或 Agent UI 的替换。

这里的“替换”是**显式选择的能力替代**，不是透明覆盖：

```text
用户插件提供新的 capability ID
        ↓
用户在 Agent Preset 中显式选择
        ↓
该 capability 承担指定业务动作
```

用户插件不能直接覆盖系统内置 capability ID，也不能无配置地拦截所有同类调用。**这不是“系统能力不可替换”的目标约束。** 用户应当可以发布自己的 capability，在同一能力契约下与内置实现并列，并选择自己的实现作为默认或某个 Agent 的实现；当前通用替换链路尚未完成，见 §14。

## 3. 当前实际产品主机

当前产品入口明确构造 `NomiCoreApplication`：

- [`nomifun-app/src/lib.rs:60`](../../crates/backend/nomifun-app/src/lib.rs#L60)
- [`apps/web/src/main.rs:287`](../../apps/web/src/main.rs#L287)
- [`nomifun-app/src/desktop.rs:49`](../../crates/backend/nomifun-app/src/desktop.rs#L49)

Fresh-v4 的 `AgentPlatform` 目前是另一条主机路径。它发布的是内置 Rust registration，没有接入当前 NomiCore 使用的用户 JS Plugin `/api/plugins` 主链路。因此，Fresh-v4 中存在的通用组装 SPI，不能直接等同于当前产品已经开放给用户的插件能力。

## 4. 用户插件可以替换的级别与范围

| 替换级别 | 当前状态 | 用户实际可以做什么 | 当前不能做什么 |
|---|---|---|---|
| 业务动作 / Tool | **已接通** | 发布新的 `FunctionTool`，实现查询、自动化、外部 API、设备或领域操作等业务能力 | 不能直接覆盖内置 capability ID |
| M1 Service action | **已接通** | 发布 Active Service，通过独立 Service Host 执行无资源 FunctionTool | 带非空 `resource_kinds` 的 Service action 当前运行时会拒绝 |
| Agent 工具组合 | **已接通** | 作为 initial 或 on-demand capability 进入 Preset | 安装插件不会自动修改已有 Agent |
| Agent Session 资源绑定 | **已接通，但宿主控制** | 使用宿主已经支持并绑定的 typed resource | 不能伪造 owner、resource ID 或 operation grant |
| Context / system prompt | **用户插件未开放** | 宿主批准的 bundled builtin 可以贡献上下文 | 普通用户 JS Plugin 不能直接写入 system prompt |
| ResourceProvider | **SPI 有，用户注入未闭环** | Kernel 有 acquire/release primitive | 当前没有普通用户 ResourceProvider 的 Agent admission |
| Skill | **部分接通** | package、catalog、compiler lock 已存在 | NomiCore 当前 Skill runtime 尚未完整接入 N1 Plugin Skill |
| MCP mapping | **部分接通** | mapping 可 materialize 并生成 lock | 当前 Nomi runtime 主要依赖显式 MCP Server binding |
| Turn Middleware / Event / Lifecycle | **内置能力已接通** | bundled、宿主批准的 Rust capability 可以参与 | 普通用户 JS Plugin 不能直接注入 |
| Agent Loop / 模型路由 | **未开放** | 无 | 不能替换 |
| Kernel / 控制面 / Snapshot authority | **未开放** | 无 | 不能替换 |
| 系统 Shell / 主 UI | **未开放** | M1 可以提供自己的插件 UI | 不能替换 NomiFun Shell 或 Agent 主界面 |

## 5. 已接通的 N1 JS Plugin Tool 路径

NomiCore 已经打通以下链路：

```text
插件安装、挂载、恢复
        ↓
JsKernelPluginAdapter
        ↓
动态 PluginRegistration
        ↓
Kernel Registry replace_all
        ↓
Catalog materialization
        ↓
Agent Preset Preview / Save
        ↓
精确 Snapshot
        ↓
Nomi Session Tool materialization
        ↓
Kernel → JS Host → 用户插件 JS action
```

相关实现：

- [`nomifun-app/src/router/state.rs:785`](../../crates/backend/nomifun-app/src/router/state.rs#L785)
- [`nomifun-app/src/router/state.rs:917`](../../crates/backend/nomifun-app/src/router/state.rs#L917)
- [`nomifun-app/src/router/plugin_platform.rs:478`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L478)
- [`nomifun-app/src/router/plugin_platform.rs:2152`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L2152)
- [`nomifun-js-kernel-adapter/src/lib.rs:263`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs#L263)

用户 Tool 只有在以下条件满足时才会成为 Agent 可用能力：

- capability kind 是 `Tool`；
- 至少存在一个 `FunctionTool` action；
- JS runtime 可用；
- 插件处于有效的 materialized mount；
- capability 通过当前宿主的可用性检查。

动态可用性判断位于：

- [`nomifun-app/src/router/plugin_platform.rs:2250`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L2250)

Tool 的 materialization 和调用位于：

- [`nomifun-ai-agent/src/plugin_tools.rs:1830`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L1830)
- [`nomifun-ai-agent/src/plugin_tools.rs:2980`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2980)
- [`nomifun-ai-agent/src/plugin_tools.rs:3337`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L3337)

## 6. M1 发布型插件的 Agent 路径

N1 安装聚合与 M1 发布聚合已经统一产品入口，但底层仍然保留不同的数据根和机器合同。M1 是用户开发插件可以使用的另一种运行形态，不应与 N1 JS package capability 简单混为同一个注册路径。

参考：

- [`docs/reviews/2026-09-13-plugin-unification-implementation.zh.md`](./2026-09-13-plugin-unification-implementation.zh.md)

M1 Agent 路径为：

```text
Active Release
        ↓
MiniApp Catalog publication
        ↓
Agent Preset 选择
        ↓
ResolvedMiniAppCapability
        ↓
materialize_miniapp_actions
        ↓
同一个 Nomi Tool session
        ↓
MiniApp Service Host
```

M1 action 只有在以下条件满足时才可执行：

- capability 支持 `Agent`；
- capability 支持 `MiniAppService`；
- MiniApp 已 enabled；
- 存在 Active Service；
- action 在 allowlist 中；
- action 是 `FunctionTool`；
- release、catalog digest、epoch 均未过期。

当前 M1 的主要运行时边界是：

> 零资源 Service Tool 已接通；带 typed resource binding 的 Service action 仍属于合同/目录层能力，当前调用会被拒绝。

拒绝逻辑位于：

- [`nomifun-plugin-platform/src/runtime/m1_application.rs:530`](../../crates/backend/nomifun-plugin-platform/src/runtime/m1_application.rs#L530)
- [`nomifun-plugin-platform/src/runtime/m1_application.rs:598`](../../crates/backend/nomifun-plugin-platform/src/runtime/m1_application.rs#L598)

## 7. Agent 的实际注入点

### 7.1 Catalog 与能力发现：已接通

插件可以进入：

- Catalog；
- Agent capability 列表；
- capability action 列表；
- required resource kind；
- dependency/conflict 声明；
- artifact/schema digest。

### 7.2 Agent Preset 组装：已接通

插件可以被显式放入：

- initial capability；
- on-demand capability；
- action allowlist；
- capability dependency；
- capability conflict；
- exact contribution lock。

相关编译逻辑：

- [`agent-control-plane/src/compiler.rs:424`](../../crates/backend/nomifun-agent-control-plane/src/compiler.rs#L424)
- [`agent-control-plane/src/compiler.rs:680`](../../crates/backend/nomifun-agent-control-plane/src/compiler.rs#L680)
- [`agent-control-plane/src/compiler.rs:1413`](../../crates/backend/nomifun-agent-control-plane/src/compiler.rs#L1413)

注意：插件安装不会自动修改现有 Agent Preset。用户需要重新 Preview、Save 和 Compile。

### 7.3 Session binding：已接通，但由宿主掌握权限

保存的 Snapshot 会在 Session 创建时重新编译和校验，并附加目标资源的 typed binding：

- [`nomi_core_session.rs:1020`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L1020)
- [`agent-kernel/src/compiler.rs:127`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L127)

插件不能自行伪造：

- resource owner；
- resource ID；
- operation grant；
- Snapshot authority；
- capability provenance。

### 7.4 初始 Tool 集：用户 Tool 已接通

用户 Tool 可以被编译进 initial capability，并直接进入 Nomi Registry，供模型在当前 Session 中调用。

### 7.5 延迟 Tool 集 / ToolSearch：用户 Tool 已接通

on-demand 用户 Tool 会先作为 deferred Tool 存在，经过 ToolSearch 激活后，在 turn boundary 进入可调用集合。

实现位于：

- [`plugin_tools.rs:2980`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2980)
- [`plugin_tools.rs:3337`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L3337)

### 7.6 System prompt Context：只有 bundled builtin 已接通

Kernel 层存在 `ContextContributionFactory` 和 JS context proxy，但 NomiCore 的 Agent admission 只接纳：

- `ContributionSourceKind::PlatformBuiltin`；
- `PluginSourceKind::Bundled`；
- 宿主预先批准的 platform builtin。

因此：

> 普通用户 JS Plugin 的 `ContextContributor` 当前不能直接注入 Nomi Agent 的 system prompt，也不能作为普通用户插件通过 Agent Preset 使用。

实现和限制位于：

- [`plugin_tools.rs:2120`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2120)
- [`plugin_tools.rs:2231`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2231)
- [`plugin_platform.rs:2274`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L2274)

### 7.7 Turn Middleware / Event / Lifecycle：可信 Rust builtin 已接通

宿主批准的 bundled Rust capability 可以参与：

- turn 前上下文；
- deferred lifecycle Tool；
- middleware turn context；
- lifecycle activation；
- EventSource、Transport、ResourceProvider、BackgroundService 等宿主生命周期。

但当前 admission 明确要求 bundled、PlatformBuiltin 和 typed host binding：

- [`plugin_tools.rs:330`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L330)
- [`plugin_tools.rs:2380`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2380)

这不是普通用户 JS Plugin 的开放注入点。

## 8. N1 合同支持但当前未完整开放的能力

N1 Plugin Package v1 的合同允许声明：

- `Tool`；
- `ContextContributor`；
- `ResourceProvider`；
- `Skill`；
- MCP tool mapping。

同时明确禁止：

- Role Contracts；
- Role Providers。

JS v1 也不能直接声明 Browser/Computer Role Provider、通用 EventSource/EventConsumer、TurnMiddleware、Transport、Scheduler、BackgroundService、Agent loop 或 Runtime authority。

合同定义位于：

- [`plugin_n1.rs:3069`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs#L3069)
- [`plugin_n1.rs:3119`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs#L3119)
- [`plugin_n1.rs:3145`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs#L3145)

### 8.1 ResourceProvider

Kernel 已有完整的 acquire/release primitive：

- [`registry.rs:749`](../../crates/backend/nomifun-agent-kernel/src/registry.rs#L749)
- [`registry.rs:1000`](../../crates/backend/nomifun-agent-kernel/src/registry.rs#L1000)
- [`js-kernel-adapter/src/lib.rs:623`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs#L623)

但当前 NomiCore 没有发现把用户 ResourceProvider 自动装配成 Agent 可选择的独立 provider 的完整 admission/acquire 路径。

用户 Tool 可以声明 `resource_kinds`，但资源种类仍由 NomiCore 固定解析器控制，例如：

- `workspace`；
- `knowledge_base`；
- `mcp_server`；
- `robot`；
- `miniapp`。

相关代码：

- [`nomi_core_resource_bindings.rs:126`](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs#L126)
- [`nomi_core_resource_bindings.rs:295`](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs#L295)

### 8.2 Skill

Kernel 已经支持：

- Skill materialization；
- `requires_capabilities` 校验；
- `ResolvedSkillLock`；
- body digest 和 provenance 锁定。

相关编译逻辑：

- [`agent-kernel/src/compiler.rs:879`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L879)
- [`agent-kernel/src/compiler.rs:1413`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L1413)

但当前 NomiCore 的传统 Skill runtime 仍主要使用 `SkillPaths`、workspace 和 `SKILL.md`：

- [`skill_resolver.rs:41`](../../crates/backend/nomifun-conversation/src/skill_resolver.rs#L41)
- [`service.rs:4398`](../../crates/backend/nomifun-conversation/src/service.rs#L4398)

本次进一步复核发现：`nomi_core_agent_projection::project_internal` 已经把
`snapshot.content.skill_locks` 中的 Skill ID 投影到 `included_skills`，随后
Conversation 将其转换为 `agent_enabled_skills` 和 `extra.skills`。因此不是完全没有接线，
而是**精确 Skill 锁在运行时投影中降为名称列表**，尚未看到按该锁装载插件制品正文、
引用资源并交给 Nomi 执行的完整消费路径。详见 §18 及其中新增证据。

因此，当前只能确认：

> Skill 的 package/catalog/compiler 合同已经具备，但 N1 Plugin Skill 到当前 Nomi runtime 的完整装载和注入闭环尚未形成。

### 8.3 MCP

N1 Plugin 可以声明 MCP mapping，Kernel 可以：

- 校验 mapping 与 capability 一致；
- materialize 到 catalog；
- 编译 `ResolvedMcpToolLock`。

但当前 Nomi runtime 的 MCP 使用路径仍然主要是：

- Session 显式绑定 MCP Server；
- `mcp.connect`；
- `mcp.tool_proxy`；
- `mcp.resource`；
- native MCP manager/tool registration。

相关代码：

- [`materialize.rs:1000`](../../crates/backend/nomifun-agent-kernel/src/materialize.rs#L1000)
- [`agent-kernel/src/compiler.rs:1450`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L1450)
- [`nomi_core_agent_projection.rs:467`](../../crates/backend/nomifun-app/src/router/nomi_core_agent_projection.rs#L467)
- [`nomi_core_session.rs:619`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L619)

目前不能宣称“N1 用户插件 MCP mapping 已自动成为当前 Nomi Agent 的可调用工具”。

## 9. 当前完整调用链

```text
N1 插件安装 / M1 插件发布
        ↓
Catalog materialization
        ↓
Agent Preset initial / on-demand 选择
        ↓
依赖、冲突、资源、allowlist 校验
        ↓
immutable Snapshot
        ↓
artifact / schema / provenance / digest lock
        ↓
Session typed resource binding
        ↓
Nomi Tool / deferred Tool / builtin Context materialization
        ↓
ToolSearch 或生命周期激活
        ↓
Kernel authority check
        ↓
JS Host 或 MiniApp Service Host 执行
```

## 10. 版本、升级与替换规则

当前模型不是运行时热插拔覆盖，而是编译后锁定：

- 插件安装不会自动改变已有 Agent；
- Agent 必须重新 Preview/Save/Compile 才会使用新插件；
- Snapshot 会锁定 capability provenance、artifact digest、schema digest；
- 已创建的 Session 不会因为插件升级而透明漂移；
- M1 还会校验 active release、release digest、catalog digest 和 epoch；
- 插件升级后，旧 Snapshot 与新版本之间需要重新通过编译和 admission 校验。

因此，当前更接近：

> **不可变 Agent 配置 + 可验证 capability 版本锁**

而不是：

> **插件覆盖正在运行的 Agent 主机实现**

## 11. 本次问题的决策建议

原版把若干方向列为“是否开放”。按本次全栈平台目标，建议不再以封闭为默认：

1. **开放同类能力替换**：保留独立 capability ID，用共同契约选择 Provider；不建设 ID override 系统。
2. **开放 UI**：插件既能贡献区域，也能替换 Agent 页面和工作台 Shell；底层使用统一命令/查询/事件接口。
3. **开放 Context、Resource、Skill、MCP、Middleware、Event、Transport、Scheduler、Service**：按接口和授权准入，逐步取消以 bundled 来源作为功能资格的硬编码。
4. **开放模型、上下文管线与整个 Agent Runtime**：既能替换 Nomi 内部明确的策略组件，也能在同一会话协议后使用另一套完整引擎。
5. **可选支持 Rust 作者**：若投入原生插件，优先支持共用协议的 Rust 进程外插件；Wasm 作为有实测收益时增加的执行后端；不把原生 Rust 动态库作为公共插件 ABI。暂不新增 Rust 插件不阻塞其他开放工作，见 §27。
6. **尽快闭合 Skill**：保留文件来源，但统一运行时装载器，不能把“已锁定 ID”当成“已装载锁定内容”。
7. **缩短组装链**：一个能力目录、一套 Provider 绑定解析、一份冻结执行计划；Preview/Save/Open 不分别维护不同的解析规则。
8. **区分替换实现与伪造权限**：资源、存储、认证和宿主服务的实现可以被部署者替换；当前运行中的普通插件不能自行替换负责约束自己的信任根。全量宿主替换通过显式启动配置/部署完成，不要求热替换。

以上架构方向已获用户接受；本轮进一步制定实施计划，不代表重构已经完成，也不在本次文档任务中自动开始代码实施。

## 12. 一句话基线

当前系统已经是一个可以把用户开发的业务能力显式装配进 Agent 的插件平台；最成熟的是 **N1 JS Tool 和 M1 无资源 Service Tool**。它还不是一个允许用户插件替换 Agent 主机、运行时内核、Prompt 管线和控制面的全栈 Agent 平台。

## 13. MiniApp 与 Plugin 合并完整性复核（2026-09-13）

### 13.1 修正后的结论

上一期工作**没有把 MiniApp 从代码和领域模型中彻底移除**。已经完成的是：

- 产品入口统一到 `/plugins`；
- 公共 HTTP 路径统一到 `/api/plugins`；
- 创建、导入、工作区和运行入口统一到 Plugin 语义；
- `nomifun-miniapp-platform`、`nomifun-plugin-service` 的实现物理合并到 `nomifun-plugin-platform`；
- 对外 DTO 和顶层 CLI 基本改用 `plugin` / `plugin_id` / `plugins`。

但尚未完成的是领域层和运行时层的统一。更准确的表述是：

> **已完成产品入口统一、公开 API 统一和 crate 合并；未完成身份模型、Agent Snapshot、Catalog provenance、数据库聚合、运行时角色和内部命名的统一。**

因此，不能把本次结果描述成“系统中已经不存在 MiniApp 概念”，只能描述成“MiniApp 已经被纳入 Plugin 产品入口，但底层仍有独立的 MiniApp 运行分支”。

### 13.2 仍在生效的结构性残留

以下不是单纯的注释或历史字符串，而是会影响当前编译、快照、准入或运行时行为的活动代码：

1. **Agent Snapshot 仍有两套 capability 集合。**\
   `ResolvedMiniAppCapability` 与 `ResolvedCapability` 并列存在，Snapshot 仍有
   `initial_miniapp_capabilities` 和 `on_demand_miniapp_capabilities`。证据：
   [`preset.rs:470`](../../crates/backend/nomifun-agent-contracts/src/preset.rs#L470)、
   [`preset.rs:646`](../../crates/backend/nomifun-agent-contracts/src/preset.rs#L646)。

2. **Session 仍使用独立的 MiniApp materialization 和 invoker。**\
   当前会分别调用 `materialize_miniapp_actions`、`with_miniapp_actions`，
   并构造 `NomiCoreMiniAppToolInvoker`，而不是将 M1 作为统一 Plugin capability
   的 runtime role/profile。证据：
   [`nomi_core_session.rs:563`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L563)、
   [`nomi_core_session.rs:591`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L591)。

3. **Catalog 和 provenance 仍把 MiniApp 当作一等来源。**\
   `ContributionSourceKind::MiniAppActiveRelease`、MiniApp catalog publication
   和对应生命周期分支仍然存在。这意味着 Agent 能力来源仍不是“Plugin + role/profile”，
   而是包含一个独立的 MiniApp source kind。

4. **数据库仍以 MiniApp 为运行时数据根。**\
   当前运行时仍直接使用 `miniapp_products`、`miniapp_projects`、
   `miniapp_releases`、`miniapp_catalog_publications` 等表，并由
   `SqliteMiniAppM1Repository` 访问。历史 migration 文件可以保留，但活动 repository、
   identity 和外键仍是 MiniApp 领域模型，不能视为已完成统一。证据：
   [`072_miniapp_m1_data_root.sql:8`](../../crates/backend/nomifun-db/migrations/072_miniapp_m1_data_root.sql#L8)、
   基线文件 `crates/backend/nomifun-db/src/repository/sqlite_miniapp_m1.rs:108`。
   本轮排期补充时，该文件已在并行工作区中改名为
   [`sqlite_plugin_runtime.rs`](../../crates/backend/nomifun-db/src/repository/sqlite_plugin_runtime.rs)；
   此处保留原基线判断，不把文件改名本身作为领域统一已完成的证据。

5. **`plugin-platform` 主要是承载旧实现，领域类型尚未重命名。**\
   `miniapp_m1.rs`、`MiniAppReleaseV1Manifest`、`MiniAppBridgeSession`、
   `MiniAppServiceStorageDescriptor` 等仍是当前 runtime 的核心类型。也就是说，
   这是“旧 crate 合并 + 外部入口改名”，不是“领域对象统一”。

6. **后端内部路由和状态仍误导为 MiniApp。**\
   即使公开路径已经是 `/api/plugins/runtimes`，内部仍有
   `miniapp_m1_read_routes`、`miniapp_m1_write_routes`、`states.miniapp` 等名称。
   证据：
   [`routes.rs:852`](../../crates/backend/nomifun-app/src/router/routes.rs#L852)、
   [`plugin_runtime.rs:55`](../../crates/backend/nomifun-app/src/router/plugin_runtime.rs#L55)。

7. **UI 类型系统和 Agent 资源选择器仍暴露 MiniApp。**\
   `PluginRuntimeId` 仍定义为 `EntityId<'miniapp'>`，Agent 资源种类仍有
   `miniapp`，并且资源选择逻辑仍按该 kind 分支。证据：
   [`ids.ts:82`](../../ui/src/common/types/ids.ts#L82)、
   [`ids.ts:143`](../../ui/src/common/types/ids.ts#L143)、
   [`AgentResourcePicker.tsx:108`](../../ui/src/renderer/components/agent/AgentResourcePicker.tsx#L108)。

8. **Wave 3 内置 Agent 能力仍以 MiniApp 命名并注册。**\
   `miniapp.read`、`miniapp.edit`、`miniapp.publish`、`miniapp.serve`
   仍是独立 capability ID 和 host 分支。这会继续把 MiniApp 暴露为 Agent 的能力类别，
   而不是 Plugin 的一种 role/profile。

### 13.3 仍会造成误导的文档和脚本

当前仍有文档或验收材料描述已退出的产品入口或 crate，例如：

- `docs/architecture/backend-crates*.md` 仍引用 `nomifun-miniapp`；
- `docs/architecture/frontend*.md` 仍写 `/mini-apps`；
- `docs/reference/api-overview*.md` 仍写 `/api/miniapps`；
- `README.md`、`CHANGELOG.md` 仍把 Agent Mini Apps 当作独立产品；
- `docs/guides/presets.zh.md` 仍把 Plugin 和 MiniApp 写成两种能力来源；
- Windows 验收脚本仍存在以 MiniApp 为独立产品的命名和 scope；
- 生成的 preset/fixture 仍包含 `miniapp.*` capability ID。

这些内容会让开发者、测试人员和后续 Agent 得出错误结论：以为 MiniApp 仍是
当前公开产品，或者以为 Plugin 与 MiniApp 是两套并列扩展机制。

### 13.4 不应机械删除的残留

以下命中需要建立 allowlist，不应直接全局替换：

- 已发布的 bridge handshake / MessageChannel 事件名，例如
  `nomifun-miniapp-bridge-*-v1`；这是版本化传输协议；
- 历史 SQL migration、历史 spec、历史 changelog 中的原始名称；
- 测试 fixture 中仅用于模拟旧文件的 `miniapp.html`；
- 与 MiniApp 无关的 `MiniMap` 等普通词。

但协议兼容并不能解释 `ResolvedMiniAppCapability`、Agent resource kind、
数据库 repository、`miniapp.*` capability ID 这些当前活动领域对象；这些仍需要迁移。

### 13.5 当前真实架构形态

当前 Agent 能力来源仍近似如下：

```text
普通 Plugin capability
MiniApp Active Release capability
MCP capability
Builtin capability
```

如果上一期需求的验收标准是“用户只面对 Plugin，Agent 内部也只使用统一
Plugin capability，并通过 Service / Surface / Tool 等角色表达差异”，那么当前结果
尚未达标。建议下一阶段先定义 canonical 对象（例如 `PluginProduct`、
`PluginRelease`、`PluginService`、`PluginSurface`），再依次合并 Snapshot、
provenance、resource kind、数据库 repository、runtime role 和内部命名。

## 14. 问题一：同类 capability 的并存、选择与替换

### 14.1 结论：认同目标，但要区分“功能相似”与“契约可替换”

用户插件应能发布一个或多个 capability，进入与内置能力相同的目录，供 Agent 和其他消费者选择。插件是分发单位，capability 是消费单位；一个插件不必被限制成一个 capability。

需要同时支持两种用法：

1. **作为独立工具使用**：两个搜索插件可以都进入 Agent 工具集，由模型根据描述选择；这是当前 Tool 路径最接近的能力。
2. **作为某个部件的替代实现**：用户把默认搜索、上下文构造器、模型路由器或 Agent Runtime 指向自己的插件；系统对该部件的调用都通过选定实现。这需要稳定契约和绑定，不是仅把两个工具都展示给模型。

`CapabilityKind::Tool` 相同、展示名称相似、甚至输入 JSON Schema 相同，都不足以证明可替换。契约还应定义输出、错误、流式/取消行为和必要语义。例如“网页搜索”和“本地向量检索”都能返回文本，但不一定符合相同的搜索契约。

### 14.2 复用现有 Role/Provider，不新增平行替换框架

仓库已经有 `RoleContractManifest`、`RoleProviderContribution`、`InstallationRoleBinding`，Preset 也已有 `system_role_provider_overrides`。这些是最接近目标的基础，不需要再叠加一套独立的 Slot Registry、Override Registry 或 Provider Marketplace：

- [`package.rs:65`](../../crates/backend/nomifun-agent-contracts/src/package.rs#L65)：Role 契约、Provider 引用、安装级绑定。
- [`preset.rs:186`](../../crates/backend/nomifun-agent-contracts/src/preset.rs#L186)：Preset payload 中的 Provider override。
- [`plugin.rs:183`](../../crates/backend/nomifun-agent-kernel/src/plugin.rs#L183)：已解析的 Role member 与 Provider mount 上下文。
- [`plugin_n1.rs:3069`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs#L3069)：N1 v1 当前明确拒绝用户发布 Role/Provider，需要版本化修改此合同及执行适配器。

建议约定：**Role 是一个可替换部件的契约，Provider 是插件对该契约的实现绑定；capability 仍是统一的可发现、可选择贡献。** Role 可包含一个或多个相关 member，用来表达同一个有状态服务的完整接口，而不是每个方法都创建一个插件层。

现有 Role member 使用契约侧的 capability 引用。推广时，允许用户的独立 capability 显式导出对应 member；该映射只在 Provider 注册处声明一次。系统 member 身份保留，用户实现身份也保留，两者不能通过复制相同 ID 混为一体。现有合同若不足以表达此映射，应修订该合同，而非外挂第二套解析器。

示意（下列 ID 仅为设计示例，不是当前已有接口）：

```text
契约 search.web/v1
    ├─ 内置实现：platform.search.web
    └─ 用户实现：acme.search.web

用户选择 acme.search.web
    → 编译 Role/Provider 绑定并锁定具体制品
    → Agent 的该契约调用落到 acme.search.web
```

独立的新能力可以先只发布 capability，不要求开发者先制定通用 Role。只有被系统或其他插件按契约依赖、需要多实现替换时才引入 Role。用户也应可发布自己的命名空间契约，系统保留的只是系统契约命名空间，不是契约创作权。

### 14.3 选择规则应少且可预测

- 单实例部件按 **Agent 显式选择 > 安装级用户默认 > 官方默认** 解析；官方默认仅是创建时的默认值，不是运行失败后的暗中回退。
- 多实例贡献，如工具集合、Context 列表、观察者，按用户选择的集合及明确顺序组装；不依赖加载先后或全局数字优先级。
- 同一单实例契约选择多个实现时，要求用户选择一个；需要轮询/路由时，使用一个明确的路由 Provider，它再消费多个实现，而不是隐式竞争。
- Agent 可在已授权、已锁定的工具/Provider 候选池内动态选择；不能借“模型自动选择”在 turn 中安装插件、改主机或提升授权。
- 依赖尽量指向契约；确实依赖某个具体实现时才精确依赖 capability。否则“用户替换 A 后，B 仍硬编码调用内置 A”会使替换名存实亡。
- 选择新默认影响新编译/新会话。已有 Session 继续使用冻结绑定；状态迁移另行显式完成。更改 UI 外观不必重建 Agent Session。

### 14.4 本项验收不是“安装成功”

同一契约至少有一个内置和一个用户实现，用户选择后，真实请求必须只调用选定 Provider；测试同时覆盖默认继承、Agent override、契约不兼容、旧会话不漂移、卸载后可解释失败。调用方不能再认识内置实现的具体 ID。这样才从“插件增加工具”升级为“插件替换部件”。

## 15. 问题二：系统 UI 插件化的可实施性

### 15.1 结论与现有基础

**可实施，且不需要改用 Rust 编写 UI。** 当前前端本来就是 React/TypeScript；Rust 主机与 JS UI 通过版本化数据接口协作，没有天然的语言障碍。工作量主要在拆开 UI、业务状态和桌面特权，而不是渲染技术。

已有基础及其边界：

- [`package.rs:419`](../../crates/backend/nomifun-agent-contracts/src/package.rs#L419) 已定义 `UiContribution` 和 `CapabilityConsumer::Ui`，但枚举存在不代表产品已经支持 UI 替换。
- [`PluginRuntimeSurfacePanel.tsx:314`](../../ui/src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.tsx#L314) 有窗口/nonce 校验及 MessageChannel 握手；[`同文件:522`](../../ui/src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.tsx#L522) 用 `allow-scripts allow-forms` 沙箱 iframe 展示插件。
- 原始基线 `crates/backend/nomifun-agent-contracts/src/miniapp_m1.rs:1933` 的 Bridge target 是 Host KV 或 Service 调用，不是完整的 Session/导航/主题 SDK；工作区统一改造后的对应声明见 [`plugin_runtime.rs:1887`](../../crates/backend/nomifun-agent-contracts/src/plugin_runtime.rs#L1887)，仍是这两类 target。此处只核对声明，不代表统一改造或 UI 会话链已验收。
- [`ProtectedAppRuntime.tsx:18`](../../ui/src/renderer/components/layout/ProtectedAppRuntime.tsx#L18) 已把鉴权、深链、桌面同步等运行职责与可见布局分离；[`Router.tsx:13`](../../ui/src/renderer/components/layout/Router.tsx#L13) 的页面仍通过静态 import/route 组装。
- [`chatPort.ts:45`](../../ui/src/renderer/pages/creativeStudio/agent/chatPort.ts#L45) 已有不依赖 HTTP/IPC 的流式会话 port，是可参考的局部拆分，不应直接冒充通用插件 SDK。

### 15.2 UI 分层开放，不停留在主题和小挂件

| 范围 | 用户可以替换什么 | 最小改造与建议 |
|---|---|---|
| 主题与展示贡献 | 主题变量、图标、工具结果卡片、附件预览 | 主题配置可纯数据化；复杂渲染走 Surface，不开放任意宿主 DOM 修改 |
| 页面内区域 | 导航项、侧栏、Inspector、消息区域、输入区 | 为有独立产品意义的区域定义契约；多实例贡献与单实例替换分开 |
| 完整 Agent 页面 | 自定义聊天 UI、计划视图、Agent 组装界面 | Session command/query/event API；完整覆盖发送、流式输出、取消、历史和错误 |
| 工作台 Shell | 整体布局、导航、首页、页面组合 | 安装级选择一个 Shell Provider，内置 Shell 也实现同一接口 |
| 独立客户端 | 用户自己的 Web/桌面客户端 | 复用同一版本化应用 API；不要求导入 NomiFun React 内部组件 |
| 桌面容器/启动恢复 | 窗口、系统托盘、启动器等 | 服务实现可在部署/启动时配置；公共网页插件不直接获得 Tauri 或 OS 任意权限 |

建议第一条纵向切片选“完整 Agent 页面”，而不是先把每个按钮变成扩展点。它能最早验证核心 UI 可替换性，又不会制造过多细粒度接口。

### 15.3 一个 UI Host API，不复制业务后端

UI 插件发布 `UiContribution`，由 UI 消费者绑定到页面或 Shell 的 Role。它不是必须暴露给 LLM 的 Tool，也不必伪造 Agent Session 才能启动。后端、UI、Agent 是同一 Catalog 的不同消费者。

最小公共接口应覆盖：

- 应用导航、主题/语言、已授权的当前用户与工作区摘要；
- Session 创建、查询、发送、取消及带 cursor 的事件订阅；
- capability/Agent 配置读写的高层产品 API；
- 文件/剪贴板/窗口等特权操作的受限代理（按实际需求逐项加入）。

消息记录、活动 turn、权限及持久化真相仍在应用服务，UI 只保存界面状态。更换 UI 时重新查询状态和续订事件，不重新发起同一 turn。Session API 必须让插件 UI 能达到内置 UI 的功能完整性，不能只开放“发一句话，收一个字符串”。

跨边界使用结构化、版本化 DTO 和事件，不传 React component、闭包或 Zustand store。第三方可自由使用 React/Vue/Svelte 或原生 DOM；SDK 负责 transport 适配与类型生成，不规定业务框架。

### 15.4 渲染与稳定性选择

默认第三方 UI 建议沿用隔离 Surface，页面和完整 Shell 可以覆盖主内容区域。内置 UI 可以保留同进程渲染，但必须消费同一应用接口。不要把第三方 bundle 动态导入宿主 React 树当作沙箱：它会获得宿主页面环境，并耦合 React/CSS/内部状态版本。

iframe 的真实代价包括焦点/输入法、拖放、剪贴板、弹层、可访问性和主题同步；这些需要在桌面 WebView 与浏览器中验证。按页面/面板形成隔离边界，避免每个消息 token 或每个按钮创建 iframe。流式输出可批量更新，避免消息风暴。

保留一个插件之外的最小恢复入口：禁用故障 Shell、切回内置 UI、查看错误。它是恢复通道，不是偷偷切换正在执行的 Agent 实现。认证结果与授权真相不由 UI 自报；内容沙箱、Bridge 授权、服务端鉴权各自负责不同边界，不能只依靠 iframe。

### 15.5 实施成本判断

主题/区域贡献为中等改造，完整 Agent 页面为中到高，完整 Shell 为高；没有证据支持精确人日承诺。主要成本是提取公共 API、移除组件对全局业务 store 的直接依赖、补全事件恢复，以及跨平台交互验证。推荐“Agent 页面 → 页面/导航绑定 → Shell”顺序，持续保留内置 UI 的同契约实现。

## 16. 问题三：逐环节开放与组件化判断

### 16.1 判断原则

§7 的七个注入点只是当前已实现链路的分类，不是 Agent 架构的完整上限。本节把它们全部纳入，并扩展到模型、规划、压缩、记忆、执行、UI 和宿主服务。

开放应主要按**契约、所需资源、生命周期、运行隔离方式**判断，不按“Rust 能做、JS 不能做”判断。内置来源可以影响默认信任及执行位置，不应永久决定是否有资格参与某个环节。

表中“低/中/高”是相对改造成本，不是工期；现状来自静态代码复核，目标契约尚未实现。生命周期只有三类常用边界：turn（一次交互）、Session（一次会话）、安装/启动（共享主机）。不要为每个插件另造一套复杂状态机。

### 16.2 全环节开放矩阵

| 环节 | 当前普通用户插件状态 | 建议开放方式 | 组装/替换时机与主要注意点 | 改造 |
|---|---|---|---|---|
| Catalog / 发现（§7.1） | Tool 等已进入目录 | 所有贡献进入统一 Catalog；插件可提供搜索/排序实现 | 安装后发现；搜索排名不直接授予能力 | 中 |
| Preset / 依赖（§7.2） | 能选择 capability，有依赖和锁 | 契约依赖、Provider 选择；允许插件发布不附带授权的模板 | 创建/保存时解析；模板只是配置种子 | 中 |
| Session 创建/资源绑定（§7.3） | 宿主固定资源解析 | 开放资源解析 Provider、初始化贡献和目标选择 UI | 创建时绑定；owner 与授权不能由插件伪造 | 中高 |
| 初始 Tool（§7.4） | N1 和无资源 M1 已通 | 延续现有 Tool 契约，统一执行适配 | Session 工具集；具体调用检查当前授权 | 中 |
| on-demand / ToolSearch（§7.5） | 动态 Tool 已通 | 搜索、排序、激活策略可替换 | 只激活计划允许的能力，turn 边界生效 | 中 |
| Context / persona / system prompt（§7.6） | 普通插件准入未通 | 开放文本/结构化 Context；也允许选择完整 Prompt 管线 | Session 与每 turn；用户决定顺序和角色，标记来源与预算 | 中 |
| Skill（§8.2） | 元数据和锁有，精确运行消费未通 | 统一 Skill 装载器；包、文件目录只是来源适配器 | 初始索引/按需正文；正文与引用资源须对应锁定制品 | 中 |
| MCP（§8.3） | mapping 有锁，原生 Server 路径仍独立 | mapping、Server 连接由同一 capability/资源接口消费 | 启动或按需；避免同一工具双重注册、来源混淆 | 中 |
| ResourceProvider（§8.1） | JS proxy 有，产品准入未闭环 | 自定义 resource kind、列举/选择、acquire/release | Session 或操作级租约；保留真实资源 owner | 中高 |
| M1 Service 资源（§6） | 非空 resource_kinds 被拒绝 | 经统一资源代理传不透明 handle | 逐操作授权与撤销，不把宿主裸路径当权限 | 中 |
| Browser / Computer 等 Role | Rust 内置 Role 基础有，N1 禁止 | JS/Rust Provider 实现同一 Role，替换执行 owner | 通常 Session 固定；状态与平台权限不能自动转移 | 中高 |
| Turn Middleware（§7.7） | bundled 特定逻辑，非通用插件 hook | 显式 before/after 阶段，输入快照和结构化 patch | turn 边界；禁止共享任意可变 Engine 引用 | 中高 |
| EventSource / EventConsumer（§7.7） | bundled 生命周期路径 | 发布/订阅命名空间事件；声明所需主题 | 观察者异步，修改流程走明确命令；有界队列与取消 | 中 |
| Lifecycle / BackgroundService（§7.7） | bundled 路径 | start/stop/dispose 和健康状态，统一 supervisor 适配 | Session 或安装级；结束时释放资源，崩溃可定位 | 中高 |
| Transport / Channel | 宿主接线与内置能力 | 收消息、发消息、附件、身份映射 Provider | 安装级服务；通过 Session API，不直接操作会话库 | 中高 |
| Scheduler / Cron / Automation | 内置 owner，非通用用户扩展 | 触发器、调度策略、任务执行器可替换 | 安装级；沿用单一任务所有权与取消/重入语义 | 中高 |
| 模型 Provider / 模型路由 | 有 Rust provider 接口，无普通插件替换闭环 | 模型流式调用契约 + 可选路由策略 | Session 锁定策略和授权候选池；按请求路由可开放 | 高 |
| 上下文选择 / 历史 / 压缩 | Nomi Engine 内聚实现 | ContextPipeline、Compactor 等有意义的策略接口 | 每 turn/模型请求前；变换视图不篡改持久化事实 | 高 |
| 长期记忆 / RAG / embedding | 部分业务 capability，非完整管线替换 | 读写/检索/索引接口及资源 Provider | namespace 隔离；模型生成的记忆可追踪、可删除 | 中高 |
| Planner / 推理循环 / 停止策略 | Nomi 内部实现 | 明确策略组件；复杂算法可直接实现整个 Runtime | 一个 Session 一个 Runtime owner；预算与取消仍可执行 | 高 |
| 工具编排 / 并行 / 结果转换 | Nomi 执行链 | 调度策略、结果渲染/变换、重试建议接口 | 副作用工具不因传输超时默认重试 | 高 |
| 子 Agent / 多 Agent 协作 | 有内置委派能力，无通用插件装配 | 委派/协调 Provider 调用统一 Session 与能力接口 | 子任务继承或缩小授权；显式预算、取消传播 | 中高 |
| 整个 Agent Runtime / Loop | 生产 handle 为 Nomi 变体 | 会话命令/查询/事件/可选 checkpoint 契约 | 创建时选引擎；支持其他引擎，不强迫其使用 Nomi 内部策略 | 高 |
| 日志 / tracing / 评估 / 计量 | 宿主设施 | 观察者、导出器、评估器插件 | 默认异步、不阻塞 turn；敏感内容按授权提供 | 中 |
| UI / Agent 页面 / Shell | Surface 有，替换未通 | §15 的 UI Role 与应用 API | 视图可重载；独立于正在运行的 Session | 中高 |
| Session 存储 / 事件存储 / checkpoint 存储 | 宿主与引擎特定实现 | 按实际一致性需求提取存储 port，提供替代后端 | 安装/启动或维护窗口切换；数据迁移不能假装热替换 | 高 |
| Preset 编译策略 / Registry 后端 | 固定控制面与 Kernel | 开放选择策略、目录存储；最终计划校验器保持单一 | 编译时或启动配置；插件提出计划，统一校验后生效 | 中高 |
| 认证 / 授权策略 / Secret provider | 宿主权威 | 部署者可选认证、策略和密钥存储实现 | 安装/启动；普通插件不能自批权限或读取其他插件密钥 | 高 |
| supervisor / sandbox / Kernel 宿主实现 | 固定主机 | 执行后端与隔离 backend 接口可替换；整宿主支持重新部署 | 停机/受控重启；不能让被监管进程自己撤销监管 | 高 |

### 16.3 最小宿主不等于把所有重要功能永久封闭

推荐默认 Rust 宿主只保留：启动与恢复、契约与绑定的最终校验、调用授权、资源租约账本、执行生命周期监督，以及真实状态提交的仲裁。UI、Prompt、模型、循环、检索、调度等产品逻辑逐步退出这层。

其中存储、认证、Secret、隔离后端仍可通过部署级 Provider 替换；这些实现属于用户选择的信任根。不可同时保证“插件随时改写自己的授权/监督者”与“该监督者能约束插件”。这不是 Rust 或 JS 能解决的矛盾。

因此建议把目标分成：**产品部件通过公开插件契约替换；基础设施通过部署级接口替换；最小宿主仲裁实现通过独立宿主实现/发行版替换。** 后两者不能冒充“普通用户安装一个 JS 插件即可替换”，整宿主重编译也不是插件化完成证据。不承诺任意时刻无状态迁移地热换一切。如果原始目标严格要求普通插件接管包括信任根在内的所有部件，本方案并未完整满足这个字面目标；具体边界与原因见 §27.4。

### 16.4 Middleware 不应演变成任意 hook 拼装的流程迷宫

优先开放少量真实阶段：输入准备、上下文构建、模型调用前后、工具调用前后、turn 结束。观察事件和修改请求分开：观察者看快照；修改者返回明确 patch。对同一字段的多个修改按用户可见的管线顺序执行，不依赖并发竞态。

取消/超时/资源释放由公共执行边界统一处理；不为每种 middleware 新建 supervisor。组件不拿 `Engine` 可变引用，宿主也不应持有引擎大锁等待跨进程插件，以免回调重入造成死锁。

同时避免另一种极端：为了支持一个完全不同的 Agent Loop，要求它复刻几十个 Nomi hooks。替代引擎只需满足外层 Runtime 会话契约；内部阶段是否支持、支持哪些，应显式声明，由组装器提前提示不兼容。

### 16.5 自定义资源和授权的开放方式

当前 `NomiCoreResourceBindingResolverRegistry::product` 按固定 `SUPPORTED_RESOURCE_KINDS` 组装，且目前每种 kind 仅一个选择，见 [`nomi_core_resource_bindings.rs:120`](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs#L120)。这不是应该永久维持的产品限制。

建议资源 Provider 自带 schema、列举/选择能力、所需操作和生命周期；用户可以拥有插件自己产生的资源域，也可以明确把外部资源交给它管理。宿主维护跨插件 grant 和不透明租约，插件维护其资源内部实现。多工作区等需求通过有名称的 binding/multiplicity 表达，不靠不断新增固定 kind。

授权应尽量在安装/绑定时一次完成，只有新增权限或敏感不可逆操作才需要额外确认；不能把“每个 hook 都弹审批”当作稳定性设计。授权只是限制实际数据与副作用访问，不限制开发者提供哪种算法或 UI。

## 17. Rust 主机与 JS 插件：优雅边界及 Rust 插件收益

本节保留包含 Rust 插件时的技术选型分析，不表示 Rust 是开放其他环节的前提。按本轮“先不考虑增加 Rust 插件”的情景，先推进 JS 与内置 Rust 的共同消费边界，不实施 native adapter/SDK；具体调整见 §27.3。

### 17.1 首要问题是接口边界，不是语言对立

不能跨语言直接传递 Rust 的 trait object、借用、`Arc`、异步 Future 或内部服务引用，但可以传递请求、结构化结果、流、取消通知和资源 handle。大部分 Agent 扩展是粗粒度 I/O 与策略调用，适合这种边界；不需要把每次 Rust 函数调用都 RPC 化。

现有 [`JsKernelPluginAdapter`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs) 已把 JS 调用适配成 Kernel handler/context/resource 接口，证明方向可行。真正缺口是宿主准入和实际消费，而不是先换语言就能解决的问题。

建议公共契约以语言无关 DTO 定义；继续利用现有 Rust canonical contracts 及 schema/TS 生成设施，Rust trait 只是主机内部 port。JS 和 Rust SDK 都实现同一套线协议，不各自维护能力目录、Preset 格式或授权系统。

### 17.2 执行形态比较

| 方案 | 优点与适用场景 | 真实代价/限制 | 建议 |
|---|---|---|---|
| JS/TS + Node 进程 | npm 生态、网络/业务集成、UI 配套；已有主机 | npm 供应链、进程资源、共享进程故障；普通 Node 不是安全沙箱 | 保留主力，扩展契约与隔离能力 |
| Rust 独立进程 + 同一协议 | 原生 SDK、设备/浏览器控制、计算、完整 Agent Runtime；没有 Rust ABI 绑定 | 各 OS/架构制品、签名/升级、进程成本；跨界仍有序列化 | 确有原生库/性能需求时优先选此形态；不是 JS 开放路线前置 |
| Rust 编译为 Wasm component | 受限算法、过滤器、解析/评分/压缩策略；类型化接口与内存隔离 | host imports、WASI 支持、异步/流及线程要按所选工具链验证；不能直接用任意 OS crate | 有需求与实测后加入，非全平台改造前置 |
| 内置 Rust 静态模块 | 零 IPC、可复用现有 Rust 实现；默认高性能路径 | 需重编译宿主，进程内故障影响主机；不是用户安装型插件 | 作为官方实现方式，保持相同逻辑契约 |
| Rust 原生动态库 | 进程内调用，部分原生场景低延迟 | Rust ABI 无稳定保证；内存、allocator、panic、线程及卸载风险 | 不作为公共插件方案 |
| 嵌入 JS 引擎 | 部分纯 JS 可减少进程成本 | npm/Node API 兼容与原生模块问题；另建宿主维护面 | 当前无必要，不再增加一条主路径 |

这里的“Rust 插件支持”应是用户可安装的 Rust 制品 + SDK + 生命周期管理，不只是仓库里多写一个内置 crate。Rust 插件不会自动解决 UI 契约、Skill 内容装载、Provider 选择、M1 双分支或冻结会话问题。

### 17.3 若增加 Rust：JS 与 Rust 进程共用执行协议

长期包模型可容纳 UI 资源、JS 服务、特定平台的 Rust 服务，以及之后可选的 Wasm 制品；都是同一插件身份和 capability 贡献，执行器按发布声明选择。无需同时支持所有形态；若选择 Rust 路线，先增加 Rust process adapter，JS-only 路线则不实施该 adapter。多后端同包编排不是首期范围。

公共调用边界需要的基本信息是：请求标识、操作名、输入、截止时间/取消、结构化错误、事件流、资源 handle。身份与已授权上下文由宿主注入，不让插件自报 owner。现有 N1 RPC/进程管理可复用，但 N1 消息合同是特定 JS host 设计，不能仅把 `node` 换成一个可执行文件就宣称 Rust 插件接通。

建议先提取最小通用协议，保留各执行后端薄适配：

- JS adapter 在 Node 中装载模块；Rust adapter 启动对应目标平台可执行文件；未来 Wasm adapter 调用组件导出。
- 同一份接口数据定义生成 SDK；不同 wire encoding 可以适配，但不能手工维护三份不一致的业务语义。
- 流式模型响应采用有界事件流，支持消费者取消；大文件/图像传制品引用或资源流，不把所有数据塞进 JSON。
- 高频纯计算尽量成批调用；不要为每 token、每字节、每个内部函数跨进程往返。先测实际瓶颈，再决定是否需要 Wasm 或静态内置实现。

不要新建全局消息总线、Service Mesh 或自制 Rust FFI 框架来解决当前问题。已有 process supervisor 可共享进程生命周期 primitive，但 Nomi Runtime、JS module、UI Surface 的状态不必被强压成同一种内部状态机。

### 17.4 必须明确的隔离事实

当前 [`extension-host.mjs:226`](../../crates/backend/nomifun-js-host/assets/extension-host.mjs#L226) 使用普通 `import()` 加载模块；[`supervisor.rs:1217`](../../crates/backend/nomifun-js-host/src/supervisor.rs#L1217) 启动 Node 并配置 stdio。这里不能据此声称“第三方 JS 已受强沙箱隔离”。共享 host 的异常/无限循环也可能影响同 host 其他插件。

**进程隔离主要隔离崩溃，不自动隔离文件、网络、凭证或同权限进程。** Rust 独立进程同样如此。通过 SDK 校验 grant 不能阻止拥有 OS 权限的插件绕过 SDK 直接访问资源。

建议先明确产品执行档位，不做名义上的沙箱：

- 可信本机插件：安装者明确接受本机权限；仍隔离故障、限制资源和清理进程。
- 不可信插件：只在真实受限的 OS sandbox 或受控 Wasm imports 中执行；未实现的平台不能宣传强隔离。完整 npm/原生 SDK 兼容与严格隔离存在代价，需要按需求取舍。

可按插件/信任域分进程，避免所有用户插件共享一个易阻塞主机；这比仅增加 Rust 支持更直接改善可靠性。具体分组策略通过并发和内存测试确定，不预先创建复杂调度平台。

Wasm 隔离也不是万能：宿主授予的文件/网络 imports 仍代表真实权限；无限循环要有 fuel/epoch 等中断与资源限制，阻塞 host call 要单独有截止时间。它不能自动把任意 Rust crate 变成可沙箱运行的组件。

### 17.5 原生动态库为什么不推荐

Rust Reference 明确说明 Rust ABI 不提供稳定性保证。使用 `extern C` 能制定自己的稳定 C ABI，但仍要处理内存所有权、allocator、错误、并发、卸载和版本，且动态库与宿主共享地址空间。这正是本项目希望避免的长期复杂度。

因此：**若增加 Rust 插件，优先以进程协议接入；不建议增加任意 Rust dylib 装载。** 是否投入 Rust/Wasm 后端，应由代表性原生库、性能或安全需求验证收益；当前可先不投入，不妨碍 JS 插件开放其他部件。

本节外部技术依据（2026-09-13 查阅，非本仓库运行验证）：

- [Rust Reference：外部块 ABI](https://doc.rust-lang.org/reference/items/external-blocks.html#abi)：Rust ABI 无稳定保证，C ABI 按目标平台规定。
- [Node.js：VM](https://nodejs.org/api/vm.html)：明确指出 `node:vm` 不是安全机制，不能拿它作为运行不可信代码的沙箱替代。
- [Component Model：为什么需要组件模型](https://component-model.bytecodealliance.org/design/why-component-model.html)：WIT 与跨语言、独立编译组件的类型化调用边界。
- [Wasmtime：Security](https://docs.wasmtime.dev/security.html)：Wasm 隔离及 WASI capability-based 文件访问的边界。

## 18. 问题四：Skill 装载未闭环究竟影响什么

### 18.1 更精确的断点定位

原版“完整装载链未闭合”的方向成立，但必须补上“ID 已经传递”的事实：

```text
N1 Skill 声明 / 制品
  → materialize SkillDefinition
  → 编译 SkillRef、body digest、required capabilities 与来源锁
  → project_internal 只投影 Skill ID
  → included_skills → agent_enabled_skills → extra.skills
  → 传统目录/名称解析 → Nomi 扫描并创建 SkillTool
                         ↑
          缺少按插件来源锁解析正文/引用资源、校验并装载的统一桥接
```

直接证据：

- [`compiler.rs:1411`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L1411) 生成 `ResolvedSkillLock`；[`compiler.rs:879`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L879) 校验 Skill provenance。
- [`nomi_core_agent_projection.rs:170`](../../crates/backend/nomifun-app/src/router/nomi_core_agent_projection.rs#L170) 只提取 `lock.skill.id`；没有把锁定内容交给 runtime。
- [`service.rs:4316`](../../crates/backend/nomifun-conversation/src/service.rs#L4316) 将 included skills 放入临时输入；[`service.rs:4384`](../../crates/backend/nomifun-conversation/src/service.rs#L4384) 计算名称快照。
- [`skill_resolver.rs:41`](../../crates/backend/nomifun-conversation/src/skill_resolver.rs#L41) 使用 SkillPaths；[`bootstrap.rs:694`](../../crates/agent/nomi-agent/src/bootstrap.rs#L694) 使用 `load_all_skills` 扫描。
- [`skill_tool.rs:20`](../../crates/agent/nomi-agent/src/skill_tool.rs#L20) 的 Skill 还有变量替换、shell、fork 等执行行为，不能把它简化为一段总是安全的 Markdown。

`ResolvedSkillLock` 本身包含 SkillRef、body digest 和所需 capability；完整来源还在 Revision contribution lock/制品元数据中。闭环消费要使用整条锁链，不应假设单个 Skill lock 已包含全部 provenance。

### 18.2 影响范围

| 情况 | 实际影响 |
|---|---|
| 发布了只有 N1 包里存在的 Skill | package/catalog/compile 成功不足以保证 runtime 找到正文；可能不可用或没有进入预期行为 |
| Tool + 配套 Skill 的插件 | Tool 可以独立调用；但模型可能没有得到插件作者提供的使用规则、工作流和限制，效果下降 |
| 本地恰有同名 Skill | 名称能解析不等于加载了所选插件版本；存在来源混淆风险，不能把偶然命中当作闭环 |
| 插件升级/删除与历史 Session | 目录文件与锁定制品若分离，无法保证老 Session 使用原版本内容；也可能找不到文件 |
| 依赖检查 | 编译时检查所需 capability，不等于 Skill 真正执行时资源已绑定、延迟能力已激活或脚本已授权 |
| 传统内置/工作区 Skill | 原有目录路径仍可工作；不能由此推断“所有 Skill 都坏了” |
| 不使用 Skill 的 Tool/UI 插件 | 不直接受此断点影响 |

这是插件平台正确性与体验问题，**不是单凭静态代码就能断言所有场景都会失败的运行故障**。本次未执行真实 Skill 安装 smoke；以上风险来自锁信息与 runtime 输入边界的核对。

### 18.3 建议的最小闭环

1. 提供一个内部 `SkillSource`/装载 port：按精确 Skill 引用、来源锁和制品返回正文及资源清单；N1 制品、内置目录、工作区目录都是来源 adapter，而不是三套 runtime。
2. Session 启动时解析选中的 Skill，校验正文 digest、来源、引用资源与所需能力；生成完整运行时 Skill descriptor。不要退回“复制进某个同名全局文件夹后扫描”的隐式方案。
3. 使用现有 Nomi Skill loader/执行器的解析和执行能力，但让它能接收已解析描述，而不是只靠磁盘扫描。目录兼容能力保留，不代表保留两份技能事实。
4. 将索引注入、按需正文读取、附件/引用文件读取、脚本/fork 执行分别接通。Skill 不自动授予额外 Tool 权限；引用脚本必须走同一资源/执行授权边界。
5. Skill 所需 on-demand capability 在使用前统一激活；不满足依赖时给出明确错误。正文进 prompt 不等于 LLM 一定遵循，验收应验证实际装载内容及调用链，而不是保证模型行为。
6. 锁定的包制品按存活 Session/Revision 引用保留，卸载撤销执行资格与回收磁盘制品分开处理。可变工作区 Skill 若要纳入精确可复现会话，必须快照为制品；否则明确标为动态内容，不能宣称已有强版本锁。

验收至少包含：仅在插件包存在的 Skill 真正进入 runtime；同名不同来源不串用；加载内容 digest 一致；引用文件可读；缺失/越权依赖被识别；升级后旧会话不漂移；传统 Skill 仍可用。自动化使用可捕获真实装载文本的 runtime fixture，再补一个真实 Agent 调用 smoke。

## 19. 问题五：更简单、更开放的 Agent 组装模型

### 19.1 现有架构的问题不是“有锁”，而是相同事实被多次翻译

应保留的基础是：插件身份清晰、显式选择、不可变会话计划、资源授权和可解释失败。它们有真实稳定性价值，不应为了“链路短”全部删除。

需要收敛的结构：

1. **N1 与 M1 两套解析结果及执行入口**：`ResolvedCapability` 与 `ResolvedMiniAppCapability`、独立 materialization/invoker，让每个消费者都必须认识来源类别，见 §13。
2. **静态 native projection、用户 Tool、bundled Context/lifecycle 分开准入**：能力“能否执行”依赖多处白名单/分支，而不是贡献声明与执行器支持矩阵。
3. **保存/打开会话时重复解析**：[`compile_nomi_plugin_snapshot`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L994) 目前对当前 Registry 重编译，再与已保存 envelope 比较。这提供严格漂移检测，但也让历史会话启动依赖当前目录形态。
4. **Skill 精确引用退化为字符串**：上游花成本锁定，下游没有精确消费，见 §18。
5. **引擎实际仍封闭**：[`AgentRuntimeHandle`](../../crates/backend/nomifun-ai-agent/src/runtime_handle.rs#L180) 的生产变体是 `Nomi`，Mock trait-object 是测试支持，不是用户 Runtime 扩展。
6. **已有接口与实际实现未分开组装**：[`LlmProvider`](../../crates/agent/nomi-providers/src/lib.rs#L143) 已是 Rust trait，但创建仍走 [`create_provider`](../../crates/agent/nomi-providers/src/lib.rs#L801)；Nomi 的压缩、hooks、循环逻辑仍聚合于 [`engine/mod.rs`](../../crates/agent/nomi-agent/src/engine/mod.rs)。Rust interface 存在不等于产品可安装、可选择、可替换。

### 19.2 推荐目标：一套对象，三段处理，不另造平台

只保留用户需要理解的三类对象：

- **插件**：可安装/发布的制品，可提供多个 capability；执行形态属于制品声明。
- **能力与契约**：capability 描述可消费贡献，Role 描述可替换部件，Provider 表达实现关系；都在同一目录中。
- **Agent 配置**：选择能力、Provider 和必要参数；保存为 Revision/Snapshot，具体资源在 Session 绑定。

内部主要处理链：

```text
插件发布 → 单一 Capability Catalog
                    ↓
用户 Agent 配置 → resolve/compile → 冻结 Snapshot（执行计划）
                                         ↓
                     Session bind：资源、当前授权、执行器就绪
                                         ↓
                    选定 Runtime + 公共能力执行接口
                      ├─ 默认 Nomi 与其策略组件
                      ├─ JS / Rust 进程插件
                      └─ 后续可选 Wasm 执行器

UI Provider → 同一应用 command/query/event API → 上述 Session
```

这里的“执行计划”就是演进后的 `ResolvedSnapshotContent`，不是新增一个与 Snapshot 双写的 `AgentPlan` 存储。SDK、RPC、Rust trait 是同一逻辑调用的不同适配面，也不是需要依次穿过的三个业务 coordinator。

三段的职责应明确：

1. **发布**：验证包/贡献契约、生成 Catalog 条目与执行描述。N1/M1 差异在这里和执行 adapter 内吸收。
2. **编译**：解析依赖、Provider、顺序、schema 和制品版本，检查所选 Runtime 支持的接口，生成一份冻结计划。
3. **绑定/执行**：不重新选择实现，只解析具体资源、检查当前撤销/可用性、准备执行实例，并调度计划。实际有副作用的调用继续执行操作级授权。

发布/会话/调用的校验针对不同事实，不能合并成一次后永远信任；但同一份契约与绑定不应在每层重复推导。

### 19.3 用户只做一次选择，系统完成编译

产品界面应是“选一个默认部件实现、添加若干能力、绑定所需资源”，不是让用户编辑 Role digest、artifact digest、mount ID、Snapshot ID。依赖和版本解析由后端完成，界面展示来源、能力、缺失配置和影响摘要。

示意配置（仅解释目标语义，不是当前 API；实现时沿用 canonical Preset/Role 字段，不再保存此平行格式）：

```yaml
agent: 我的研究助手
runtime: acme.research-runtime
providers:
  model: platform.model-gateway
  search.web: acme.search
  context.pipeline: acme.research-context
tools:
  - platform.files
skills:
  - acme.research-method
ui: acme.research-workbench
```

Runtime 若不支持 `context.pipeline` 替换，应在配置阶段说明，而不是运行时静默忽略。新用户可以只选官方模板，系统自动补齐默认；高级用户再替换任何公开部件。模板展开后成为普通配置，没有永远覆盖用户选择的隐藏模板逻辑。

Preview 和 Save 调用同一个编译服务。可以提供“保存并使用”一次操作；若 Preview 后 Catalog/配置已变化，返回简明差异或要求确认，不让用户手工再走一串 Compile/Lock/Bind 页面。Snapshot 是稳定性实现，不应成为使用负担。

### 19.4 默认 Nomi 组件化与完整 Runtime 替换并行成立

不要一次性把 Engine 每个函数切成微插件。先提取具备独立替换价值的边界：模型调用、Context 管线、工具执行、历史/压缩、委派、输出事件；每个边界有真实第二实现时再验证抽象是否够用。

外层 Runtime 最小协议建议覆盖：

- 创建/打开会话并绑定冻结计划；
- 开始 turn、取消 turn、关闭会话；
- 查询当前状态、订阅有顺序的事件流并识别终态；
- 显式报告可选功能：steer、resume、fork、checkpoint，以及支持哪些内部可替换策略。

不要求所有引擎支持每一种可选功能；消费者根据支持声明呈现功能或在组装时报告不兼容。也不要求替代引擎使用 Nomi 的 prompt、压缩格式或内部 checkpoint。

宿主负责会话身份和接受执行结果，Runtime 负责一次会话的执行状态；同一 Session 不允许两个引擎同时充当执行 owner。不同 Runtime 的 checkpoint 不假定互通：不支持迁移时可显式新建会话/导入可读历史，而不是伪装“继续同一个内存状态”。

Runtime 通过公共 Tool/模型/资源服务使用已绑定能力。若用户选择完全自主访问网络/系统的可信 Runtime，应明确这是更宽的信任档位，不能一边允许绕过代理，一边声称所有操作都由 Kernel 细粒度管控。

复用当前 `NomiCoreSessionOwner` 等已统一消费边界；将 `AgentRuntimeRegistry` 的构造与 handle 演进为生产可注册的 Runtime factory/port。Fresh-v4 的契约可以作为参考或迁移输入，但不再维持两个同时竞争的生产 Session authority。

### 19.5 把锁定保留下来，把重复解析移出去

建议 `Save` 产生可直接执行的冻结计划；`Open` 只验证计划完整性、被引用制品仍保留、授权未撤销和所需执行器可用，不再从“当前 latest Catalog”重新求解同一依赖图。

这需要先实现精确版本制品寻址/保留和足够完整的执行描述，**不能现在直接删除 Session 重编译校验**。现有重编译包含正确性保护；只有新的等价校验与回归覆盖到位，才可替代它。

可缓存编译结果，缓存键覆盖配置、实际选中制品/契约及编译器语义版本，不让不相关插件发布无谓地使全部 Agent 失效。执行时撤销权限、禁用插件、资源丢失等仍必须生效；不可变计划不是永久授权。

### 19.6 内置能力必须真正走同一接口

内置与第三方可以使用不同执行后端，但不能使用不同产品能力等级：内置实现也发布 capability/Role，受同一绑定解析、生命周期与错误语义约束。官方静态 Rust adapter 可以零 IPC，不要求为了形式统一把所有内置模块移到子进程。

迁移验收必须检查原先的直接调用是否已退出，而不仅检查新接口存在。每开放一个部件，删除对应的内置 ID 分支、来源白名单或旧组装入口；“新接口 + 永久旧路径 + fallback”不是完成。

### 19.7 简化失败语义，不追求不存在的无条件稳定

- **必需组件失败**：停止相关操作并解释来源；不能把缺少模型/Context 管线的执行报告为成功。
- **明确可选的观察/展示贡献失败**：允许跳过，但留下诊断；是否可选由契约/用户配置确定，而不是统一吞错。
- **Provider 替换**：默认在新 Session 生效；有状态组件必须有显式迁移，否则重新建立实例。
- **副作用结果未知**：保留未知状态，不自动重试；普通纯函数失败不需要全套 durable receipt/outbox。
- **UI 故障**：可恢复内置视图，不取消或重放后台正在执行的 turn。

平台能保证接口一致、故障可见、资源可回收和不暗中切换；不能保证任意第三方算法都正确，或任意 native 插件都不会破坏其已获授权的资源。

## 20. 实施路线：每步交付真实替换，同时删除一条旧分支

不建议大爆炸重写，不建议先把所有 SPI 定义完再找消费者。按风险与依赖推进以下纵向切片；跨平台发布时验证真正涉及的平台行为，不把所有平台全量测试作为每次文档/局部改造的门槛。

| 阶段 | 交付内容 | 必须退出/收敛的旧结构 | 可观测验收 |
|---|---|---|---|
| A. 统一贡献与执行描述 | 确定统一 Plugin/Release/capability 身份；N1/M1 用执行 descriptor 表达差异 | 活动 Snapshot 双 capability 集、消费者中的 M1 专用判断；历史数据做明确迁移 | N1 与 Service Tool 经同一 Agent 调用入口；原有数据可读、失败可解释 |
| B. 第一个可替换契约 | 复用 Role/Provider；一内置一用户实现；开放 N1 相应合同与 adapter | 对该能力的具体内置 ID 直连和 bundled-only 准入 | 用户选自己实现后，内置路径不再被调用；旧 Session 保持原绑定 |
| C. 闭合现有声明能力 | Context、Skill、Resource、MCP、Service resource 按真实消费者逐项接通 | Skill 名称降级、重复 MCP 工具、固定资源特判逐项退出 | 插件包唯一 Skill 可用；Context 真注入；自定义资源被真实调用并释放 |
| D. 完整 Agent UI | 公共会话 API、插件 Agent 页面、之后扩展到 Shell | 页面直接耦合内部业务 store、插件仅能 KV/Service 的封闭边界 | 插件 UI 完成发送/流/取消/历史；切 UI 不重放 turn；恢复入口可用 |
| E. Rust process SDK | 同一契约与公共执行协议；平台制品选择、取消、升级/回收 | 不再让能力资格依赖语言；避免另建 Rust Catalog/Preset | JS/Rust 分别实现同一能力；崩溃/超时不拖死宿主；清理进程树 |
| F. Nomi 策略与 Runtime | 抽取模型/Context/执行接口，接入第二个真实 Runtime | 生产 Nomi-only handle、该路径重复 projection/组装 | 两种 Runtime 均可真实完成 open/turn/observe/cancel；状态归属唯一 |
| G. 深层宿主与可选 Wasm | 存储、调度、认证/隔离等按第二实现需求提取；评估 Wasm | 新端口接入时删除对应硬编码，而非永久双主链 | 存储迁移/重启恢复、权限边界；Wasm 有代表性性能和隔离收益 |

A/B 的边界确定后，C/D 可以按不相交模块安排实施；E 不必等全部 UI 完成。但本次只给出路线，没有启动多代理开发或任何代码迁移。

### 20.1 一条端到端样例胜过一套庞大的门禁体系

建议选一个“研究助手”参考插件：自定义搜索 Tool + 配套 Skill + Context + Agent 页面；之后让 Rust Provider 替代搜索，并接入一个不同 Runtime。它逐步验证整个发布、选择、装载、调用、观察、取消、升级链。

测试重点是行为：真实选中了谁、读到了哪个 Skill 正文、资源是否释放、事件是否连续、取消是否有效、故障能否恢复。结构测试/manifest/digest 可以辅助，但不能代替真实消费者，也不能成为必须长期双写的另一套系统。

### 20.2 防止架构复杂度继续增长的实施纪律

1. 每增加一个公共抽象，说明至少一个真实消费者与替代实现；没有替换收益的不拆。
2. 统一的是领域身份、选择规则与契约，不是强行统一所有后端内部实现。
3. 一份事实只持久化一处；投影可重建，避免 Registry/Catalog/Session 各存可独立修改的同一配置。
4. 每个迁移切片列出旧路径退出条件；数据迁移保留必要历史，临时协议适配明确期限。
5. 不把所有函数 RPC 化，不把所有失败 durable 化，不把每个按钮插件化。
6. 不以新增 Wasm、Rust dylib、第三个 JS runtime 来掩盖现有宿主接口缺失。
7. 公开接口必须有调用方与实现方的契约测试；接口版本发生不兼容变化时显式升级，不静默猜测。

## 21. 本次复核口径与最终判断

### 21.1 证据范围与限制

本次基于报告基线提交及当前工作区静态阅读，核对了 N1 校验、动态 Catalog 准入、Kernel Role/Provider/Context/Resource 接口、Skill 编译与 Nomi 装载、M1/Surface Bridge、React Router、Runtime handle、模型接口和 Session 重编译。相关代码证据已就近列在各节；外部语言/隔离判断引用官方技术文档。

研究期间工作区出现了并行代码修改及文末统一实施基线补充，均予以保留；附录只调整章节编号，未重写实施内容。本报告的现状以开头提交和本次实际读取的链路为依据，不将并行未完成改造计为已验证交付。

没有执行生产服务、真实模型请求、插件性能基准或跨平台 UI smoke。因此：

- 现状结论是代码链路判断，不新增“线上验证通过”或“三平台已兼容”的声明。
- Rust/Wasm 的性能收益、进程粒度和 UI 隔离交互需要代表性 spike 验证，不能从语言名称推导性能倍数。
- §14～20 的成本是相对工程复杂度。新增 §25 提供带范围和假设的规划级人周粗估，不是实测工期或交付承诺；需经 §26 的启动验证校准。
- 本次只修改这份报告，验证文档差异、引用目标与格式，不运行无关的完整构建/测试。

### 21.2 最终判断

**目标可实施，而且现有 capability、Role/Provider、JS adapter、Surface 和 Session facade 已提供有价值的基础；但当前仍是“业务能力可装配”，不能描述为“全栈主机部件可替换”。**

最值得投入的不是“把全部逻辑改成 JS”或“让 Rust 动态库接管宿主内存”，而是：

1. 让所有可替换实现进入同一 capability/Role 体系，用户显式选择，内置实现不例外；
2. 把 UI、Nomi 策略和完整 Runtime 变成真正有消费者的公开边界；
3. 修复 Skill/资源等已经声明但未完整消费的断链；
4. 以一份冻结执行计划缩短组装链，运行时只绑定与校验必要的动态事实；
5. 用共用协议支持 JS 与 Rust 进程插件，按实测需要引入 Wasm；
6. 把最小信任根与部署级替换讲清楚，不把必要安全不变量扩大成产品功能封锁。

可概括为：**同一能力体系、多种实现语言、显式组装、单一执行真相；默认内置组件也是可以被用户替换的实现。**

## 22. Rust 插件的设计层级：并列执行后端，不是第二套平台

### 22.1 先把三个容易混淆的 runtime 分开

| 概念 | 当前/目标职责 | Rust 是否需要另做一套 |
|---|---|---|
| 语言运行环境：当前 `nomifun-js-runtime` | 发现、下载、探测、选择 Node 及其版本 | **不需要对应的 Rust 环境管理器**；用户安装预编译 native 制品，不安装 rustc/Cargo |
| 插件执行后端：JS Host + Kernel adapter | 装载模块、调用 capability、管理实例、取消和回收 | **需要并列的 native-process adapter 与 Rust SDK**，与 JS 后端同级 |
| Agent Runtime：Nomi 或替代引擎 | 拥有 Agent Loop、模型/工具编排和会话执行状态 | 与实现语言正交；一个 Rust Tool 不是一套 Agent Runtime，完整替代引擎另列工作包 |

直接依据：

- [`nomifun-js-runtime/src/lib.rs`](../../crates/backend/nomifun-js-runtime/src/lib.rs) 明确只负责 Node discovery/probing/fingerprinting/selection，不拥有 Catalog 或插件部署。
- [`managed.rs`](../../crates/backend/nomifun-js-runtime/src/managed.rs) 处理 Node 下载/解包；Rust native 制品没有相同的运行环境下载需求。
- [`nomifun-js-host/src/lib.rs`](../../crates/backend/nomifun-js-host/src/lib.rs) 与 [`nomifun-js-kernel-adapter/src/lib.rs`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs) 分别承担进程通信和 Kernel 注册适配。
- [`nomi-process-runtime/src/lib.rs`](../../crates/shared/nomi-process-runtime/src/lib.rs) 已提供 `ChildProcessBuilder`、进程树清理、监督和恢复 primitive；native 后端应复用它们。
- [`package.rs`](../../crates/backend/nomifun-agent-contracts/src/package.rs) 当前 entrypoint 枚举仍是 InProcess/JavaScript；因此 native 制品声明、校验和装载确实是新增工作，不是已有功能改个名称。

这些是本轮核对的职责事实，不代表工作区正在进行的 Plugin 统一改造已验收完成。

### 22.2 正确的同级关系

```text
统一 Plugin / Release / Capability / Role / Preset / Session
                            │
                  公共执行契约与授权上下文
                            │
       ┌────────────────────┼────────────────────┐
       │                    │                    │
 JS 模块执行后端      Native 进程执行后端     内置 Rust adapter
 Node + JS SDK       预编译程序 + Rust SDK    宿主静态链接
       │                    │
       └──── 复用进程管理、传输基础和制品管理 ────┘

Wasm：未来可增加的执行后端，本期不建设。
Agent Runtime：上述后端能承载的一种契约实现，不是第四种语言。
```

因此答案是：**在“插件执行后端”层面与 JS 同级；在“插件平台”层面共用一套；在“语言环境管理”层面不对称，不应照抄 JS runtime。**

推荐内部称 `native-process`，而不是把执行协议命名为 `rust-runtime`。Rust 是首个官方 native SDK；将来其他语言若实现同一协议，不需要新增平台类型。但首期不承诺其他语言 SDK。

### 22.3 共用什么、新增什么、暂时不做什么

| 类别 | 具体内容 | 安排 |
|---|---|---|
| 原样或少量适配复用 | 插件产品/发布/安装、Catalog、Role 选择、Snapshot、资源授权、日志入口 | 不建 Rust 专用副本，不新增独立 Rust 插件工作台 |
| 提取小公共边界 | 请求/结果/错误/取消/有界事件、握手版本、调用上下文 | 由共同工作包 P2 完成；JS 保留模块装载特有行为，native 保留启动特有行为 |
| 复用底层生命周期 | process supervisor、进程树终止、stderr/stdio、制品校验与路径边界 | 复用 primitive，不把 Node mount 状态机硬搬到 Rust |
| native 特有新增 | 按 OS/架构选择预编译制品、可执行入口、实例启动、协议握手、native Kernel adapter | 工作包 P5；首期 Windows x64 一个目标 |
| Rust 作者体验新增 | 小型 SDK、示例、构建/打包说明、调试日志和契约测试 | 作者机器或 CI 使用 Cargo；用户安装时不编译源码 |
| 不建设 | Rustup/Cargo 版本管理、另一套 registry、Rust 专用 Preset、dylib ABI、自定义编译云 | 不进入排期，不以“以后可能用到”为由预建 |

Rust 程序不需要语言解释器，但可能依赖系统库、MSVC runtime 或其他 native library。发布 manifest/打包检查仍需识别目标和依赖；“预编译”不等于“一个二进制在所有系统运行”。首期只接纳明确满足目标要求的制品，缺失依赖给出可解释错误。

公共协议采用适合现有代码的版本化 stdio 请求/响应与有界事件即可。标准输出用于协议，日志写标准错误；宿主验证请求关联和已握手版本。先支持实际需要的操作，不为尚未开放的模型流、任意 callback 或跨机 RPC 定义完整框架。

### 22.4 首期 Rust 支持的上限

首期交付：可信本机、预编译、Windows x64、一个 release 选择一个后端，支持 Tool 和本期已开放的 Context 契约，复用 Role 绑定与正常安装/升级/禁用流程。可附带 UI 资源，但不要求同一包同时协调 JS 服务与 Rust 服务。

启动进程按插件实例/授权作用域隔离，必要时惰性启动；实例只由一个生命周期 owner 管理。首期不做跨租户共享池、运行中热换二进制或任意多进程编排。即使是可信插件，也要有超时、取消、协议大小边界、故障诊断和进程树回收。

完整 Agent Runtime、跨平台 native 分发、Wasm、强沙箱、源码插件自动构建都不计入这个交付。它们不是 Rust Tool 插件的前置条件。用户对本机 native 代码的信任必须明确；digest 只校验完整性，不能证明作者可信。

## 23. 把大目标分成可以独立交付的版本

### 23.1 不把全栈目标压进第一个版本

整体确实是一个中大型架构项目；“开放所有环节”不是若干开关的工作量。最大的成本是拆出真正稳定的消费边界、迁移现有用户路径并删除旧分支，而不是增加一种编程语言。

建议按以下三个版本组织，版本名是本文计划标签，不要求新增产品版本体系：

| 版本 | 对用户能承诺什么 | 包含哪些工作包 | 明确不承诺什么 |
|---|---|---|---|
| R1：可替换能力与 Agent UI 的开发者预览 | 一个真实系统契约有内置/用户可选实现；Tool、Context、插件 Skill 真正消费；插件完整 Agent 页面；JS 与 Rust native 实现可安装 | P0～P6 | 不叫“任意主机部件全栈完成”；不含整个 Loop 替换、完整 Shell、任意资源 Provider 或强沙箱 |
| R2：开放式 Agent Runtime 平台 | 扩展资源/MCP/lifecycle；模型与关键 Nomi 策略可替换；第二个真实 Runtime；完整 Shell | P7～P10 | 不承诺不同引擎 checkpoint 通用或所有宿主设施均已可替换 |
| R3：部署级主机开放与目标平台发布 | 存储/调度/认证等选定宿主服务可在启动时替换；目标平台验证与发布 | P11～P12 | 不承诺无条件热换信任根，不把所有第三方实现的正确性包办 |

§16 的完整矩阵仍是方向，不能因为先发布 R1 就把剩余环节永久封闭。R2/R3 的开发清单在 R1 真实使用后细化；没有第二实现需求的内部函数不提前抽象。

### 23.2 R1 的范围要足够小，也要真实有价值

R1 以一个“研究助手”插件为纵向验收样例：选择自己的搜索/检索实现，加载包内 Skill 与 Context，在插件 Agent 页面完成一次真实会话。native SDK 再提供相同契约的 Rust 实现，验证替换不依赖语言。

至少选一个**原本由系统固定调用、会在真实产品路径中使用的部件契约**。不能只给两个新增 Tool 起相似名称就当作系统替换完成。P0 决定具体示范部件；若搜索并无真实系统固定调用点，就选已有 Context 或另一实际部件。

R1 的边界：

- Provider 的注册、选择、冻结与实际调用一致；无需每次手工 Preview/Compile，提供“保存并使用”的正常入口。
- Skill 优先交付锁定正文、只读引用资源、索引/按需读取与已支持的 Tool 依赖；shell/fork/hook 组合若不能完整保持授权语义，应明确拒绝该模式并留在 R2，不能静默降级。不是另写一个 Skill 解释器。
- Plugin Agent 页面覆盖发送、流式结果、取消、历史、错误与重新订阅；不是仅能显示一段对话的展示页。完整工作台 Shell 和像素级区域替换后置。
- 资源先复用现有 typed binding；自定义 kind、多实例资源、M1 带资源 Service 的全面开放放 P7。R1 首个替换例子不得依赖未完成的资源扩展。
- 先保留有正确性价值的 Session 重编译保护，只统一选择与投影语义；删除重复解析要等完整执行 descriptor 和等价校验就绪，归入 P8/P9。
- 一个后端入口可配 UI，多个后端的包内编排后置；不做新的插件市场、付费分发系统或在线编译平台。

### 23.3 可选的更小首发

按本轮暂不新增 Rust 插件的情景，先规划 **R1-JS**：保持同一公共边界，移出 P5 的 native 后端；P0 不再要求 native 小样，改为验证内置 Rust 与用户 JS 能共同消费契约。JS 的 Provider 替换、Skill/Context、插件 Agent 页面仍能形成实际产品价值。

Rust 保留为可选后续投资，不是 R2/R3 的阻塞依赖，也不表示用户已决定永久取消。只保持语言无关的数据边界，不预建空壳 native 框架。§27 给出该情景的完整任务对应；其余含 Rust 的版本/排期表用于对比，不应混排。

## 24. 可执行工作包、依赖与代码责任范围

### 24.1 R1 工作包

下表是规划级估算，1 人周 = 5 个全职工程工作日，假定熟悉 Rust/TS 与本仓库。工作量包含本包实现、定向测试、必要文档和正常 review；P6 只计跨包集成及发布验证，避免重复计算模块测试。多人参与一个包时按总人周计，不按等待时长计。

| 包 | 交付物与明确退出条件 | 主要责任范围（现有模块或拟新增薄适配） | 依赖 | 基础人周 |
|---|---|---|---|---|
| P0. 基线与两条验证 | 固定正在改造后的候选基线；选定真实替换部件；native 小样跑通调用/取消/崩溃；UI 走现有 API 完成一次发送/观察；据结果重估 | 报告、现有示例/测试、contracts/runtime/UI port 的最小 spike | 无；不等待所有长期功能设计 | 1～2 |
| P1. 统一主链收口 | 验收当前 Plugin 身份、Catalog、Snapshot 与 Session 合流；相关旧分支退出；N1 与发布型 Service 使用统一消费入口 | contracts、control-plane、kernel、plugin-platform、DB、app；已有统一改造优先完成 | P0；若并行工作已验收则扣除已完成部分 | 2～4 |
| P2. 第一个真实替换与公共协议 | 内置 + 用户 JS Provider 同契约；默认/Agent override；共用调用 DTO、错误、取消；Preview/Save 同一选择语义；真实调用不再直连内置 ID | contracts/kernel、JS adapter/host、app composition、Agent 工作台选择 UI | P1 的契约及统一主链稳定 | 3～5 |
| P3. Context 与 Skill 闭环 | 用户 Context 真注入；Skill 按精确制品装载正文/资源；依赖与不支持模式明确报错；旧目录来源继续可用 | plugin_tools、conversation skill_resolver、skill-library、Nomi bootstrap/SkillTool | P2 公共边界；先定 runtime descriptor | 3～5 |
| P4. 插件 Agent 页面 | UI contribution/绑定；会话 command/query/event Bridge；插件界面收发、取消、读历史、恢复订阅；内置界面可恢复 | UI Surface/Router、公共 bridge/types、app 会话 API；业务 API 仍由现有 owner 提供 | P2 的选择/身份边界；不依赖 P5 | 3～5 |
| P5. Native 后端与 Rust SDK | 预编译 native 制品正式安装、选中、调用与升级；Rust Tool/Context；错误、取消、退出清理；同一产品入口 | 新 native-process adapter/SDK、共享 process-runtime、少量制品校验/app 接线 | P2 协议稳定；Context 联调依赖 P3，但 Tool 可先完成 | 3～5 |
| P6. R1 集成与开发者预览 | 参考插件全链路、故障恢复、升级不漂移、Windows Web/桌面验证、可复现打包；文档和诊断可交给外部开发者 | 集成测试、发布脚本/示例、各包 owner 联合修复 | P3/P4/P5 | 2～3 |
| **合计** | **R1 基础工程量，不含风险余量** | — | — | **17～29** |

P1 不是要求把正在进行的统一工程再重做一次。当前工作区已包含大量 contracts/DB/app/UI 的统一改动；本轮没有验证其完整性。P0 按退出条件区分“已验收”“仍需集成”“尚未完成”，只估剩余工作，不把 Git 修改文件数量当进度。若发现基础功能损坏需要大范围修复，应重估 P1，而不是把修复隐藏在 P5 的 Rust 预算中。

具体 crate 名称只是责任定位，不预先规定新增十几个 crate。优先在现有模块内建立清楚接口；native adapter 和可独立依赖的 Rust SDK 确有发布/依赖隔离需要时再建小 crate。

### 24.2 Rust 专属新增工作量如何构成

P5 的 3～5 人周对应以下 15～25 人日。共同能力契约/协议提取已计入 P2，P6 只做跨产品联合验收，三者不能重复报账。

| native 子任务 | 基础人日 | 交付边界 |
|---|---|---|
| 目标/入口声明与制品校验 | 2～3 | Windows x64，精确 release/entrypoint，缺失目标/依赖报错；不执行安装期任意构建脚本 |
| 启动、握手、调用、取消、回收适配 | 4～6 | 复用 process-runtime；native 与 JS 使用公共调用语义，但不复制 Node module loader |
| Rust SDK 与参考 Provider | 2～4 | 作者实现 Tool/Context，不手写协议；可运行的同契约替代实现 |
| 构建/打包与产品状态接线 | 2～4 | 作者/CI 构建，复用安装入口；产品显示目标、就绪/错误和日志 |
| native 故障契约测试、文档与 review | 5～8 | 崩溃、超时、协议错误、旧版本、禁用、重启与进程树清理 |
| **合计** | **15～25** | **不含跨平台/强隔离/完整 Agent Runtime** |

这是“公共基础已就绪后”的边际预算，不是从零开发整个插件平台只需 3～5 人周。若实测表明必须重写现有进程管理或全面改造多版本制品存储，该前提不成立，应回到 P0/P2 重估；不能继续沿用这个数字。

### 24.3 后续版本工作包

| 包 | 具体范围与退出条件 | 依赖 | 基础人周 |
|---|---|---|---|
| P7. 扩展贡献闭环 | 自定义 ResourceProvider 与 Service typed resources；MCP mapping 统一调用；有限明确的 middleware/event/lifecycle 契约；补齐 R1 暂不支持的 Skill 执行模式 | R1；复用同一授权/执行接口 | 5～9 |
| P8. Nomi 关键策略与执行计划 | 模型调用/路由、Context 管线、压缩/历史视图与工具编排的有意义接口；有第二实现；完整 descriptor/等价校验到位后退出重复求解 | R1；依赖资源的策略需 P7；与 P9 共用 Runtime 边界评审 | 6～10 |
| P9. 完整 Runtime 替换 | Nomi 与一个真实外部 Runtime 通过统一 Session 接口运行；事件/取消/可选恢复一致；只保留一个执行 owner | R1；与 P8 先固定外层契约，不要求 P8 所有内部策略先完成 | 5～9 |
| P10. 完整 Shell | 导航、页面/布局选择、应用状态重建、恢复入口；不与 UI 插件共享宿主内部可变 store | P4/P6；独立于 P8/P9 引擎内部实现 | 3～6 |
| P11. 部署级服务开放 | 按真实需求依次提供 Transport/Scheduler、存储、认证/Secret、监督/隔离后端的替换接口；默认实现同接口；选定第二实现与维护窗口切换 | R2 已验证的 Session/资源边界 | 8～14 |
| P12. 目标平台与发布收口 | 已选定 native/UI/runtime 组合在 Windows x64、Linux x64、macOS arm64 验证；对应打包、依赖、必要签名/公证和发布说明 | 可提前做单平台 spike；正式收口依赖所发布范围稳定 | 4～7 |
| **后续合计** | **R2：19～34；R3：12～21** | — | **31～55** |

P7～P12 是低置信度的预算占位，不能直接作为开发承诺。其中 P11 跨越多个领域，正式启动前必须拆成独立服务小包；8～14 人周以复用现有设施并选择有限、代表性的第二实现为前提，不包括重做认证产品、数据库系统或自制 OS sandbox。§27.2 已将此前未明确分配的策略、UI 子范围等补入对应包；这些补项需拆卡重估，不能默认全部被本表原预算吸收。

完成上述工作代表所列主干部件具有真实替换路径；如果“任意部件”被扩展为每一种历史功能、每个 native 平台及所有策略组合都要完整第二实现，工作量不受这张表覆盖，需新增范围。不能把表里的粗估包装成开放性无限、成本有限的承诺。

### 24.4 依赖与允许并行的位置

```text
P0 → P1 → P2 ─┬→ P3 ─┐
               ├→ P4 ─┼→ P6 / R1
               └→ P5 ─┘   （P5 的 Context 联调与 P3 汇合）

R1 ─┬→ P7 ─────────────┐
    ├→ P8 ↔ P9 ───────┼→ R2 → P11 ─┐
    └→ P10 ───────────┘             ├→ P12 / R3
           目标平台 spike 可提前 ────┘
```

图中的 P8 ↔ P9 表示共享契约协调，不是编译时相互依赖；应先由共同 owner 固定外层 Runtime port，再分别做 Nomi 内部策略和外部引擎适配。

P1/P2 是首期关键路径，不宜让多人同时改同一份 contracts/compiler。P3/P4/P5 在接口稳定后才是适合并行的任务；它们共享的 app composition 接线由一个集成 owner 管理。阶段依赖满足即可合流，不要求先完成与该切片无关的全部未来 SPI。

## 25. 工作量、团队配置和日历安排

### 25.1 粗估的前提与可信度

以下是用于预算和组队的区间估算，不是根据现有历史交付速度计算出的承诺。需要在 P0 后首次校准，在 P2/R1 结束后再次更新。

估算前提：

1. 核心开发者熟悉本仓库、Rust async、TS/React 和跨进程通信；新人学习成本另计。
2. 复用当前插件发布/制品/进程管理/Session 设施；并行统一改造会先形成可验证基线，不从零重写平台。
3. R1 优先 Windows x64 的 Web 与桌面路径，native 插件为显式信任的本机预编译程序；不含恶意插件强沙箱。
4. 不建设新市场、远程构建、自动迁移任意引擎状态；R1 只支持 §23 明确的功能子集。
5. 团队能持续投入，且有真实模型测试条件；发布平台设备、签名/公证账户等可获得。外部等待可能延长日历，但不直接等于工程人周。
6. 普通模块测试/review 已计入工作包；集成包处理跨路径问题。额外 20%～30% 是未知返工风险，不是把正常测试漏到最后。

已有模块职责的复用判断可信度较高；R1 工程量可信度中低；R2/R3 尤其深层服务拆分可信度低。使用 AI 辅助可以提高部分实现效率，但没有本项目实测前，不按固定倍数缩短测试、集成和架构校验时间。

### 25.2 总量与 Rust 的占比

| 交付范围 | 基础工程量 | 加 20%～30% 风险余量后的预算区间 | 解释 |
|---|---|---|---|
| R1-JS：暂缓 native 正式支持 | 14～24 人周 | 17～32 人周 | 保守地仅扣除 P5；P0 改做内置 Rust/JS 契约验证，P2 保留；不额外乐观扣减 P0/P6 |
| R1：包含 Rust native | 17～29 人周 | 21～38 人周 | 首个有实用价值的开放平台预览，不是全栈终点 |
| R2 增量 | 19～34 人周 | 23～45 人周 | 模型/策略、Runtime、资源/lifecycle、Shell |
| R3 增量 | 12～21 人周 | 15～28 人周 | 部署级服务及所列目标平台收口 |
| **R1 + R2 + R3 主干累计** | **48～84 人周** | **58～110 人周** | 对基础合计统一加余量；分项四舍五入可能略有差异 |

P5 的 Rust 专属新增是 3～5 人周，占 R1 约六分之一的量级；这个比例只帮助判断重点，不代表每个实际项目都相同。公共协议/Provider 改造对 JS 本身也是必要投入，不能全部算成“为了 Rust 额外付出的成本”。

但也不能说“Rust 很便宜，只有一个 adapter”：native 的打包、版本、故障清理和用户信任边界都在 P5 内。若要求一开始同时支持三平台、强沙箱、完整 Agent Runtime，新增范围会明显超过 P5。

结论：**整体工作量确实大；Rust 不是让它翻倍的主要因素。** 优先控制首发范围及统一链路，比取消 Rust 或多开几个开发分支更能控制总成本。

### 25.3 推荐团队：两名后端 + 一名前端，集成职责明确

| 角色 | 主责任 | 关键配合边界 |
|---|---|---|
| 后端 A / 技术负责人 | contracts、Role/Provider、compiler/Kernel、Session 一致性；统一接口与合流 | P1/P2 单一接口 owner，P8/P9 外层 Runtime 契约负责人 |
| 后端 B / 运行集成 | Context/Skill、native adapter/SDK、process/资源接入、故障测试 | R1 同时有 P3/P5，需与 A 在 P2 完成后分担，不能假定两包由 B 同时满速完成 |
| 前端 / 产品集成 | Provider 选择体验、UI Host API 的前端部分、插件 Agent 页面、后续 Shell | 与后端 A 约定 Session API；不复制业务 store 或另建会话后端 |
| 测试/发布责任 | 模块 owner 自带测试，一名主责组织 E2E/发布 | 可由上述人员轮值；若另配 QA/平台工程师，其容量和成本单列，不当作免费劳动力 |

不建议一开始拆成“JS 团队”和“Rust 团队”各设计一套契约。应按公共内核、运行集成、UI/产品边界拆分，语言后端只是其中的实现任务。

### 25.4 三人团队的参考日历

本节日历按含 native 的 R1 编排；JS-only 不执行其中 P5/native 验证安排，使用 §27.3 的依赖和 §27.6 的规划窗口。

按约 70%～80% 的可排开发容量考虑 review、沟通与日常维护，另受关键路径制约，R1 可先按 **12～18 周日历**做预算窗口。不是把 17～29 人周直接除以三。

| 时间窗口（允许交叠） | 后端 A | 后端 B | 前端 | 里程碑 |
|---|---|---|---|---|
| 第 1～2 周 | P0 基线/替换部件与契约确认 | P0 native 协议/取消验证，核查 P1 剩余项 | P0 会话 UI API 验证，列现有入口缺口 | 第一次重估，明确首发范围 |
| 第 2～4 周 | P1 contracts/compiler 主链收口，启动 P2 | P1 app/运行集成，准备 P3 descriptor | P1 UI 类型合流；P2 选择界面 | 统一基线可用，不再让新功能依赖两套 Snapshot |
| 第 4～7 周 | P2 真实 Provider 替换与公共协议 | 配合 P2 JS host/adapter；随后启动 P3 | P2 产品入口，随后 P4 页面与 Bridge | 首个内置部件被用户实现实际替换 |
| 第 6～12 周 | P2 收口后承担 P5 主体与 app 接线 | P3 Context/Skill，支持 P5 Context 联调 | P4 插件 Agent 页面、故障/恢复交互 | 三条支线汇合；内部开发者可试用 |
| 第 12～14 周 | P6 联合修复/发布检查 | P6 native/Skill 故障验证 | P6 Web/桌面真实流程验证 | 条件满足时发布 R1 |
| 最迟预算至第 18 周 | 按实际风险重排，不新增范围 | 同左 | 同左 | 覆盖未知集成返工；超窗必须重估，不无期限顺延 |

窗口重叠表示契约稳定后允许后续工作开始，不表示同一个人能在多个包各投入 100%。若 P1 已由当前并行工程验收完成，计划可前移；若 P2 需要大改调用语义，则下游日期相应重估。

其他团队规模的 R1 日历参考：

| 持续投入人数 | 含正常协作与风险的规划窗口 | 说明 |
|---|---|---|
| 1 名全栈/后端主力 | 约 26～48 周 | 还要兼顾 UI；若不熟悉前端则更长，宜先发 R1-JS 或减少非核心 UI 范围 |
| 2 名互补开发者 | 约 14～26 周 | 并行度受后端共享文件与测试牵制 |
| 3 名上述配置 | 约 12～18 周 | 推荐的首期组队规模 |
| 4 名以上 | 约 10～16 周起评估 | 可以增加测试/平台专人，但 P1/P2 关键路径不能按人数无限缩短 |

以三人稳定团队看，R1～R3 主干累计可先按 **约 7～12 个月量级**理解，而不是把 R1 的窗口当全部完成时间；实际会受 R1 反馈、R2/R3 选定服务数量和平台条件影响。若要求一人承担完整目标，则是更长期工程，应该按版本分批投入。

### 25.5 最容易让预算失真的附加要求

| 附加范围 | 是否已经包含 | 处理方式 |
|---|---|---|
| Linux x64 / macOS arm64 native 与桌面发布 | R1 不含，P12 含所列组合 | 如果前移到 R1，就搬移相应预算和平台等待，不能要求首期同价提前完成 |
| Wasm 首个受限执行后端 | 不含 | 可先占 4～8 人周基础探索/接入预算；正式支持按 imports、异步与工具链 spike 重估 |
| 不可信 native/Node 插件的跨平台强沙箱 | 不含 | 是独立安全工程；可先留 8～16+ 人周探索/整合占位，但未选定 OS 机制前没有可靠总价 |
| 多后端同包编排、远程插件、在线构建服务 | 不含 | 单独提出产品需求和预算，不能塞入 native adapter |
| 任意旧开发数据库/历史协议全部无损兼容 | 不含无限兼容 | 按需要保护的真实数据确认一次性迁移范围；不能借此自动清空数据 |
| 全部内置功能逐个建立第二实现 | 不含无限展开 | 每个领域单独定义消费者与验收，按实际范围累加 |

上述附加数字同样是低置信度预算占位，不应直接相加得出合同报价。尤其强沙箱的威胁模型、系统支持范围不同，成本可能超出占位。

## 26. 如何启动、验收与控制范围

### 26.1 建议现在只下达第一批明确任务

**先启动 P0，并收口正在进行的统一改造；不要同时启动 P0～P12。** 本轮只是形成此计划，实际代码任务应另行下达。

P0 的建议执行顺序：

1. 固定一个可验证候选提交，记录当前统一工程已通过的测试和未完成项，保护工作区其他修改。确认开发数据处理范围，不在核对阶段做破坏式重建。
2. 从实际产品调用点选一个替换部件，写出“内置实现/用户实现分别何时被调用”的小测试；据此定义最小 Role/Provider 合同。
3. 用一个最小 Rust 程序验证宿主启动、握手、调用、超时取消、崩溃与回收；同一调用数据走一次 JS，验证公共协议没有 Node 私有对象。这个小样不是正式 SDK 或插件发布完成证据。
4. 用现有 Session API 验证插件页面所需的发送、事件、取消、历史；列出真正缺失的 Bridge 能力，避免先造大而全 UI SDK。
5. 给出 P1 剩余工作、P2 接口草案、首发支持矩阵和更新后预算。P0 总投入控制在 1～2 人周；并行资源不足时缩减验证范围，不把它扩大为完整新平台。

JS-only 情景用“内置 Rust adapter 与用户 JS Provider 的相同输入/输出、错误、取消契约测试”替换第 3 步，不开发新的 native 入口；其他步骤保留。

这些短验证通过后，启动 P1/P2；P2 契约稳定后再把 P3/P4/P5 并行安排。P0 小样能直接转为测试或示例就保留，否则删除，不长期留下第四条生产路径。

### 26.2 每包使用同一份轻量任务卡

只需在现有任务跟踪中写清以下内容，无需新建一套证据平台：

- 负责人、允许修改的模块和与其他任务共享的接口；
- 一个可演示的产品结果、明确不做项；
- 前置契约/版本、进入条件；
- 必须退出的旧分支或重复事实；
- 能证明行为的定向测试、人工/真实集成步骤；
- 合流方式、失败时撤销该切片的办法、当前剩余估算。

如果使用多名开发者或 AI 辅助，也按这些职责边界拆任务；不要让多个执行者同时重写 contracts/compiler/app composition。测试与接口 review 由具体 owner 负责，而不是假设增加并行数就自然获得质量。

### 26.3 R1 必须看到的结果

1. 一次真实产品调用分别绑定内置/JS/native 实现，调用记录能确认选中者；不选择的实现不被暗中调用。
2. 插件 Skill 只存在于包内，也能按锁读取正确正文与引用资源；同名来源不混淆；不支持的执行模式提前报错。
3. 用户 Context 真正影响传给模型的输入；不是 Catalog 中有一个条目而 runtime 忽略。
4. 插件 Agent 页面完成发送、流式观察、取消、历史和重载恢复；恢复界面不重发已有操作。
5. 禁用、升级、超时、进程崩溃的行为明确，资源和子进程可回收；旧会话不悄悄改绑新实现。
6. 现有 Nomi/JS/Service 基础流程仍可用；关键旧分支已退出；示例插件作者可以按文档独立构建、安装和调试。

R1-JS 的第 1 项只要求内置/用户 JS 两种实现；不要求 native 安装或故障验收。其余产品退出条件不因省略 Rust 插件而降低。

验证按风险选择：contracts/compiler 变更跑相关 Rust 契约/编译测试，Skill 跑精确装载测试，UI 跑交互与 Bridge 测试，native 跑真实子进程故障测试。跨模块集成或候选发布再跑相应更宽检查；不为每个局部修改要求全仓全平台检查。

### 26.4 三个固定复盘点

- **P0 后**：决定预算前提是否成立，确认 P1 实际剩余量、平台范围和首个替换部件。
- **P2 后**：用实际交付速度更新 P3～P6；若公共接口尚不稳定，不强行并行后续。决定完整 R1 一起发布还是先发 R1-JS。
- **R1 后**：按外部开发者真正受阻的环节排序 P7～P12；用真实数据细化 R2/R3，不继续沿用未经校准的长期人周估计。

若任一包剩余估算超出上界约 30%，或必须新增第二套 Catalog/Session authority、独立语言权限系统、全面 Node Host 重写，暂停扩大该包并重估方案。优先调整范围/拆包/删除不必要抽象，不通过降低验收或无期限延期维持原计划数字。

### 26.5 开发安排的最终建议

以下保留含 Rust 路线的安排；当前讨论的 JS-only 情景以 §27 为准，不要求先完成 Rust 小样或 P5。

推荐采用 **三人核心团队、先做 P0/P1/P2、R1 控制在 12～18 周规划窗口**的安排；正式投入前以 P0 的实测校准。Rust 不单独成立另一条平台主线，而是在公共边界稳定后交付 P5。

资源较少时先发 R1-JS，或者只增加少量角色投入测试/发布；不要同时开展 Wasm、强沙箱、全部内部策略、完整 Shell 和深层存储替换。完整目标保留，但按真实可用版本逐步推进。

这不是把大项目说成小项目，而是让每一笔投入都产生可验证的用户能力，并能在 R1/R2 之后自主决定下一阶段投入，而不必等待“全栈全部完成”才获得价值。

## 27. 回到原始五项需求：是否真正解决，以及 JS-only 路线的完成标准

### 27.1 直接结论与逐项对应

**完整设计针对这五项需求，但不能说“第一版全部解决”，也不能说“已有工作包已足够精细地覆盖所有环节”。** 本轮核对发现：P7/P8/P11 仍是大范围占位，工具发现、记忆、规划、协作、观测和部分 UI 的责任与验收此前不够明确。本节补齐对应关系，不把研究结论当作实施完成。

暂不增加 Rust 插件，主要放弃的是用户分发预编译原生实现的便利，不是放弃 JS 插件对 Agent 业务部件的替换能力。仍需修改 Rust 宿主的实际调用边界；“不新增 Rust 插件”绝不等于“不改 Rust 代码”。

| 原始需求 | 架构上的解决方式 | R1-JS 能完成什么 | 完整需求仍需什么 |
|---|---|---|---|
| 1. 用户 capability 与内置并存并可选用替换 | 独立 capability ID、共同 Role 契约、显式 Provider 绑定；自由工具与固定部件替换都支持 | P1/P2：统一目录与选择，至少一个真实内置部件被用户 JS 实现替换 | P7/P8/P9/P11：把后续每个部件的消费者接入绑定；用户自定义契约也能发布/消费；不能以一个样例宣称全部内置可替换 |
| 2. 用户替换系统 UI | UI contribution + 页面/区域/Shell 契约 + 一套公开应用 API | P4/P6：完整 Agent 页面，不只是装饰面板 | P10：Shell、主题/结果呈现及有独立意义的页面区域；独立客户端验证；桌面特权通过 P11 的部署接口，不等于 iframe 接管桌面容器 |
| 3. 全环节进一步插件化 | 内置与插件共同消费粗粒度接口，JS 使用 DTO/流/资源 handle | P2/P3：第一个部件、Context、Skill；Tool 原路径统一 | 按 §27.2 的 29 行逐个闭环；P7/P8 开放内部组件，P9 开放整个 Runtime，P11 处理部署服务；Rust 后端不是前置条件 |
| 4. Skill 装载链未闭合 | 锁定制品 → 完整 descriptor → 现有 Nomi Skill 装载/执行器 | P3：包内正文、只读引用资源、索引/按需加载和支持的 Tool 依赖 | P7：shell/fork/hook 等剩余执行模式及授权；R1 若暂不支持须明确拒绝，不能把部分模式完成说成全部完成 |
| 5. 更简单、开放、稳定的组装 | 一套 Catalog、一套绑定解析、一份冻结执行计划、一个 Session 执行 owner | P1/P2：用户选择后“保存并使用”，默认继承和真实调用一致 | P8/P9：完整 descriptor、等价校验、退出重复求解与硬编码调用；P7/P10/P11 不得再各造一套组装系统；按 §27.5 验收简单性 |

因此，**首发交付的是“可替换平台的真实闭环”，R2 才覆盖主要 Agent 业务环节，R3 才继续覆盖部署级服务。** 每个版本都应公布尚未开放项，不把未来版本承诺算进当前能力。部署级替换和最小信任根的特殊边界见 §27.4。

### 27.2 全部 29 个环节与工作包逐一对账

下表是 §16.2 的任务/验收补充，不是第二套架构，也不新增平行注册中心。编号仅用于需求跟踪。所有“验收”都是待实施标准，不表示当前已通过。

**每行都要验证公共声明/准入、用户绑定、实际消费者、故障行为和旧分支退出。** 可共享契约测试基础，但不能用一行成功推断其他行成功；若某行延期，应明确登记为未完成。表中 P2 等先交付基础、P7/P8 等补全的行，不能只完成前半段就关单。

| 编号 / §16 环节 | 对应工作包 / 阶段 | 该环节的实际完成证据 |
|---|---|---|
| 01 Catalog / 发现 | P1/P2：统一目录；P8：发现策略（R1/R2） | 内置/用户贡献同目录可见；可选发现/排序实现，排序不绕过准入或授权 |
| 02 Preset / 依赖 / 模板 | P2：已有契约绑定；P7：用户契约与模板发布（R1/R2） | 自定义命名空间契约能被另一插件依赖；模板一键组装但不携带额外授权，缺失依赖可解释 |
| 03 Session 创建 / 资源绑定 / 初始化 | P2：统一计划；P7：可扩展绑定与初始化（R1/R2） | 新 resource kind 和初始化贡献能实际运行，失败可清理；不依赖固定 kind 白名单或重复资源 owner |
| 04 初始 Tool | P1/P2/P6（R1） | 内置、用户 JS、发布型 Service 经统一入口调用，未选中实现不被暗中调用 |
| 05 on-demand / ToolSearch | P8（R2） | 用户搜索、排序、激活策略改变实际候选/调用路径；激活范围仍在已授权计划内 |
| 06 Context / persona / system prompt | P3：Context；P8：完整 Prompt 管线（R1/R2） | 实际模型输入可捕获并核对来源/顺序；替换整条管线后无隐藏内置 Prompt 拼接 |
| 07 Skill | P3：正文/资源；P7：剩余执行模式（R1/R2） | 包内独有 Skill 被按锁装载；同名不串用；脚本/fork/hook 的执行与取消、权限行为有测试 |
| 08 MCP | P7（R2） | Server 连接与 mapping 通过同一能力/资源路径使用，无双重注册，断连和释放可验证 |
| 09 ResourceProvider | P7（R2） | 自定义 kind、多个命名 binding、列举/获取/释放与撤销闭环，资源实现可更换 |
| 10 发布型 Service 资源（原 M1） | P7（R2） | 带资源 Service 真正接收并消费授权 handle，禁用/撤销生效，不再仅支持无资源子集 |
| 11 Browser / Computer 等 Role | P2：通用绑定；P7：领域消费者（R1 基础/R2 完成） | 分别验证用户 JS Provider 执行真实浏览器/桌面动作并接替内置 owner，权限/状态生命周期明确 |
| 12 Turn Middleware | P7：阶段接口；P8：引擎消费（R2） | §16.4 声明的阶段均有真实消费点；有序 patch、失败、取消、重入得到验证，不是任意可变引用回调 |
| 13 EventSource / EventConsumer | P7（R2） | 用户事件源与消费者都能工作；订阅、背压、取消和敏感字段过滤可验证 |
| 14 Lifecycle / BackgroundService | P7（R2） | 安装/Session 作用域 start/stop/dispose、健康与恢复真实接通，不遗留子进程或租约 |
| 15 Transport / Channel | P11（R3） | 替代 Channel 收发、附件与身份映射进入同一 Session API；停用后不重复消费消息 |
| 16 Scheduler / Cron / Automation | P11（R3） | 触发器、调度策略与任务执行器有明确接口；替代实现的重入、取消、重启恢复有真实验证 |
| 17 模型 Provider / 路由 | P8（R2） | JS 模型实现参与真实会话的流式文本/工具调用、取消与错误路径；路由只选锁定候选，不硬编码内置模型 |
| 18 上下文选择 / 历史 / 压缩 | P8（R2） | 用户管线与压缩器改变请求上下文；历史视图与持久化事实分离，预算与恢复行为清楚 |
| 19 长期记忆 / RAG / embedding | P7：资源；P8：策略（R2） | 记忆写入/读取、检索、embedding 实现可分别绑定；索引资源、来源与删除有验证，不只是新增一个搜索 Tool |
| 20 Planner / 推理循环 / 停止策略 | P8：Nomi 组件；P9：整个循环（R2） | 在 Nomi 内可替换规划/停止策略；替代引擎另走 Runtime 契约；两者均可取消，预算控制不失效 |
| 21 工具编排 / 并行 / 结果转换 | P8（R2） | 自定义编排改变真实调度与结果消费；副作用工具不因不明结果被隐式重试，插件不绕过授权 |
| 22 子 Agent / 多 Agent 协作 | P8：委派/协调；P9：跨 Runtime 联调（R2） | 用户协调策略实际创建/管理子任务，授权不扩大，预算/取消传播和结果归属清楚 |
| 23 整个 Agent Runtime / Loop | P9（R2） | 一个真实 JS 外部 Runtime 可替代 Nomi，统一发送/事件/取消；无暗中回到 Nomi；恢复能力显式声明 |
| 24 日志 / tracing / 评估 / 计量 | P7：事件出口；P8：评估消费（R2） | 用户 exporter、evaluator、meter 能消费授权数据；慢/失败观察者不拖住 turn，非权威观察结果不能改写安全审计 |
| 25 UI / Agent 页面 / Shell | P4：Agent 页；P10：其余 UI；P11：桌面服务（R1/R2/R3） | Agent 页及 Shell 真正替换；主题、工具卡片/附件呈现、已定义区域可独立贡献/替换；恢复入口和公共 API 功能对等 |
| 26 Session / 事件 / checkpoint 存储 | P11（R3） | 分别按一致性要求验证替代后端，含事务/顺序/恢复与维护窗口迁移；不要求不同引擎 checkpoint 格式相同 |
| 27 Preset 编译策略 / Registry 后端 | P2：统一绑定；P8：选择策略；P11：目录存储（R1/R2/R3） | 插件可提出组装/选择方案并经过同一最终校验；Registry 存储可换，不引入第二个 Catalog 权威 |
| 28 认证 / 授权策略 / Secret provider | P11（R3，部署者选择） | 三类接口分别验证第二实现与启动配置；失效行为明确，插件不能自行审批授权或读取他人密钥 |
| 29 supervisor / sandbox / Kernel 宿主 | P11/P12（R3，部署级且有例外） | 可换监督/隔离后端在已选平台有实际测试；整 Kernel 替换另属宿主实现，不计作普通 JS 插件完成；强沙箱产品化另估 |

相较旧工作包，本轮明确补入：01/05 的发现与激活策略、02 的用户契约/模板、11 的 Browser/Computer 消费、19/20/22 的记忆/规划/协作、24 的观测评估、25 的 UI 子范围、27 的选择策略与目录存储。不是新增用户需求，而是此前计划粒度不足。它们不能继续藏在“等策略”三个字里。

P7/P8/P10/P11 启动前应按上表拆成有 owner 的小任务；共用 DTO、事件、资源与测试设施，避免一行一个新框架。内置默认组件也必须经过对应逻辑接口；为性能保留进程内 adapter 可以，绕过选择规则的内置特例不可以。

### 27.3 暂不新增 Rust 插件：工作怎么改，而不是目标怎么缩水

本情景只保留现有 **Rust 宿主/内置 adapter + JS 插件后端 + UI Surface**。不新建 native-process 后端、Rust 作者 SDK、原生插件制品目标管理或 Wasm 后端。

| 工作包 | JS-only 安排 |
|---|---|
| P0 | 保留基线确认、真实替换部件、UI API 验证；用内置 Rust/JS 同契约测试代替 native 小样，不要求 Cargo 示例 |
| P1/P2 | 全部保留；统一 Catalog/绑定与语言无关 DTO 本来就是 JS 替换 Rust 内置组件所需，不是为 Rust 插件预建平台 |
| P3/P4 | 全部保留；Skill、Context、完整 Agent UI 与 Rust 插件无依赖 |
| P5 | 移出本情景范围；不以 stub、空 crate 或未使用入口形式偷偷保留 |
| P6 | 依赖改为 P3/P4；验证内置/JS 与现有 Service 路径，不等待 native 安装、升级或发布 |
| P7/P8 | 按 §27.2 开放 JS 消费接口；Rust 内部 trait 需在宿主适配为结构化异步接口，不把内部对象直接跨进程暴露 |
| P9 | 保留完整 Runtime 替换，验收第二个实际 JS Runtime；不是“JS 只能写 Tool，所以本包取消” |
| P10/P11 | UI 与部署服务拆分保留；可被 JS 实现的服务通过公开 port 接入，必须早于插件加载的部分按 §27.4 处理 |
| P12 | 仍验证所选 OS 上 Rust 宿主、Node/JS、UI 和 Runtime；删除新增 Rust 插件的目标制品/SDK 验证，不误删宿主桌面签名与发布工作 |

首期依赖收敛为 `P0 → P1 → P2 → P3/P4 → P6`。P2 稳定后 P3/P4 才能并行；无 P5 前置。R2/R3 不得因“以后可能加 Rust”而等待原生后端。

优雅性的关键不是把所有调用变成 JSON-RPC，而是提取**有业务意义的粗粒度边界**：模型请求及有界流、一次上下文构造、一个资源租约、一轮调度决策。公共类型尽量从已有 canonical contracts 生成；一套错误/取消语义；不跨边界传 Rust 引用、不持引擎大锁等待 JS、不逐 token 同步往返、不另建 JS 专用组装器。

JS-only 的实际损失：纯 Rust/native 库不能作为新类型插件直接打包安装，某些高频计算或设备集成可能不适合该执行形态。可以按实际需要使用现有公开服务或宿主受控代理，但新增代理仍要计入工作量；不能临时允许插件任意启动二进制，变相绕过被延期的 native 制品与生命周期设计。确认有无法接受的功能/性能差距后，再单独评估 P5。

### 27.4 “任意部件”的边界必须诚实，不把部署替换冒充普通插件替换

区分三个层级，产品能力清单必须标明所属层级：

1. **普通可安装 JS/UI 插件**：Tool、Context、Skill、资源、middleware、模型、规划/记忆/编排、整个 Runtime、Agent 页面和 Shell。目标是用户安装、选择后直接使用；不要求用户重编译宿主，也不要求改官方源码才能替换。
2. **部署者选择的服务实现**：Channel、Scheduler、存储、认证/Secret、监督/隔离后端等。其中一般业务服务可做成安装级 JS 服务；涉及启动顺序、持久化权威或权限根的服务需在启动/维护窗口选择，由独立 bootstrap 加载和监管。接口应公开，但是否可通过同一插件安装入口交付，要在 P11 分项证明，不能一概宣称普通插件已经可换。
3. **最小 bootstrap 与最终权限/状态仲裁本身**：约束插件的最后一层，不能由被约束插件自行取消。替换它通常是另一个宿主实现/发行版。公开协议可以支持这种可移植性，但这不等于当前宿主内的插件功能，也不在 P11/P12 粗估中承诺另写一套完整 Kernel。

这些区别不是把存储、认证等永远写死的理由。P11 应拆开“策略实现”和“最终执行/提交”：插件可提供认证、授权策略或候选计划，宿主按部署者配置调用，最终结果仍由唯一权威落实。早期启动服务不能依赖尚未装载的 Catalog/Session，避免“必须先读插件数据库，才能启动插件数据库实现”的循环依赖；启动配置只选实现和引导凭据，不发展成第二套运行期目录。

因此，如果目标严格解释成“每一行都能以普通 JS 插件安装，甚至替换约束自己的信任根”，**目前方案不能声称全部满足**；增加 Rust 插件同样不能消除这个安全与所有权矛盾。部署开放与普通插件开放应分别验收，不以“可以 fork 项目”交差。也不默认承诺会话中途热换引擎、存储或设备 owner。

### 27.5 简单性也是验收项，不是只有功能清单

组装体验应是：**安装插件 → 选择实现/使用模板 → 保存并使用**。已有授权可继承时不逐阶段确认；只有新增权限或确有风险的操作才需要额外确认。Preview 是解释与诊断能力，不是用户每次必须手工执行的步骤。

以下条件共同约束架构，避免为开放性叠加框架：

- **一套事实**：一个 Catalog、一份 Role/Provider 绑定解析结果、一份精确制品/资源执行计划；UI、conversation、Runtime 不各自猜测默认实现。资源 lease 等运行状态仍有各自明确 owner，不强塞入静态快照。
- **一套用户选择语义**：Agent override > 安装默认 > 官方默认；集合按用户可见顺序；错误可追溯到具体契约/实现。不同语言、UI、资源不再创建专用配置链。
- **旧硬编码退出**：已开放部件的生产消费者只依赖契约，不直接调用内置 ID；内置和用户实现都能用同一消费测试验证。不能只把新路径加上而永久保留两条生产主链。
- **编译与运行各司其职**：保存/创建时解析并锁定；运行时消费计划。P8/P9 在 descriptor 完整、等价校验覆盖后退出重复求解，但保留当前撤销状态、权限、资源有效性检查；“少一层”不等于跳过安全校验。
- **一个 Session 执行 owner**：外部 Runtime 不同时被 Nomi Loop 二次调度。共享公共状态/API，不强迫不同引擎实现 Nomi 的内部 hooks。
- **不强迫作者走长链**：独立 Tool 不必先写 Role；默认由 manifest/SDK 生成重复样板；只为有替换需求的组件制定契约，不让每个 helper function 都插件化。
- **能力不被静默忽略**：所选 Runtime 必须消费已声明支持的贡献；不支持的 middleware/Skill 模式在组装时明确提示。不能“装得上、选得上、运行没效果”。
- **故障与性能可度量**：每个异步边界验证超时/取消/清理，记录启动、首响应、IPC 次数/负载和内存相对基线；按 P0 实测设可接受阈值，不凭空保证零开销。

P6 验收首条闭环；P8/P9 验收执行计划与引擎切换；P10/P11 验收新增消费者未复制组装和授权逻辑。删除有重复事实的链路比单纯减少函数/模块数量更重要。

### 27.6 工作量口径：哪些数字还可用，哪些必须重估

| JS-only 范围 | 基础工程量参考 | 当前可如何使用 |
|---|---|---|
| R1-JS | 14～24 人周；含 20%～30% 余量约 17～32 人周 | 范围仍按 §23.2 的首条纵向闭环；P0/P2 后校准，不新增“所有环节”验收 |
| 到 R2 的原主干累计 | 33～58 人周（14～24 + 19～34） | 旧工作包算术参考；不是本轮 29 行细化后的全量承诺 |
| 到 R3 的原主干累计 | 45～79 人周（再加 12～21） | 同样只是扣除 P5 后的低置信度主干参考，不含另写 Kernel/强沙箱产品化 |

这里只保守扣除 P5 的 3～5 人周。P0/P6/P12 确有部分 native 专属工作不再执行，但有其他集成工作保留，在拆项前不再乐观扣减，避免重复计算收益。

按两名熟悉 Rust/运行集成的后端加一名前端，R1-JS 可先按 **约 10～16 周日历**安排预算窗口，受 P1/P2 关键路径约束，不是人周直接除以三；若当前并行统一工程已验收，按实际剩余量扣除。已有代码改动数量不代表 P1 完成。

需要修正此前整体估算的解读：**“不加 Rust 后主干约 45～79 人周”不等于“原始所有环节完整满足只需 45～79 人周”。** §27.2 补明确的项目此前未逐项定量，可能使 P7/P8/P10/P11 超出原区间；20%～30% 风险余量也不能拿来吞掉明确的范围缺项。全量开发日历在这些包拆卡前只能称中长期项目，不能继续把粗略半年到一年当成全目标保证。

P0 建立 29 行覆盖台账，P2 固定公共边界，R1 后逐领域拆包重估；每项记录“已实现并验证 / 部分完成 / 未开始 / 部署级 / 明确例外”及证据。没有范围改变，就不得默默删掉难做的环节；确要减少范围则单列产品决策。

### 27.7 开发决策建议

当前建议是：**先不新增 Rust 插件后端；先交付 R1-JS，同时把完整开放目标保留在逐项台账中。** 先投入 P0/P1/P2，证明真实系统部件能够被 JS 替换，再推进 P3/P4/P6；不把原生 SDK 当成开放性的前提。

后续用 P7/P8/P9/P10 完成 Agent 业务层与 UI，P11/P12 逐个完成部署级服务和已选平台。不以只发布接口、单个示范插件或存在 Rust trait 判定全部完成。是否补 Rust 插件，应由实际无法满足的原生依赖/性能需求驱动，而不是为了让架构图看起来对称。

对本次问题的最终回答是：**五项需求都有相应设计路径；首期只部分解决；旧计划的若干后续环节尚未细化，本节已补入任务和验收，但仍需重新估算。普通插件不能替换最小信任根的例外也必须明说，不能拿部署/重编译替换冒充插件替换。**

## 附录 A. 按目标要求完成统一的实施基线

本节是后续代码实施的固定边界。它不是兼容迁移方案，而是产品重构阶段的
“破而后立”方案。

### A.1 Canonical 领域实体

活动代码只允许使用以下产品实体：

- `PluginProductId`：用户看到和管理的插件产品身份；
- `PluginProjectId`：插件源码/工作区身份，安装型和发布型插件共用；
- `PluginReleaseId` / `PluginReleaseRef`：不可变发布物身份；
- `PluginMountId`：已安装 N1 package 的挂载身份，与产品身份明确区分；
- `PluginSurfaceSessionId`：插件 UI surface 会话身份；
- `PluginService`、`PluginSurface`、`PluginTool`：插件运行角色，而不是产品类别。

`PluginProduct` 可以同时拥有 Tool、Service、Surface、ResourceProvider 等贡献。
不能再用互斥的 `UiOnly` / `Service` 产品种类决定能力准入；产品能力应由发布
Manifest 的 contributions 和 role/profile 派生。

### A.2 Canonical Agent 模型

`ResolvedSnapshotContent` 只保留：

```text
initial_capabilities
on_demand_capabilities
skill_locks
mcp_tool_locks
resolved_role_providers
```

所有能力（N1 package、已发布 PluginProduct、平台内置能力、MCP 映射）都进入
同一个 `ResolvedCapability` 类型。发布型插件的 active release、release digest、
publication epoch、catalog digest 作为统一 provenance/execution profile 的字段，
不能再以 `ResolvedMiniAppCapability` 另开一套数组。

### A.3 Canonical 数据根

运行时表、repository、外键和 identity 统一为 `plugin_*`。目标包括：

```text
plugin_products
plugin_projects
plugin_releases
plugin_catalog_publications
plugin_surface_sessions
plugin_service_test_receipts
plugin_source_mutation_*
```

开发阶段按破坏式切换处理：不保留 `/api/miniapps`、旧字段别名、旧 CLI 或旧
协议别名；已有开发数据库可以重建。若未来需要保留真实用户数据，必须另行设计
一次性数据迁移，不能把兼容字段重新带回活动领域模型。

### A.4 实施顺序和闸门

1. **Contracts**：先完成 ID、provenance、Snapshot 和 generated schema；闸门是
   `ResolvedMiniAppCapability`、`MiniAppActiveRelease`、`miniapp_id` 不再出现在
   活动 Agent contract。
2. **Plugin platform / DB**：重命名并合并 M1 release、service、surface、storage
   聚合；闸门是运行时只依赖 `PluginProduct`/`PluginRelease` 和 `plugin_*` 表。
3. **App / Session**：合并 catalog sink、runtime state、route state 和 tool
   materialization；闸门是 Session 只有一个 Plugin capability materialization 入口。
4. **UI / API**：统一 ID parser、resource kind、bridge 协议、DTO 和用户文案；
   闸门是用户可见入口及前端活动类型不再出现 MiniApp。
5. **Docs / tests / generated fixtures**：重生成合同、预置 Agent、验收脚本和
   架构文档；闸门是 residual scan 只允许历史记录和明确的兼容 allowlist。
6. **最终验证**：执行 Rust workspace check、相关集成测试、前端 typecheck/build，
   并对活动代码运行大小写不敏感的 `MiniApp|miniapp|MINIAPP` 扫描。

任何阶段如果重新引入 `MiniApp` 兼容 alias、第二套 capability 数组或独立
MiniApp resource kind，都视为没有完成统一，而不是“暂时过渡”。
