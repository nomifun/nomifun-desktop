# NomiFun Agent Capability Platform v2 决策记录

> 文档状态：**CURRENT DECISION LEDGER / 2026-09-02**
>
> 权威顺序：`05-system-capability-replacement-foundation.zh.md` 是一期当前修订合同；本文与 05 冲突时，以 05 为准。
>
> 状态来源：`GLOBAL-CLOSURE-TODO.zh.md` 是当前实施进度、阻塞项和关闭状态的唯一来源。本文只记录架构决策及理由，不声明代码、Gate、平台验证或发布已经完成。
>
> 历史处理：保留 D-001～D-028 的决策编号、仍有效的结论和形成理由。已被 05 否定的旧要求只保留极短撤销原因，不再作为正文中的候选方案、实施步骤或验收合同。

## 状态约定

- `已确认`：结论继续有效，且未被 05 实质改变。
- `已修订（05）`：原决策问题和目标仍有效，但当前可执行合同已由 05 替换。
- `后续阶段`：方向仍有效，但不属于一期交付合同；必须等后续设计正式确认后实施。

## 决策总览

| ID | 决策 | 状态 | 当前有效结论 |
|---|---|---|---|
| D-001 | 产品与内部领域命名 | 已确认 | 产品称“Agent 设定”；内部使用 `AgentPreset` / `AgentPresetRevision`，运行实例使用 `AgentSession` |
| D-002 | Agent 执行模式 | 已确认 | 只保留 FullAuto；不保留审批、确认、Grant/Lease/Permit 状态机 |
| D-003 | 平台事实与 Runtime 所有权 | 已确认 | NomiFun 持有平台和业务事实；Runtime 只是通过受管 Host ports 执行 frozen Snapshot 的执行器 |
| D-004 | Codex 基座与 Nomi 替换 | 已修订（05） | 最终只有一个 Codex-derived Runtime，并完整保留 Coding 能力；Sidecar 先做 upstream spike，Nomi 在一次性 C9 中最终删除 |
| D-005 | 普通插件运行模型 | 已确认 | 普通第一方和第三方插件按 trusted in-process code 处理；不建设通用 sandbox/权限平台 |
| D-006 | Thin Kernel 与业务边界 | 已确认 | Kernel 只保留不可由普通插件自举的基础事实；业务域进入统一插件主链 |
| D-007 | Package/Capability/Skill/MCP 分层 | 已修订（05） | 保留四层语义；Browser/Computer 增加 canonical Role/Provider seam，所有消费者仍只调用 canonical Capability |
| D-008 | initial/on-demand 能力 | 已修订（05） | 保留两个集合和按需激活；Selection 只保留真实消费字段，不建设未执行的配置面 |
| D-009 | 官方 Agent 模板 | 已确认 | 保留角色型模板与业务边界；精确 seed 来自 canonical inventory，不以固定数量测试作为 Gate |
| D-010 | Agent 设定编辑体验 | 已修订（05） | 普通界面只展示产品语言、能力分组和资源 picker；Revision/Snapshot/digest 等进入折叠技术详情 |
| D-011 | 首个 Vertical Slice | 已修订（05） | `chat.minimal`、完整 `coding.codex` 和同链 fixture 仍是早期切片；一期完成按 05 的真实产品闭环判断 |
| D-012 | v4 数据代际 | 已确认 | fresh v4，不迁移 pre-v4 数据，不建设 Converter 或 dual read/write |
| D-013 | 旧数据目录处理 | 已确认 | 同文件系统 whole-root rename 后创建空 v4 root；归档不进入运行、恢复或兼容主链 |
| D-014 | Legacy 删除边界 | 已修订（05） | 开发期检查生产旧主链不可达，发布期检查 Nomi 制品不存在；不要求文档、测试和历史字符串全仓 exact-zero |
| D-015 | SessionEvent 与 Projection | 已修订（05） | 保留一套语义 SessionEvent；Projection 只保存 UI 终态，不复制完整 Event Log，token/delta 默认 transient |
| D-016 | 第三方插件正式产品化 | 后续阶段 | 一期只冻结 source-neutral 主链；用户安装、SDK、Extension Host、市场和切换 UI 留给二期 |
| D-017 | Remote 与 Agent 设定映射 | 已确认 | `RemoteBinding` 复用 canonical Agent binding；显式 `open/turn/observe/cancel` 和 `agent_session_id` |
| D-018 | 轻量 Chat 与完整 Coding | 已确认 | 用结构正确性保证轻量 Chat 和完整 Coding；不建设性能 benchmark、统计质量或性能 RC |
| D-019 | 实施并行与估算 | 已修订（05） | 不再固定五流、ROM、Agent 数或周数；按当前 TODO、独占写集和真实依赖动态并行 |
| D-020 | Nomi 最终删除门禁 | 已修订（05） | 不做在线 sticky canary；release-required 主链就绪后执行一次性 C9，再做三平台正式 RC |
| D-021 | Conversation 与 Session 身份 | 已确认 | 新架构只有 `AgentSession/AgentSessionId` 一个产品会话 aggregate 和 UUIDv7 主键 |
| D-022 | Test Revision 与真实 Effect | 已修订（05） | Test 走普通 Revision/Session；Effect 只分 `read_only`、`managed_effect`、`external_uncertain_effect` |
| D-023 | 官方模板 Seed 政策 | 已修订（05） | 保留 role-complete/context-minimal；不以固定模板或 Capability 数量生成结构 Gate |
| D-024 | Session 删除 | 已修订（05） | 简化为 `live → deleting → dispose/kill → 幂等删除 → minimal tombstone` |
| D-025 | Snapshot 可执行性 | 已修订（05） | 只保留一个 canonical Compiler；Snapshot 只冻结实际执行闭包，结构不兼容时只读并显式 fork |
| D-026 | Remote token rotate/revoke | 已修订（05） | 原子 generation/hash 是 admission fence；不跨 Response Body 持锁，不增加 grace/worker/Session 索引 |
| D-027 | Nomi 排空 | 已修订（05） | 原在线 drain 设计撤销；改为一次性 C9 bounded shutdown、进程树清理和必要的 uncertain 标记 |
| D-028 | 发布平台矩阵 | 已修订（05） | 首发阻断平台为 Windows x64、macOS arm64、Linux Desktop x64；macOS x64/Linux Headless 后续交付 |

## 全局有效约束

1. **交付速度和逻辑简单优先。** 新抽象必须直接减少主链分支、重复事实、状态机或调试面。
2. **只有一份 canonical 机器事实。** Rust、SQL、schema 和行为测试是实施后的事实源；文档不复制第二套需要逐字段同步的合同。
3. **不预建没有真实消费者的系统。** 新 DTO、状态机、coordinator、全局 digest、跨平台笛卡尔积或 fixture 必须由当前产品行为证明必要。
4. **权限只保留最小同步检查。** Principal/ownership、Capability allowlist、typed resource binding、Remote ingress authentication 和 provider credential route 可以存在；审批平台、动态授权和通用 policy engine 不进入一期。
5. **Runtime、Plugin 和业务域不得绕过 Snapshot。** Runtime 不扫描全局服务，业务消费者不直接取得具体 Browser/Computer 后端，Gateway 不重建第二条执行主链。
6. **验证与风险成比例。** 日常变更使用最小定向检查，主要合流和 RC 才运行 broad checks；环境或 harness 障碍记录一次并转人工，不盲目重试。

## 决策正文

### D-001：产品与内部领域命名

- 状态：`已确认`
- 产品对象统一称 **Agent 设定**。
- 内部可编辑对象为 `AgentPreset`，不可变版本为 `AgentPresetRevision`。
- 产品运行实例为 `AgentSession`，不使用“系统 Agent”或 `AgentDefinition` 指代完整设定。
- 产品不提供 Runtime/Engine catalog；执行实现是内部基础设施。

理由：设定、不可变版本和运行实例具有不同生命周期。分开命名可以避免把用户配置、Runtime 进程和产品会话混成一个对象。

### D-002：Agent 执行模式

- 状态：`已确认`
- 产品只保留 FullAuto。`YOLO` 只能作为历史或研发别名，不能成为第二套机器合同。
- Agent 只能调用 Snapshot 已冻结的 Capability、Tool 和 typed resources；范围外调用明确失败。
- v4 不保存 approval、confirmation、permission mode、Grant、Consent、Lease 或 Permit。
- 未来若出现真实审批需求，必须作为独立产品需求重新设计，不能预埋等待状态。

理由：能力范围已经由 Preset、Compiler 和 Snapshot 决定，再增加审批状态机会形成第二套权限事实和大量不可恢复的等待分支。

### D-003：平台事实、业务数据与 Runtime 的所有权

- 状态：`已确认`
- Capability、AgentPreset、SessionEvent、Snapshot、Secret 引用和业务数据都由 NomiFun 持有。
- Codex-derived Runtime 只通过受管 Host ports 使用当前 frozen Snapshot 的 Model、Context、Tool 和 Event。
- Runtime 不得直接取得全量数据库、Secret、`AppServices` 或任意业务 service bag。
- Knowledge、Memory、Companion、Channel、Browser、Computer、Robot 等领域继续拥有自己的业务事实。
- Runtime thread/rollout/checkpoint 只是可丢弃或可重建的执行绑定，不是产品 Session 主键或历史事实源。

理由：Runtime 可以升级、崩溃或替换，而产品事实必须保持稳定。把权威数据留在 NomiFun 可以避免 Runtime 私有格式支配业务模型。

### D-004：Codex 基座、Sidecar 与 Nomi 最终替换

- 状态：`已修订（05）`
- 最终产品只有一个 Codex-derived Runtime，不提供 Engine 选择器，也不长期保留 Nomi fallback。
- `coding.codex-native` 必须尽可能保留 Codex 的 workspace、AGENTS、Git、文件、patch、shell/PTY、Skills、MCP、计划、子 Agent、review、恢复和原生 Responses 语义。
- 非 Coding Agent 使用同一 Runtime 的精简 Profile，关闭未选择的 Coding 能力，而不是引入第二个 Engine。

Sidecar 当前合同：

1. 先对官方 Codex app-server 的 initialize/version、thread/turn、cancel 和 event 协议做真实 upstream spike。
2. 一个 Runtime binding 当前独占一个受管进程；结束时先关闭协议，再由 Host 清理整棵进程树。
3. Host-managed Tool 到达 Host 后直接按 Effect 策略处理并返回，不要求额外的 `native_action/start` RPC。
4. 只有 upstream callback/Tool seam 无法提供 Codex-native file/shell action 所需的最小调用前通知时，才允许一个窄 patch。
5. hello 只校验协议 major、build identity 和必要 feature，不镜像整个产品合同。
6. 是否保留浅 fork 必须由 spike 结果决定，不能因旧 Host adapter 已存在就倒推自定义 RPC 必须存在。

原“先建设三项自定义 Sidecar RPC 和完整 fork 合同”的要求已撤销，因为它依赖不存在的 patch source，并把外部假设变成一期硬阻塞。

理由：目标是获得成熟 Coding Runtime，同时把依赖和进程故障与桌面 Host 隔离；不是为了维护一套比 upstream 更大的私有协议。

### D-005：普通插件运行模型

- 状态：`已确认`
- 普通第一方和未来第三方插件统一按 trusted in-process code 处理。
- 一期不建设 WASI Host、通用 subprocess ABI、sandbox、签名链、供应链信任、插件权限引擎或多层 Secret Broker。
- Package manifest 只声明加载、版本、依赖、配置、贡献和生命周期所需事实。
- Codex Runtime 继续使用 Sidecar，是为了复用 upstream 产物、隔离依赖和治理进程树，不是插件安全策略的例外层。

理由：当前阶段需要的是一条可调试、可组合的插件主链。为尚未交付的第三方生态预建隔离系统会显著增加 RPC、状态和跨平台成本。

### D-006：Thin Kernel 与业务域边界

- 状态：`已确认`
- Kernel 只保留 App bootstrap、SQLite/migration、最小 ownership、canonical Compiler、Capability Registry、AgentSession authority、Runtime client/supervisor、Model Broker、最小 Remote auth、基础 EventBus 和 Plugin Manager。
- Knowledge、Memory、Companion、Browser、Computer、IM/Channel、Customer、Robot、Creative、Requirement、AutoWork、Cron、IDMM、AgentExecution、SSH、Office、Webhook 等业务能力属于插件或具名领域 owner。
- Composition Root 可以构造基础设施，但不得重新形成业务型 God Service。
- 迁移完成后，`AppServices` 业务 service bag、`GatewayDeps` 大型 capability 入口和旧 Factory 手工组合应从正常运行图删除。

理由：Kernel 只应持有无法由普通插件自举的共同事实。业务留在 Kernel 会让每个新能力继续修改中央装配和所有入口。

### D-007：四层领域与 Browser/Computer Role seam

- 状态：`已修订（05）`

四层语义继续有效：

- `Package`：安装、版本、依赖和分发单位。
- `Capability`：AgentPreset 可选择并由 Runtime 调用的稳定可执行能力。
- `Skill`：模型可读的说明、工作流和资源；自身不是执行器，不能自动扩张 Snapshot。
- `MCP`：外部 Tool 来源与传输；物化后的 Tool 进入统一 Capability 主链。

Browser/Computer 的一期增补合同：

1. 稳定系统能力角色为 `system.browser_use` 和 `system.computer_use`；它们不是 Agent Persona，也不进入用户 Capability 目录。
2. `browser.*` / `computer.*` 继续是唯一 canonical Capability façade，Provider 不注册或抢占同名 ID。
3. Package 通过 source-neutral `RoleProviderContribution` 提供具体实现；exact identity 包含 Role contract、Package、Mount 和 contribution digest。
4. installation default binding 与 Agent Revision override 在 Snapshot 创建前选择 Provider；不存在 override 时继承 installation default，不增加 `latest/follow` 状态。
5. `ResolvedRoleProviderLock` 冻结实际 Provider、合同、来源、成员和资源引用，并参与 Snapshot digest；non-Agent operation 在 admission 时取得同一种 exact lock。
6. `RoleDispatcher` 是 Kernel Registry 内的一条 exact route。Tool、ContextContributor 和 ResourceProvider 都读取同一个 frozen Provider lock，并使用 Provider Mount 的 config/state/service view。
7. 第一方 Provider 必须经过与 alternate fixture 相同的 registration、materialization、index、resolver 和 dispatch，不得使用 built-in shortcut。
8. Browser 的 owner/lane/profile/cancel/close 与 Computer 的 target 级单次 action 串行语义继续保留。
9. Knowledge `browser.render_content`、Gateway Browser/Computer 和 computer stdio 不得直接调用具体 Hub、Registry 或 Tool。
10. Provider 缺失或不兼容时 typed fail；不按来源、安装顺序或健康分自动选择，也不静默回退第一方实现。

一期只冻结 Role/Provider 机器接缝和第一方 dogfood。Node Plugin、MCP Adapter、CLI Provider、用户切换 UI、市场和 Chat Dev 属于二期。

理由：如果一期仍让系统消费者直接认识第一方 Browser/Computer，二期“可替换实现”就必须再次修改 Compiler、Snapshot、Gateway、Knowledge 和所有业务消费者。窄 Role seam 可以避免第二次主链重构，又不引入通用 Provider graph。

### D-008：initial/on-demand 能力范围

- 状态：`已修订（05）`
- Preset 保留 `initial_capabilities` 与 `on_demand_capabilities` 两个集合。
- Compiler 在创建 Snapshot 时解析两个集合；Runtime 只能激活 frozen on-demand ceiling，不能从全局 Catalog 扩权。
- initial 进入首轮 Tool/Context；on-demand 只保留紧凑索引，并在真实使用时 lazy acquire 对应 Provider/resource。
- Capability Selection 首版只保留 capability ref、action allowlist 和 resource binding refs。
- `required`、`exposure`、destination constraints、budget override 和未传入 Handler 的 config 在出现真实执行语义前删除。
- initial/on-demand 由所在集合表达，不复制第二套字段。

理由：两个集合足以同时保证可复现范围和较小上下文；额外字段如果不改变执行，只会扩大 schema、UI 和兼容成本。

### D-009：官方 Agent 模板及业务边界

- 状态：`已确认`
- 已确认的角色型 seed 包括轻量问答、通用助理、Coding、伙伴、Robot、客服和创意工坊。
- Research 是可复用 Capability Pack；Requirement、AutoWork、Cron、IM/Channel 和 Remote 选择或触发 exact AgentPreset，而不是创建专属 Agent 类型。
- Browser、Computer、Knowledge、Memory、MCP、SSH、Office 和 Webhook 是 Capability/Package/resource，不是 Persona。
- 精确 Capability ID、binding 和 initial/on-demand partition 由 canonical seed inventory 维护。
- Catalog 测试验证 ID 唯一、依赖闭合和关键角色可运行，不把固定模板数或 Capability 数量当作发布证明。

理由：模板只表达真正不同的 Persona 和开箱体验。为每个 transport、workflow 或技术能力建立 Agent 类型会复制模型、资源和能力配置。

### D-010：Agent 设定编辑器与产品导航

- 状态：`已修订（05）`
- 默认界面只展示名称、用途、模型、按用户任务分组的能力/能力包、工作区/知识库/连接器 picker、保存和“试用 Agent”。
- Save/Test 自动执行内部 Preview，不要求用户理解或先操作 Preview。
- initial/on-demand 默认由模板和 Capability metadata 决定；只有开发者模式可以覆盖。
- binding ID、resource ID、owner、operation 和 typed parameters 由后台生成，不要求用户手填。
- Revision、Snapshot、digest、protocol、raw Event 和 JSON 放入默认折叠的技术详情/导出诊断。
- Snapshot 不兼容时，界面只展示“在新会话中继续”，后台执行显式 fork。
- Package、Capability、Skill 和 MCP 仍保持各自清晰的管理入口；不恢复“设定市场”混合对象。

原“在普通编辑器直接展示完整 exact-set、digest、内部 ID 和复杂 Preview”的要求已撤销，因为它把实施合同泄漏成用户操作。

理由：用户需要配置任务能力和资源，而不是维护内部编译产物。隐藏技术细节可以保留诊断能力，同时缩短核心流程。

### D-011：首个端到端 Vertical Slice

- 状态：`已修订（05）`
- `chat.minimal` 零工具问答用于证明最小装配。
- `coding.codex` 用于证明完整 Codex-native Coding 能力。
- `sample.echo` 或等价 test fixture 用于证明 first-party 与未来扩展走同一 registration/materialization/invoke 主链。
- 三者不得使用 test-only Preset schema、mock Runtime 主链、旧 Factory、`GatewayDeps` 或业务型 `AppServices` 捷径。
- 它们是早期架构切片，不再被当作一期全部完成条件；当前完成定义以 05 的 release-required 产品闭环和 `GLOBAL-CLOSURE-TODO` 为准。

理由：三个切片分别覆盖最小成本、最高能力和可扩展主链，但不能替代真实 Browser、Remote、automation、生命周期和发布验证。

### D-012：fresh v4 数据代际

- 状态：`已确认`
- v4 使用独立、干净的 baseline 和数据 root。
- 不开发 pre-v4 Converter、import、dual read/write、compatibility view、旧字段 fallback 或 migration replay。
- pre-v4 Conversation、Nomi session、Preset、Knowledge、Memory 和业务 side stores 不进入 v4。
- fresh-v4 尚未 Stable 时直接修正 baseline 和 fixture，不为开发数据增加兼容 migration。
- v4 正式升级只依据 data generation、migration lineage/checksum 和 schema compatibility，不要求应用 build、决策文档或全局 ledger digest 完全相同。

理由：用户已接受重新配置，以换取干净数据模型。为不再读取的数据维护兼容层没有产品价值。

### D-013：旧数据目录处理与 Clean Cutover

- 状态：`已确认`
- 首次切换 v4 时，在同文件系统把整个旧 canonical data root 原子 rename 为 sibling archive，再创建空 v4 root。
- rename 或路径校验失败时停止启动，不使用 copy/delete、逐文件 move 或跨卷 fallback 冒充成功。
- archive 不被 v4 Runtime、Kernel、Plugin、API 或 UI 枚举、解析或恢复。
- 产品不提供 Legacy Viewer、Import、Restore 或 rollback generation。

理由：whole-root rename 提供最低成本的误删保护，同时不要求理解任何旧 schema，也不会把归档变成长期兼容入口。

### D-014：Legacy API、装配和制品删除边界

- 状态：`已修订（05）`
- 新 Slice 切换真实消费者时，同步删除对应 legacy route、DTO、repository、配置、Factory wiring 和双路执行入口。
- 不新增 deprecated alias、dual read/write、兼容 facade、隐藏 feature flag 或“下一版本再删”的新债务。
- 开发期只要求 `production_legacy_reachability = 0`：新 Session 和 public route 不能进入旧主链。
- Release 只要求 `release_legacy_artifacts = 0`：最终 feature、package、binary、config 和 process 不包含 Nomi。
- D-020 deletion manifest 中已有的 Nomi-only Browser/Computer wiring 可以作为精确、有期限的 allowlist 留到 C9，但不得增长或接入新架构。
- 文档、测试、fixture 和历史字符串不进入复杂 allowed/deferred/unclassified 分类，也不阻塞 release residual。

原“每个波次、全仓符号、文档和 evidence 都必须 exact-zero”的要求已撤销，因为它把删除审查扩张成长期规则引擎。

理由：需要删除的是仍可到达或仍会发布的旧系统，而不是抹除所有历史痕迹。两类 residual 足以保护产品主链和最终制品。

### D-015：SessionEvent、Projection 与恢复事实

- 状态：`已修订（05）`
- 保留一套语义 SessionEvent、单调 cursor、最终用户/助手消息、必要 Tool 摘要和稳定引用。
- Projection 只保存 UI 当前需要的终态、文本、Tool 摘要和引用，不再内嵌完整 `events[]` 或复制 Event Log。
- 模型 token/delta、typing、heartbeat、重复 progress 和 provider wire 默认 transient。
- 正常完成只持久化最终 assistant message；中断时最多保存一份 bounded partial。
- Runtime checkpoint/rollout 是可丢弃 cache；产品历史和 Projection 不以其为事实源。
- 可靠业务事实和外部 Effect reconcile 归 owning domain；SessionEvent 不复制完整业务记录。
- 不建设逐 token event sourcing、第二个 Runtime event DB、全局 EffectCoordinator 或只为证明 evidence 的 receipt 网络。

理由：语义 Event 足以支持产品历史、cursor 和恢复；复制全量原始流或把 Projection 变成第二份 Event Log 会产生写放大和多事实源。

### D-016：第三方插件正式支持范围

- 状态：`后续阶段`
- 一期只保证 vendor-neutral Package/Capability/Skill/MCP 主链，以及 Browser/Computer 的 source-neutral RoleProviderContribution shape。
- 一期不交付用户 Plugin 安装、启停、Replace、Uninstall、Node Runtime Manager、Extension Host、JavaScript SDK、MCP Role Adapter、CLI Provider、Provider picker、市场或 Chat Dev。
- 二期必须复用一期 canonical contribution、binding、Snapshot 和 dispatcher，不能另造 JS/MCP 专用身份或旁路。
- Skill 始终是 instructions/workflow；可执行部分必须来自 Capability/Provider。

理由：一期必须把未来扩展接缝放在正确位置，但用户插件产品需要 loader、SDK、配置和切换体验，应在主链稳定后独立验收。

### D-017：Remote 调用与 Agent 设定映射

- 状态：`已确认`
- `RemoteBinding` 复用 canonical Agent binding，固定 exact Preset revision、Snapshot 和 typed resources。
- Remote 协议只提供显式 `open/turn/observe/cancel`；REST/MCP 是传输适配器，不定义第二套 Session 模型。
- `open` 返回唯一 `agent_session_id`；后续请求必须显式提交该 ID，不按 token、连接、IP、客户端名或“最近会话”隐式复用。
- Binding 更新只影响之后创建的 Session；既有 Session 使用 frozen Snapshot。
- Remote 仍是 FullAuto，不增加 confirmation、danger approval、scope DSL 或 Remote 专属 Agent 类型。
- Runtime admission 与 SQLite 提交不伪装成跨系统原子事务；Session 必须确定进入 `ready` 或可诊断 `failed`，非 ready 不执行。

理由：服务端 Binding 让客户端保持简单，并确保 Remote、桌面和 automation 使用同一个 AgentSession/Snapshot 主链。

### D-018：轻量 Preset 与完整 Coding 边界

- 状态：`已确认`
- `chat.minimal` 不选择 Tool、Skill、MCP、Workspace、Knowledge 或业务 Context，最终模型请求保持 `tools=[]`。
- Compiler 只正向构造 Snapshot 已选择内容，不能全量初始化后再过滤。
- `coding.codex-native` 必须保留完整的原生 Coding feature、协议语义和代表性工作流。
- 验收使用能力集合、协议 conformance、正常构建测试和少量真实 E2E。
- 不建设 token/TTFT/P50/P95、reference-device、paired corpus、统计显著性、长期观察窗口或独立性能平台。

理由：轻量和完整都可以由结构与功能直接证明。量化性能计划在当前阶段成本高，且不决定架构正确性。

### D-019：实施并行、机器分工与估算

- 状态：`已修订（05）`
- 不固定五条 workstream、ROM、coding agent 数、工程周数、HP-1/HP-2 或 whole-cohort recheck 日历。
- 工作分解以 `GLOBAL-CLOSURE-TODO` 当前开放项、真实依赖和独占写集为准。
- 并发只在写集明确分离且能减少关键路径时使用；中央 schema、Composition Root、共享 Gate 等文件由单一 integration owner 串行合流。
- 当前主机是唯一实现与 merge 主机。多个 writer 只在本机互斥写集明确分离时并行；
  中央合同、组合根、Gate、锁文件和 GLOBAL TODO 由主机串行合流。
- 每批运行最小定向检查；broad checks 只在主要合流、Windows 核心闭环和最终 RC 执行。
- 遇到环境、真实凭据、原生主机或 harness 障碍时记录阻塞原因和人工步骤，不反复重试，也不写不优雅的测试绕过。

原固定“五流、213/314 EW、6～8 agents、29/42 周及两次 HP”的计划已撤销，因为估算和组织假设被误用为必须实现的产品合同。

理由：并发度应由当前可独立任务决定。本机互斥写集可以获得并发收益，同时避免跨机器
交接、分支同步和共享构建资源争用。

### D-020：Codex 最终切换与 Nomi 删除门禁

- 状态：`已修订（05）`
- Nomi 在过渡期冻结：不再承接新能力、新数据模型或新架构，只允许 deletion manifest 中的精确旧接线存在到 C9。
- 先完成 release-required 用户闭环、canonical Compiler/小 Snapshot、Browser/Computer Role seam 和生产主链无旧 reachability。
- 随后执行一次性 C9 shutdown：停止 Nomi admission，取消内部 Nomi 工作，bounded shutdown，清理 descendant process tree，对无法确认的真实外部 Effect 记 uncertain，验证 Nomi process/binding/public route/release artifact 不再存在，再删除 Nomi。
- C9 后不恢复 Nomi fallback、Engine selector、per-turn 切换或 pre-v4 binary。
- Nomi-free 候选随后在 Windows x64、macOS arm64、Linux Desktop x64 完成正式 RC；Stable 提升已验证的同一 RC bytes。

原“逐领域在线 sticky canary、shadow、durable handoff 后再排空”的方案已撤销，因为本地 pre-Stable fresh-v4 不需要服务器级零停机迁移平台。

理由：一次性 clean cut 可以保留最终“完全删除 Nomi”的目标，同时删除长期双 Runtime、在线迁移和证明系统的成本。

### D-021：统一 AgentSession 身份

- 状态：`已确认`
- 新架构只有 `AgentSession/AgentSessionId` 一个 canonical aggregate 和 lowercase UUIDv7 主键。
- 中文 UI 可称“会话”；英文聊天界面可称 Chat，执行与诊断界面称 Session。
- API、Rust/TypeScript、数据库和 Event 不再使用 `Conversation` 作为新技术术语。
- Remote `open` 返回同一个 `agent_session_id`；fork 创建新的 AgentSession，并记录有界 parent/fork provenance。
- 标题、消息、Projection、Runtime binding、Remote provenance 和删除生命周期都归同一个 Session。

理由：Conversation 与 Session 双 ID 会复制创建、恢复、删除和映射逻辑。一个 aggregate 足以覆盖聊天和自动化执行。

### D-022：Agent Test 与三类 Effect

- 状态：`已修订（05）`
- dirty draft 点击 Test 时先保存普通、可见、immutable `AgentPresetRevision`；clean draft 复用当前 Revision。
- Test 通过普通 AgentSession API 创建真实持久 Session，使用真实 Snapshot、资源和 FullAuto 主链。
- 不建设 test-only Session、DraftSnapshot、模拟 Runtime、测试专用表或审批弹窗。
- UI 可以明确提示会产生真实副作用，但提示不能创建第二套确认状态。

Effect 只保留三种策略：

```text
read_only
managed_effect
external_uncertain_effect
```

- 本地 DB、KV、文件和 VCS 使用事务、revision/CAS 或同目录临时文件 + rename，并记录一个最终 Tool result。
- 外部发送、远程命令、设备控制等可能出现未知结果的操作，在 dispatch 前保存必要 reservation；unknown 时禁止自动 retry，由 owning domain 使用原 idempotency identity reconcile。
- `EffectClass` 可以作为展示和路由 metadata，但不能把所有非读操作推进统一完整状态机。
- 不建立全局 EffectCoordinator、Wave 级 JSON/CAS journal、固定 receipt 集合或与 SessionEvent 重复的记录。

理由：Test 必须与真实执行同构；Effect 正确性则取决于操作性质，不应为了形式统一给所有写操作增加分布式状态机。

### D-023：官方模板 Seed 政策

- 状态：`已修订（05）`
- 核心原则继续是 **role-complete but context-minimal**。
- `chat.minimal` 保持 exact-empty；`coding.codex` 保持完整 `coding.codex-native`。
- 其他角色模板应具备开箱成立所需能力，但低频/重型能力放入 on-demand，不在每轮全部注入。
- Runtime 只能 search/activate Snapshot 已冻结的 on-demand ceiling。
- 精确 seed、binding 和 partition 由 canonical inventory/manifest 生成，不由本文复制字段级列表。
- 测试不锁定模板、Capability 或 generated record 的固定数量。

理由：角色必须可用，但固定数量和文档清单容易漂移。canonical inventory 加行为测试比源码字符串和计数 Gate 更可靠。

### D-024：AgentSession 删除与 minimal tombstone

- 状态：`已修订（05）`

当前唯一删除流程：

```text
live
→ deleting
→ 停止新写入
→ cooperative dispose
→ hard-kill descendant process tree
→ 幂等删除 Session 自有内容
→ minimal tombstone
```

- Runtime 返回真实 `RuntimeDisposeReport`；Session Store 只删除自己拥有的表和内容。
- 启动时发现 `deleting`，重新执行幂等清理并完成 tombstone。
- tombstone 只承担 ID 防复用、迟到请求围栏和已删除状态，不保存可恢复内容。
- 已发生的外部业务 Effect 事实仍由 owning domain 保留，不因删除 Session 被伪装撤销。
- 重复 delete 和迟到 callback 确定性返回 deleted，不重建 Event、Projection 或 Runtime binding。

原 `ZeroOutstandingProof`、多维零计数和复杂 Delete Operation 状态机已撤销，因为调用者填写“零”不能证明真实资源已消失。

理由：真实 dispose report、进程树清理和幂等存储删除已经覆盖产品语义；额外证明对象只会增加无法闭合的状态。

### D-025：单 Compiler、小 Snapshot 与旧 Session 可执行性

- 状态：`已修订（05）`

Compiler 只有一个 canonical 纯函数实现：

```text
Preview ─┐
Save ────┼─> one canonical Compiler
Test ────┘          │
                    └─> Snapshot + authority + diagnostics

Session Open ─> 读取已保存 Snapshot + 当前执行兼容检查
```

- Control Plane 只把 diagnostics 映射成产品 DTO，不复制 dependency closure、profile 或 digest 算法。
- Session Open 不重新编译，也不维护第二份 checkpoint/Snapshot compatibility 实现。

Snapshot 只冻结实际执行闭包：

- 已选择的 Capability、Provider 和 Package contribution；
- 实际 Tool schema、Model Route 和 typed resource binding；
- 当前需要的 Runtime protocol/features；
- initial/on-demand 分组；
- Snapshot 自身 digest。

以下全局事实不决定旧 Session 是否可执行：

- 未选择的 Package/Capability；
- 整个 target inventory 或官方模板全集；
- 决策文档 digest；
- 与当前 Session 无关的全局 schema ledger。

兼容性只在 Runtime binding 建立、实际 Capability 激活或其执行实现变化时检查并缓存。结构不兼容时，原 Session 保持可读，执行返回 `SNAPSHOT_EXECUTOR_UNAVAILABLE`；用户显式选择当前 binding/resources 后 fork 新 Session。不得静默换 Provider、重写旧 Snapshot、resolve latest 或降级 Coding。

原“每次 resume/turn 对完整全局 ceiling、inventory 和多组 digest 做 exact compatibility proof”的要求已撤销，因为无关全局变化不应使既有 Session 失效。

理由：一个 Compiler 消除控制面与 Kernel 漂移；小 Snapshot 锁定真正影响执行的事实，同时保留确定性恢复和显式 fork。

### D-026：Remote token rotate/revoke

- 状态：`已修订（05）`
- token validator 的原子 generation/hash/status 是请求认证的唯一线性化点。
- 已通过验证的请求可以正常完成；fence 之后验证的旧 token 立即返回 `REMOTE_AUTH_REQUIRED`。
- 不把 `RemoteRequestAdmissionPermit` 或异步读锁持有到 HTTP Response Body 被完整消费。
- rotate/revoke 不删除、fork、rebind 或级联 cancel 既有 AgentSession。
- replacement token 认证为同一 owner 后，可以显式携带 `agent_session_id` 继续现有 Session。
- 不建设 TTL/grace、token scope、token→Session 索引、后台 revoke worker 或第二个 coordinator。

原跨 Response Body 持锁方案已撤销，因为客户端只读 status 后 revoke 会让写锁永久等待。

理由：认证 fence 与 Session lifecycle 是两件事。原子 admission 状态已经提供足够且可验证的 revoke 语义。

### D-027：一次性 C9 shutdown

- 状态：`已修订（05）`

当前 C9 操作顺序：

```text
停止 Nomi 新 admission
→ 取消全部内部 Nomi 工作
→ bounded application/runtime shutdown
→ kill descendant process tree
→ 对无法确认的真实外部 Effect 记 uncertain
→ 验证 Nomi process、binding、public route、release artifact 不再存在
→ 删除 Nomi
```

- 禁止把同一 AgentSession 中途切换到 Codex Runtime。
- 禁止自动 replay/retry 外部 Effect。
- 不等待所有 owning domain reconcile 完成才删除 Nomi；uncertain 使用原 identity 留给领域处理。
- 不保留祖先 deadline 最小值、per-domain sticky canary、read-only shadow、durable Session handoff 或多维 outstanding ledger。

原在线排空方案仅保留“一旦开始删除就停止新 admission、最终清理进程树”的目标；其服务器级迁移机制已撤销。

理由：一期尚未 Stable，且 fresh-v4 不承诺在线零停机。bounded shutdown 能给 C9 确定终点，并避免永久 opening/draining 状态。

### D-028：三平台发布与验证策略

- 状态：`已修订（05）`

首批 release-blocking 平台：

1. Windows Desktop x64；
2. macOS Desktop arm64；
3. Linux Desktop x64。

macOS x64 与 Linux Headless x64 保留设计兼容和后续交付入口，但不阻塞首个 Stable；未在真实原生环境验证时不得宣称已交付。

当前收口链：

```text
S0 STOP-LOSS
→ S1 FOUNDATION
→ S2 CORE FUNCTIONAL
→ S3 NATIVE SMOKE
→ S4 C9 CLEAN CUT
→ S5 FINAL RC
→ S6 STABLE
```

- Windows 完成 release-required 核心闭环和代表性功能/失败/进程清理。
- macOS arm64 与 Linux Desktop x64 对真实候选 Artifact 运行 build/package/install/launch、critical capability 和 lifecycle smoke。
- C9 后，三个首发平台对正式 Nomi-free RC 运行最终 package/install/fresh/critical E2E/lifecycle。
- Stable 原样提升已验证的 RC bytes，不重新构建另一份制品。
- `release-lock.json` 只记录真实 Host/Sidecar/Package digest；`platform-result.json` 记录目标平台、实际 suite、结果和日志引用。
- 相同 Artifact digest 可以复用仍相关的证据；只有产品 ABI、Runtime protocol、Package 或目标平台 Artifact 改变才使对应结果 stale。
- dirty worktree 可运行 verify 作为诊断；只有 release attestation 要求 clean commit 和真实 Artifact。
- 原生平台结论必须来自对应真实 Host；cross-compile、Rosetta、VM、容器或静态检查只能作为 preflight。

原“五个 native cells、固定 HP、四元 tuple、whole-cohort exact evidence 和两轮全量 Gate”已撤销，因为其验证成本超过首发产品风险，并造成反复换机和证据失效。

理由：三平台覆盖当前首发桌面用户，Windows 承担完整核心验证，另外两台真实 Host 验证平台制品和关键能力。这样保留发布可信度，又避免把所有内部测试复制成五平台笛卡尔积。

## 当前阅读与实施规则

1. 先完整读取 `05-system-capability-replacement-foundation.zh.md`，再用本文追溯 D-001～D-028 的决策理由。
2. 领取和关闭工作只看 `GLOBAL-CLOSURE-TODO.zh.md`；不得从本文推断某项已经实现或通过 Gate。
3. Browser/Computer 实施必须先落 Role/Provider seam，再接具体 owner；不能在旧直连上叠加 adapter。
4. Sidecar 先完成 upstream spike；不能继续围绕不存在的私有 patch 扩大 Host contract。
5. 新实现优先删除重复 Compiler、全局 Snapshot 事实、通用 Effect 状态机、虚假 zero proof 和旧生产旁路。
6. 任何需要恢复旧固定 ROM、在线 canary、五平台首发、全量 exact-zero/evidence 或复杂 handoff 的变化，都必须重新提出产品理由并获得明确决策。
