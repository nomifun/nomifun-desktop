# NomiFun Agent Capability Platform v2 决策记录

> 文档状态：**CURRENT DECISION LEDGER / 2026-09-06**
>
> 权威顺序：`05-system-capability-replacement-foundation.zh.md` 是一期当前修订合同；本文与 05 冲突时，以 05 为准。
>
> 状态来源：`GLOBAL-CLOSURE-TODO.zh.md` 是当前实施进度、阻塞项和关闭状态的唯一来源。本文只记录架构决策及理由，不声明代码、Gate、平台验证或发布已经完成。
>
> 历史处理：保留 D-001～D-039 的决策编号、仍有效的结论和形成理由。已被 05 否定的旧要求只保留极短撤销原因，不再作为正文中的候选方案、实施步骤或验收合同。

## 状态约定

- `已确认`：结论继续有效，且未被 05 实质改变。
- `已修订（05）`：原决策问题和目标仍有效，但当前可执行合同已由 05 替换。
- `已修订（2026-09-03）`：当前产品执行口径覆盖旧的 Runtime 组合或发布前提；原设计理由继续保留。
- `后续阶段`：方向仍有效，但不属于一期交付合同；必须等后续设计正式确认后实施。

## 2026-09-05 当前执行覆盖

本节承接 2026-09-03 用户确认的当前执行顺序，并补充 2026-09-05 本机 checkpoint；
其优先级高于本文此前把 Codex 或 Nomi 删除
写成一期当前前提的表述：

1. 当前产品唯一执行内核是 NomiFun 原有 Nomi engine，由
   `nomifun-ai-agent`、`nomifun-conversation`、`nomi-agent` 和现有
   `AgentRuntimeRegistry` 承载。
2. Web、Desktop 和 `nomicore` 默认入口使用 `NomiCoreApplication`；`FreshV4Application`
   作为显式、低成本的多-runtime host boundary 保留，供二期/三期重新接入
   Codex app-server 或其他 Runtime。
3. 不提供运行中 Runtime 切换、per-turn 切换或自动 fallback。一个
   `AgentSession` 绑定一个明确的 host；不可用时显式失败或保持可读。
4. Codex app-server 当前只做协议/Sidecar 研究和未来宿主设计；fixture、adapter、
   synthetic contract、Broker smoke 和生命周期单测都不等同于 Codex-native
   已移植或已完成。
5. 实时状态、open/blocked/external/deferred 和下一步只以
   `GLOBAL-CLOSURE-TODO.zh.md` 为准。
6. 当前 App 只组合一个 `NomiCoreSessionOwner` facade，供普通 Session、Remote、
   Cron、AutoWork、Channel、Companion、IDMM 和 AgentExecution 共用。它继续委托
   原有 Nomi engine 的 Conversation 生命周期；共享 owner 消除多实例 lifecycle
   分裂，但不把 Conversation-backed compatibility 误报为 canonical scheduled
   Session migration。
7. Remote 的状态转移与事件追加必须在同一 SQLite 事务内完成；相同 operation key
   的不同 payload 是冲突，未知结果必须保留 unknown，Remote 响应使用 Remote
   event cursor。AutoWork 的后台任务关闭必须有 cancellation/join 或 exact claim
   cleanup；这两条是当前已验证的实现约束，不是新的全局 coordinator。
8. 2026-09-05 本机通过 Windows Credential Manager runner 完成真实 StepFun
   Nomi-core Chat/Coding、Cron 同 Session run/replay 和 Remote
   `open -> turn -> observe -> cancel` smoke，结果为
   `live_smoke_status=pass code=OK status=200`。该 evidence 关闭
   `SL-S3-07` 和 `SL-S3-11`；不把 automation 的 canonical Session 缺口或
   Desktop 人工验收隐式视为完成。Nomi-core Remote REST/MCP 的 installation
   Bearer、owner JWT/local-trust 兼容、旧 selector query fail-closed、cursor、
   idempotency、revoke 和 delete 后不可复活均有独立或真实 smoke 证据。
9. Nomi-core `/mcp` 复用公共 Streamable HTTP transport 的会话 admission、四工具
   schema 和 installation Bearer boundary；host 只注入四个 Nomi-core Remote
   operation，不伪造 Fresh-v4 `AgentPlatform` 或第二套 Session authority。
10. 05 §15 的 AP-0～AP-7 是进入 06 代码实施的前置门禁。当前只确认了部分
    contract/route/migration 写集；`GLOBAL-CLOSURE-TODO.zh.md` 中的 AP 状态才是
    实时状态，不能由本决策记录推断 admission 已通过。
11. Agent 的公开产品入口固定为 `/agent`，Session 使用
    `/agent-sessions/:agentSessionId`；旧 `/presets`、`/settings/agent-presets`、
    `/settings/agent` 只能作为限期迁移围栏，不能继续承载 authoring。
12. 持久化执行投影统一使用 `agent_snapshot`。061/062/064 分别完成 snapshot 命名、
    ContributionLock 和 `payload_json` 的物理收口；065 增加用户 Preset retirement
    tombstone，066 退役包含旧资源实例字段的 Preset 并删除活动 Binding，同时保留
    Revision/Snapshot/Session 历史。Fresh-v4 只 seed 官方模板且没有旧 preset
    projection 表；当前 migration head 为 66。
13. 2026-09-06 人工走查确认首页 Guid 未真正提供 AgentPreset 选择，旧 AP-7
    admission 因而撤销。后续纠偏已经通过真实 Tauri Desktop 产品走查：默认 Nomi
    保留模型选择，AgentPreset 会话锁定 Snapshot 模型，能力三态/明细、删除、首页
    预选和目标级 Workspace/Knowledge 已闭环。`SL-S4-02` 已关闭；AP-7 已针对实现
    提交 `ef5f5380915e0d5c06004f03b66cbd1302b3fe03` 重新签署。手机模式不属于
    `nomifun-desktop` 本期服务范围。

## 决策总览

| ID | 决策 | 状态 | 当前有效结论 |
|---|---|---|---|
| D-001 | 产品与内部领域命名 | 已确认 | 产品统一称“Agent 工作台”；内部使用 `AgentPreset` / `AgentPresetRevision`，运行实例使用 `AgentSession` |
| D-002 | Agent 执行模式 | 已确认 | 只保留 FullAuto；不保留审批、确认、Grant/Lease/Permit 状态机 |
| D-003 | 平台事实与 Runtime 所有权 | 已修订（2026-09-03） | NomiFun 持有平台和业务事实；当前产品 host 是 NomiCore/Nomi engine；任一 Runtime 都必须经 frozen Snapshot 和显式 host boundary 接入 |
| D-004 | Codex 基座与 Nomi 替换 | 后续阶段 | 当前不切换或删除 Nomi；Codex app-server 仅保留协议/Sidecar 研究和未来宿主入口，重新立项后再评估 Codex-native 或 external integration |
| D-005 | 普通插件运行模型 | 已确认 | 普通第一方和第三方插件按 trusted in-process code 处理；不建设通用 sandbox/权限平台 |
| D-006 | Thin Kernel 与业务边界 | 已确认 | Kernel 只保留不可由普通插件自举的基础事实；业务域进入统一插件主链 |
| D-007 | Package/Capability/Skill/MCP 分层 | 已修订（05） | 保留四层语义；Browser/Computer 增加 canonical Role/Provider seam，所有消费者仍只调用 canonical Capability |
| D-008 | initial/on-demand 能力 | 已修订（05） | 保留两个集合和按需激活；用户可将每项能力设为关闭、启动即用或可按需申请；Selection 只保留能力引用和真实 action allowlist |
| D-009 | 官方 Agent 模板 | 已确认 | 保留角色型模板与业务边界；精确 seed 来自 canonical inventory，不以固定数量测试作为 Gate |
| D-010 | Agent 工作台编辑体验 | 已修订（05） | 普通界面展示能力明细与三态控制、模型和产品语义；只展示所需资源种类，具体资源 picker 放在会话/伙伴/自动化消费目标；Revision/Snapshot/digest 等进入折叠技术详情 |
| D-011 | 首个 Vertical Slice | 已修订（2026-09-03） | `chat.minimal` 与 Nomi-backed Coding 是当前真实切片；`coding.codex`/fixture 只保留为历史或未来 Codex 研究，不是当前完成证据 |
| D-012 | v4 数据代际 | 已修订（2026-09-03） | Fresh-v4 保留为显式备用 host/data boundary；当前 Web/Desktop/`nomicore` 使用 NomiCore/Nomi data root，不迁移 pre-v4 数据 |
| D-013 | 旧数据目录处理 | 已确认 | 同文件系统 whole-root rename 后创建空 v4 root；归档不进入运行、恢复或兼容主链 |
| D-014 | Legacy 删除边界 | 已修订（2026-09-03） | 当前检查默认入口只走 NomiCore、无 Runtime 切换/fallback；Nomi 是当前产品内核，Nomi-free 删除门禁只属于未来 Codex 切换 |
| D-015 | SessionEvent 与 Projection | 已修订（05） | 保留一套语义 SessionEvent；Projection 只保存 UI 终态，不复制完整 Event Log，token/delta 默认 transient |
| D-016 | 第三方插件正式产品化 | 后续阶段 | 一期只冻结 source-neutral 主链；用户安装、SDK、Extension Host、市场和切换 UI 留给二期 |
| D-017 | Remote 与 Agent 设定映射 | 已确认 | `RemoteBinding` 复用 canonical Agent binding；显式 `open/turn/observe/cancel`、`agent_session_id`、原子 Remote event cursor 和未知结果保留 |
| D-018 | 轻量 Chat 与完整 Coding | 已修订（2026-09-03） | 轻量 Chat 和完整 Coding 当前均由 Nomi engine 执行；on-demand 没有 canonical activation port 时 fail-closed；Codex-native 另属后续阶段，不以 fixture/adapter/Broker smoke 代替 |
| D-019 | 实施并行与估算 | 已修订（05） | 不再固定五流、ROM、Agent 数或周数；按当前 TODO、独占写集和真实依赖动态并行 |
| D-020 | Nomi 最终删除门禁 | 后续阶段 | 当前 Nomi 是产品执行内核，不执行 C9 删除；只有未来 Codex Runtime 正式接替并完成独立证据后，才重新评估一次性删除 |
| D-021 | Conversation 与 Session 身份 | 已确认 | 新架构只有 `AgentSession/AgentSessionId` 一个产品会话 aggregate 和 UUIDv7 主键 |
| D-022 | Test Revision 与真实 Effect | 已修订（05） | Test 走普通 Revision/Session；Effect 只分 `read_only`、`managed_effect`、`external_uncertain_effect` |
| D-023 | 官方模板 Seed 政策 | 已修订（05） | 保留 role-complete/context-minimal；不以固定模板或 Capability 数量生成结构 Gate |
| D-024 | Session 删除 | 已修订（05） | 简化为 `live → deleting → dispose/kill → 幂等删除 → minimal tombstone` |
| D-025 | Snapshot 可执行性 | 已修订（05） | 只保留一个 canonical Compiler；Snapshot 冻结能力/Provider/Tool/Model 闭包和资源种类约束，不冻结消费目标资源实例；结构不兼容时只读并显式 fork |
| D-026 | Remote token rotate/revoke | 已修订（05） | 原子 generation/hash 是 admission fence；不跨 Response Body 持锁，不增加 grace/worker/Session 索引 |
| D-027 | Nomi 排空 | 后续阶段 | 原在线 drain 设计撤销；C9 bounded shutdown 只在未来 Codex 切换并决定删除 Nomi 时执行 |
| D-028 | 发布平台矩阵 | 已修订（2026-09-03） | 当前首发验证针对 Nomi-core 候选：Windows x64、macOS arm64、Linux Desktop x64；macOS x64/Linux Headless 后续交付 |
| D-029 | 当前产品 Runtime 与多-runtime host boundary | 已确认（2026-09-05） | Web/Desktop/`nomicore` 默认使用 NomiCoreApplication；由单一 NomiCoreSessionOwner 共享当前 Nomi engine Session 生命周期；FreshV4Application 是显式未来 host；不支持运行中切换或 fallback |
| D-030 | Automation 使用 host-owned typed Session boundary | 已修订（2026-09-05） | Cron 只提交封闭 runtime overlay 并通过原子关系/typed receipt 工作；AutoWork 使用 issuer-scoped opaque lease、Session projection revision fence 和 owner/revision/operation-aware config CAS；不得把任意 runtime `extra` 当作 Session authority |
| D-031 | 领域 adapter 的真实迁移判定 | 已修订（2026-09-05） | 以生产 legacy 清零、共享 host owner、typed boundary 和行为回归判定本阶段 host-boundary 收口；Channel/IDMM 的未来 canonical live event、完整 receipt 与 continuation contract 仍单独跟踪，不伪装为已完成 |
| D-032 | 真实 Provider smoke 与凭据边界 | 已确认（2026-09-05） | 真实证据必须经过 Nomi-core AgentSession、Provider/Model route、代表性工具调用和关闭审计；Windows 凭据只经受控 Credential Manager/Bun runner 短暂持有，在构建完成后一次性 stdin 交接，不进入仓库、参数、日志或 Cargo/build/test/application 子进程环境；一次 smoke 只关闭其明确覆盖的 TODO |
| D-033 | Nomi-core Remote MCP transport | 已确认（2026-09-05） | Streamable HTTP transport/session admission/tool schema 由 `nomifun-public` 统一持有；Nomi-core 通过 `CanonicalRemoteOperations` 注入既有 Remote handler，复用 owner、provenance、idempotency、cursor 和 runtime，不构造伪 `AgentPlatform` 或第二套状态机 |
| D-034 | `SL-S3-10` host-boundary 收口 | 已确认（2026-09-05） | Cron/AutoWork/Requirement/AgentExecution/Channel/IDMM 均经同一个 `NomiCoreSessionOwner` 接收领域自有 typed contract；Conversation-backed bridge 仅保留在测试支持或 app composition，审计必须报告 `production_legacy_files=0`、`transitional_adapters_with_legacy_dependencies=0`、`candidate=none`，但不宣称 canonical Session 已具备所有未来 live event/receipt 能力 |
| D-035 | Agent 工作台公共入口与迁移围栏 | 已修订（2026-09-06） | 侧边栏明确显示“Agent 工作台”并进入 `/agent`；首页 Guid 是选择已保存 AgentPreset 并启动会话的入口，不是第二个 authoring surface；旧 preset 深层路由必须删除 |
| D-036 | `agent_snapshot`、Revision payload/locks 与 Fresh-v4 clean cut | 已修订（2026-09-06） | 061/062/064 使用物理命名且不做 alias；065/066 完成 retirement 与旧资源绑定 Preset 退役；Fresh-v4 只 seed 官方模板，Revision 使用 `payload_json` 与 ContributionLock，不保留 Package template source 或旧 preset projection 表 |
| D-037 | AP-7 admission 证据边界 | 已修订（2026-09-06） | 旧签署因缺失真实 AgentPreset 启动选择器而撤销；纠偏实现、真实 Tauri Desktop 产品走查和 clean gate 均完成，AP-7 已针对 `ef5f53809` 重新签署；06 仍保持独立边界 |
| D-038 | 首页 AgentPreset 选择与高层 Session 创建 | 已修订（2026-09-06） | Guid pill bar 只列可执行用户 AgentPreset，`+` 打开 `/agent`，工作台启动会话会预选；客户端只提交 `preset_id/title`，服务端解析稳定 Revision/Snapshot/Binding；普通 Nomi 保留模型选择，Preset 会话锁定 Snapshot 模型；具体 Workspace/Knowledge/Connector 只在消费目标选择；执行引擎只属基础设施 |
| D-039 | Preset Session 模型与资源种类冻结 | 已确认（2026-09-06） | Snapshot digest 冻结精确模型和 `required_resource_kinds`；Preset Session 不允许 UI 或公开 PATCH 改模型，桌面资源入口只读冻结资源种类；当前 Catalog 升级不得改写历史 Session |

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
- 产品入口和用户可见对象统一称 **Agent 工作台 / Agent**；“Preset”只保留为内部聚合名。
- 内部可编辑对象为 `AgentPreset`，不可变版本为 `AgentPresetRevision`。
- 产品运行实例为 `AgentSession`，不使用“系统 Agent”或 `AgentDefinition` 指代完整设定。
- 产品不提供 Runtime/Engine catalog；执行实现是内部基础设施。

理由：设定、不可变版本和运行实例具有不同生命周期。分开命名可以避免把用户配置、Runtime 进程和产品会话混成一个对象。

### D-002：Agent 执行模式

- 状态：`已确认`
- 产品只保留 FullAuto。`YOLO` 只能作为历史或研发别名，不能成为第二套机器合同。
- Agent 只能调用 Snapshot 已冻结的 Capability/Tool ceiling，并只能使用当前消费目标
  已绑定且满足 required resource kinds 的资源；范围外调用明确失败。
- v4 不保存 approval、confirmation、permission mode、Grant、Consent、Lease 或 Permit。
- 未来若出现真实审批需求，必须作为独立产品需求重新设计，不能预埋等待状态。

理由：能力范围已经由 Preset、Compiler 和 Snapshot 决定，再增加审批状态机会形成第二套权限事实和大量不可恢复的等待分支。

### D-003：平台事实、业务数据与 Runtime 的所有权

- 状态：`已修订（2026-09-03）`
- Capability、AgentPreset、SessionEvent、Snapshot、Secret 引用和业务数据都由 NomiFun 持有。
- 当前产品由 `NomiCoreApplication` 组合原有 Nomi engine；本阶段继续把 canonical
  Snapshot 的 Model、Context、Tool 和 Event 边界收口到该执行图。
- 未来 Runtime 只能通过显式 host boundary 使用 frozen Snapshot；`FreshV4Application`
  保留为可复用的独立组合，不成为当前默认入口。
- Runtime 不得直接取得全量数据库、Secret、`AppServices` 或任意业务 service bag。
- Knowledge、Memory、Companion、Channel、Browser、Computer、Robot 等领域继续拥有自己的业务事实。
- Runtime thread/rollout/checkpoint 只是可丢弃或可重建的执行绑定，不是产品 Session 主键或历史事实源。

理由：Runtime 可以升级、崩溃或替换，而产品事实必须保持稳定。把权威数据留在 NomiFun 可以避免 Runtime 私有格式支配业务模型。

### D-004：Codex 基座、Sidecar 与 Nomi 最终替换

- 状态：`后续阶段`
- 当前产品不切换或删除 Nomi；Nomi engine 是本阶段唯一执行内核。
- Codex app-server 当前只保留协议/Sidecar 研究和未来宿主边界，不宣称源码已移植、
  已成为核心或已完成 Codex-native Coding。
- 未来若重新立项，才评估 `coding.codex-native` 是否应保留 Codex 的 workspace、
  AGENTS、Git、文件、patch、shell/PTY、Skills、MCP、计划、子 Agent、review、恢复和
  原生 Responses 语义。

未来 Sidecar/Runtime 研究合同：

1. 先对官方 Codex app-server 的 initialize/version、thread/turn、cancel 和 event 协议做真实 upstream spike。
2. 一个 Runtime binding 当前独占一个受管进程；结束时先关闭协议，再由 Host 清理整棵进程树。
3. Host-managed Tool 到达 Host 后直接按 Effect 策略处理并返回，不要求额外的 `native_action/start` RPC。
4. 只有 upstream callback/Tool seam 无法提供 Codex-native file/shell action 所需的最小调用前通知时，才允许一个窄 patch。
5. hello 只校验协议 major、build identity 和必要 feature，不镜像整个产品合同。
6. 是否保留浅 fork 必须由 spike 结果决定，不能因旧 Host adapter 已存在就倒推自定义 RPC 必须存在。

原“先建设三项自定义 Sidecar RPC 和完整 fork 合同”的要求已撤销，因为它依赖不存在的 patch source，并把外部假设变成一期硬阻塞。上述研究结果不改变当前 Nomi-core 主链。

理由：目标是获得成熟 Coding Runtime，同时把依赖和进程故障与桌面 Host 隔离；不是为了维护一套比 upstream 更大的私有协议。

### D-005：普通插件运行模型

- 状态：`已确认`
- 普通第一方和未来第三方插件统一按 trusted in-process code 处理。
- 一期不建设 WASI Host、通用 subprocess ABI、sandbox、签名链、供应链信任、插件权限引擎或多层 Secret Broker。
- Package manifest 只声明加载、版本、依赖、配置、贡献和生命周期所需事实。
- 未来若接入 Codex Runtime，可使用 Sidecar 复用 upstream 产物、隔离依赖和治理进程树；
  这不是当前 Nomi-core 的插件安全策略例外层，也不是本阶段产品执行路径。

理由：当前阶段需要的是一条可调试、可组合的插件主链。为尚未交付的第三方生态预建隔离系统会显著增加 RPC、状态和跨平台成本。

### D-006：Thin Kernel 与业务域边界

- 状态：`已确认`
- Kernel 只保留 App bootstrap、SQLite/migration、最小 ownership、canonical Compiler、Capability Registry、AgentSession authority、Runtime client/supervisor、Model Broker、最小 Remote auth、基础 EventBus 和 Plugin Manager。
- Knowledge、Memory、Companion、Browser、Computer、IM/Channel、Customer、Robot、Creative、Requirement、AutoWork、Cron、IDMM、AgentExecution、SSH、Office、Webhook 等业务能力属于插件或具名领域 owner。
- Composition Root 可以构造基础设施，但不得重新形成业务型 God Service。
- 当前 `NomiCoreApplication` 可以保留 `AppServices`、`ConversationService` 和
  `AgentRuntimeRegistry` 作为原有 Nomi engine 的组合根；这不是 Codex 旁路，也不是
  允许新增业务依赖的理由。
- `GatewayDeps`、旧 Factory 手工组合和过宽的业务 service bag 仍应按真实消费者迁移
  逐步收缩；是否能删除由 `GLOBAL-CLOSURE-TODO` 的实际依赖状态决定，不以 Codex
  接入或 C9 删除作为当前前置。

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
- Capability Selection 首版只保留 capability ref 和 action allowlist。
- `resource_binding_refs` 与具体资源实例不得进入 Preset/Revision/Snapshot；资源需求由
  Capability 的 `required_resource_kinds` 表达，实例由消费目标绑定。
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
- 默认界面只展示名称、用途、模型、每项能力的名称/说明/来源/可用性/资源种类，
  以及关闭、启动即用、可按需申请三态、保存和“试用 Agent”。
- Save/Test 自动执行内部 Preview，不要求用户理解或先操作 Preview。
- initial/on-demand 由模板预置，用户可以在工作台显式调整。
- 工作区、知识库、MCP/Connector 等具体资源在会话、伙伴或自动化目标中选择。
- binding ID、resource ID、owner、operation 和 typed parameters 不进入 Preset 编辑器。
- Revision、Snapshot、digest、protocol、raw Event 和 JSON 放入默认折叠的技术详情/导出诊断。
- Snapshot 不兼容时，界面只展示“在新会话中继续”，后台执行显式 fork。
- Package、Capability、Skill 和 MCP 仍保持各自清晰的管理入口；不恢复“设定市场”混合对象。

原“在普通编辑器直接展示完整 exact-set、digest、内部 ID 和复杂 Preview”的要求已撤销，因为它把实施合同泄漏成用户操作。

理由：用户需要声明 Agent 有什么能力、哪些能力可按需申请，并在具体使用场景中
绑定资源，而不是把某个知识库或路径永久冻结进 Agent 设计。隐藏技术细节可以保留
诊断能力，同时缩短核心流程。

### D-011：首个端到端 Vertical Slice

- 状态：`已修订（2026-09-03）`
- `chat.minimal` 零工具问答用于证明最小装配。
- 当前完整 Coding 切片由 Nomi engine 执行，覆盖读写、patch、shell/process、
  diff/commit 和真实 Session 生命周期。
- `coding.codex`/`coding.codex-native` 保留为历史设计名称或未来 Codex 重新接入时的
  候选切片，不作为当前 Nomi-core 完成条件。
- `sample.echo` 或等价 test fixture 用于证明 first-party 与未来扩展走同一 registration/materialization/invoke 主链。
- fixture 只能证明对应合同或路由分支，不得升级为真实 Runtime、真实模型、
  产品 Chat/Coding 或 Codex-native PASS。
- 当前完成定义以 05 的 Nomi-core release-required 产品闭环和
  `GLOBAL-CLOSURE-TODO` 为准。

理由：三个切片分别覆盖最小成本、最高能力和可扩展主链，但不能替代真实 Browser、Remote、automation、生命周期和发布验证。

### D-012：fresh v4 数据代际

- 状态：`已修订（2026-09-03）`
- Fresh-v4 继续使用独立、干净的 baseline 和 data root，作为显式备用 host/data
  boundary；它不是当前 Web、Desktop 或 `nomicore` 的默认产品组合。
- 当前 Nomi-core 组合使用与既有 Nomi 数据语义相容的 Nomi-core data root，并通过
  独立命名避免覆盖 Fresh-v4 实验数据。
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

- 状态：`已修订（2026-09-03）`
- 新 Slice 切换真实消费者时，同步删除对应 legacy route、DTO、repository、配置、Factory wiring 和双路执行入口。
- 不新增 deprecated alias、dual read/write、兼容 facade、隐藏 feature flag 或“下一版本再删”的新债务。
- 当前开发期要求 Web、Desktop 和 `nomicore` 默认入口只进入 NomiCoreApplication，
  不隐式选择 Fresh-v4/Codex host，也不提供运行中 Runtime selector 或 fallback。
- 当前 Release 验证 Nomi-core 候选的真实 feature、package、binary、config、process、
  数据和生命周期；Nomi 是预期执行内核，不属于当前要删除的 legacy artifact。
- 未来只有在 Codex Runtime 重新立项并正式接替后，才恢复 Nomi 删除清单和
  `release_legacy_artifacts = 0` 的 Nomi-free 门禁。
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
- `RemoteBinding` 复用 canonical Agent binding，固定 exact Preset revision 和 Snapshot；
  Remote 自己的具体资源在其消费目标 admission 中绑定，不写回 Preset/Snapshot。
- Remote 协议只提供显式 `open/turn/observe/cancel`；REST/MCP 是传输适配器，不定义第二套 Session 模型。
- `open` 返回唯一 `agent_session_id`；后续请求必须显式提交该 ID，不按 token、连接、IP、客户端名或“最近会话”隐式复用。
- Binding 更新只影响之后创建的 Session；既有 Session 使用 frozen Snapshot。
- Remote 仍是 FullAuto，不增加 confirmation、danger approval、scope DSL 或 Remote 专属 Agent 类型。
- Runtime admission 与 SQLite 提交不伪装成跨系统原子事务；Session 必须确定进入 `ready` 或可诊断 `failed`，非 ready 不执行。
- Remote operation 的状态转移与对应事件追加使用同一 SQLite 事务；同一 operation key
  的请求 payload 不一致时返回冲突；terminal outcome 吸收迟到完成；REST/MCP 返回
  Remote event cursor，而不是 Conversation message cursor。
- deadline 或 persistence failure 只能保留 `unknown` 和可诊断恢复路径，不能把未确认
  的 Provider 结果改写成成功或 rejected。

理由：服务端 Binding 让客户端保持简单，并确保 Remote、桌面和 automation 使用同一个 AgentSession/Snapshot 主链。

### D-018：轻量 Preset 与完整 Coding 边界

- 状态：`已修订（2026-09-03）`
- `chat.minimal` 不选择 Tool、Skill、MCP、Workspace、Knowledge 或业务 Context，最终模型请求保持 `tools=[]`。
- Compiler 只正向构造 Snapshot 已选择内容，不能全量初始化后再过滤。
- 当前 Coding 由 Nomi engine 保留完整的文件、patch、shell/process、VCS、MCP、
  Knowledge、Browser/Computer、SSH、历史、stream、cancel 和 artifact 能力。
- 当前验收必须经过真实 Nomi AgentSession、Provider/Model route、工具调用和 durable
  receipt；直接 Broker smoke、synthetic observation 或 Codex adapter fixture 不足以关闭。
- 当前 Nomi-core 没有 canonical on-demand activation port 时，相关选择必须保持
  unavailable；不能把 deferred capability 伪装为 initial capability 或 metadata-only
  success。
- 2026-09-05 的真实 StepFun smoke 已证明上述 Nomi-core Chat/Coding owner 链路可在
  本机执行；它使用 initial capability 闭包，并同时验证 on-demand placement 会
  fail-closed。该结果是 `SL-S3-07` evidence，不改变独立 automation、Remote
  transport 或 Desktop UI 的完成定义；这些项目随后各自验证，其中 `SL-S4-02` 已于
  2026-09-06 由真实 Tauri Desktop 走查关闭。
- `coding.codex-native` 的原生协议和工作流要求保留为未来 Codex 重新立项输入。
- 不建设 token/TTFT/P50/P95、reference-device、paired corpus、统计显著性、长期观察窗口或独立性能平台。

理由：轻量和完整都可以由结构与功能直接证明。量化性能计划在当前阶段成本高，且不决定架构正确性。

### D-019：实施并行与估算

- 状态：`已修订（05）`
- 不固定五条 workstream、ROM、coding agent 数、工程周数、HP-1/HP-2 或 whole-cohort recheck 日历。
- 工作分解以 `GLOBAL-CLOSURE-TODO` 当前开放项、真实依赖和独占写集为准。
- 并发只在写集明确分离且能减少关键路径时使用；中央 schema、Composition Root、共享 Gate 等文件由单一 integration owner 串行合流。
- 当前主机是唯一实现与 merge 主机。多个 lane 只在本机互斥写集明确分离时并行；
  中央合同、组合根、Gate、锁文件和 GLOBAL TODO 由主机串行合流。
- 不建立跨机开发任务、机器专用 Prompt/manifest/result template、远端 SHA 同步或跨机
  attestation。外部原生环境只验证冻结候选，不承担代码开发和 merge。
- 每批运行最小定向检查；broad checks 只在主要合流、Windows 核心闭环和最终 RC 执行。
- 遇到环境、真实凭据、原生主机或 harness 障碍时记录阻塞原因和人工步骤，不反复重试，也不写不优雅的测试绕过。

原固定“五流、213/314 EW、6～8 agents、29/42 周及两次 HP”的计划已撤销，因为估算和组织假设被误用为必须实现的产品合同。

理由：并发度应由当前可独立任务决定。本机互斥写集可以获得并发收益。曾考虑的跨机器
开发交接、分支同步和专用证明材料已经撤销；它们不是当前执行方式。

### D-020：Codex 最终切换与 Nomi 删除门禁

- 状态：`后续阶段`
- 当前 Nomi 不是待删除的过渡 Runtime，而是本阶段唯一产品执行内核；不执行 C9
  删除，也不冻结 Nomi 的必要产品修复。
- `FreshV4Application` 只作为显式 host boundary 保留；不提供 Engine selector、
  per-turn 切换或自动 fallback。
- 未来只有在 Codex Runtime 完成 source/build、Provider、工具、历史、取消和平台
  证据并正式接替后，才执行一次性 C9 shutdown、进程树清理和 Nomi-free RC 评估。

原“逐领域在线 sticky canary、shadow、durable handoff 后再排空”的方案已撤销，因为本地 pre-Stable fresh-v4 不需要服务器级零停机迁移平台。

理由：该流程只服务未来的 Runtime 替换窗口。当前 Nomi-core 阶段不需要也不执行
Nomi 删除，因此不会把 C9 作为当前交付阻断。

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
- Test 通过普通 AgentSession API 创建真实持久 Session，使用真实 Snapshot、当前
  Test 目标提供的资源和 FullAuto 主链。
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
- 实际 Tool schema、Model Route 和 required resource kinds；
- 当前需要的 Runtime protocol/features；
- initial/on-demand 分组；
- Snapshot 自身 digest。

具体 target resource bindings 属于 Session、伙伴、Automation 或 non-Agent
operation admission，不参与 Preset Revision/Snapshot digest。

以下全局事实不决定旧 Session 是否可执行：

- 未选择的 Package/Capability；
- 整个 target inventory 或官方模板全集；
- 决策文档 digest；
- 与当前 Session 无关的全局 schema ledger。

兼容性只在 Runtime binding 建立、实际 Capability 激活或其执行实现变化时检查并缓存。
结构不兼容时，原 Session 保持可读，执行返回 `SNAPSHOT_EXECUTOR_UNAVAILABLE`；
用户显式选择当前 target resources 后 fork 新 Session。不得静默换 Provider、重写旧
Snapshot、resolve latest 或降级 Coding。

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

- 状态：`后续阶段`

未来 Codex 切换成立后的 C9 操作顺序：

```text
停止 Nomi 新 admission
→ 取消全部内部 Nomi 工作
→ bounded application/runtime shutdown
→ kill descendant process tree
→ 对无法确认的真实外部 Effect 记 uncertain
→ 验证 Nomi process、binding、public route、release artifact 不再存在
→ 删除 Nomi
```

- 当前阶段禁止把同一 AgentSession 中途切换到 Codex Runtime。
- 禁止自动 replay/retry 外部 Effect。
- 不等待所有 owning domain reconcile 完成才删除 Nomi；uncertain 使用原 identity 留给领域处理。
- 不保留祖先 deadline 最小值、per-domain sticky canary、read-only shadow、durable Session handoff 或多维 outstanding ledger。

原在线排空方案仅保留“一旦开始删除就停止新 admission、最终清理进程树”的目标；其服务器级迁移机制已撤销。

理由：该流程只服务未来的 Runtime 替换窗口。当前 Nomi-core 阶段不需要也不执行
Nomi 删除，因此不会把 C9 作为当前交付阻断。

### D-028：三平台发布与验证策略

- 状态：`已修订（2026-09-03）`

首批 release-blocking 平台：

1. Windows Desktop x64；
2. macOS Desktop arm64；
3. Linux Desktop x64。

macOS x64 与 Linux Headless x64 保留设计兼容和后续交付入口，但不阻塞首个 Stable；未在真实原生环境验证时不得宣称已交付。

当前 Nomi-core 收口链：

```text
S0 STOP-LOSS
→ S1 FOUNDATION
→ S2 NOMI-CORE FUNCTIONAL
→ S3 NATIVE SMOKE
→ S4 FINAL RC
→ S5 STABLE
```

- Windows 完成 Nomi-core release-required 核心闭环和代表性功能/失败/进程清理。
- macOS arm64 与 Linux Desktop x64 对真实 Nomi-core 候选 Artifact 运行
  build/package/install/launch、critical capability 和 lifecycle smoke。
- 当前三平台最终候选仍是 Nomi-core RC；C9 和 Nomi-free RC 只属于未来 Codex 切换。
- Stable 原样提升已验证的 RC bytes，不重新构建另一份制品。
- `release-lock.json` 只记录当前真实 Host/Package digest；未来存在 Codex Sidecar 时
  才记录相应 Sidecar digest；`platform-result.json` 记录目标平台、实际 suite、结果
  和日志引用。
- 相同 Artifact digest 可以复用仍相关的证据；只有产品 ABI、Runtime protocol、Package 或目标平台 Artifact 改变才使对应结果 stale。
- dirty worktree 可运行 verify 作为诊断；只有正式 release evidence 要求 clean commit 和真实 Artifact。
- 原生平台结论必须来自对应真实 Host；cross-compile、Rosetta、VM、容器或静态检查只能作为 preflight。
- macOS arm64 与 Linux Desktop x64 Host 只执行冻结候选的外部原生验证；发现缺陷后由
  当前主机修复，不建立跨机开发分支或交接协议。

原“五个 native cells、固定 HP、四元 tuple、whole-cohort exact evidence 和两轮全量 Gate”已撤销，因为其验证成本超过首发产品风险，并造成反复换机和证据失效。

理由：三平台覆盖当前首发桌面用户，Windows 承担完整核心验证，另外两个真实目标环境
验证平台制品和关键能力。这样保留发布可信度，又避免把外部原生验证变成跨机开发流程，
或把所有内部测试复制成五平台笛卡尔积。

### D-029：当前产品 Runtime 与多-runtime host boundary

- 状态：`已确认（2026-09-05）`
- Web、Desktop 和 `nomicore` 的默认入口统一使用 `NomiCoreApplication`，其内部
  组合并运行 NomiFun 原有 Nomi engine。
- `FreshV4Application` 保留为显式、低成本的备用 host boundary。它可以在后续
  Runtime 接入或验证时单独组合，但不会被默认入口隐式选择。
- 一个 `AgentSession` 只能绑定一个明确的 Runtime host；不支持运行中切换、
  per-turn 切换、自动 fallback 或静默重选。
- Codex app-server 当前是协议/Sidecar 研究和未来宿主，不是已移植进 NomiFun
  的核心。只有未来完成 source/build、工具回调、Provider、历史、取消、删除和
  平台证据后，才重新评估是否接入。
- 当前 App 内的 `NomiCoreSessionOwner` 是唯一 Nomi-core Session facade；它只共享
  一个原有 Nomi `ConversationService` 实例和 runtime registry，向各业务域提供窄
  typed ports。该 facade 是组合边界，不是第二个持久化 Session aggregate，也不
  自动消除尚未迁移的 Conversation-backed compatibility。

理由：保留未来扩展所需的最小组合边界，同时让当前产品只有一个可验证的执行所有者，
避免在 Nomi-core 稳定前维护双 Runtime 主链。

### D-030：Automation 使用 host-owned typed Session boundary

- 状态：`已修订（2026-09-05）`
- Cron 的执行请求只携带用户可见消息和封闭的 `CronTurnRuntimeOverlay`。workspace、
  model、delegation policy、creation time、Session identity 等字段必须从 host 的
  最新 Session projection 解析；Cron/Session 双侧关系由单事务 CAS 绑定。
- Cron 不拥有 AgentPreset store 或 Compiler。App 注入
  `NomiCoreCronAgentPresetResolver` 实现 `CronAgentPresetResolver`；创建或更新任务时，
  resolver 在 owner scope 内取得稳定 Revision 与持久化 Snapshot，并在写入 Cron job 前
  冻结为 `agent_config.agent_snapshot`。执行、重放和普通任务编辑不重新解析 latest
  Preset；缺少 resolver、稳定 Revision 或 Snapshot 时 fail-closed。model-only Cron
  是独立显式路径。
- AutoWork 的 runtime preparation capability 由同一个 host 实例签发，并绑定 owner
  与 Session。attachment planning 产生的 snapshot token 必须在 durable admission 前
  用最新 Session projection revision 重校验；issuer、作用域或 revision 不一致时
  fail-closed。
- Gateway 的 Cron create/update/delete 在任何会话 model 补写或 Cron 数据库写入前，
  必须取得 transport-derived operation identity，再提交给 Cron 的 process-owned
  mutation waiter；调用方超时只放弃观察，不取消已经被 owner 接管的 mutation。
  当前没有 operation identity 的旧 native tool 入口只作为明确的兼容边界保留。
- AutoWork 配置写入使用 owner-scoped expected revision 和 operation identity，在同一
  `extra` 对象中只改 AutoWork-owned metadata；相同 operation 重放只接受相同配置，
  过期 writer 不得覆盖其他字段。运行中的 loop 由 per-target transition lock
  串行化，相同配置保持 no-op。
- durable receipt、accepted 等待、丢失 receipt、reconciliation 和 shutdown cleanup
  仍由 owning domain 负责；typed boundary 不将未知结果转换成成功，也不自动重放
  可能已经跨过不可逆边界的请求。

理由：Automation 的主要风险不是再增加一个通用状态机，而是防止 Cron/AutoWork
在准备、配置变化、进程中断和重放时重新取得一份不一致的 Session authority。封闭
overlay、不可伪造的 host capability、revision CAS 和每目标锁足以覆盖当前 Nomi-core
产品需要，同时保留未来 canonical Session host 的替换空间。

### D-031：领域 adapter 的真实迁移判定

- 状态：`已修订（2026-09-05）`
- 依赖审计同时报告三类事实：生产模块是否仍直接依赖旧实现、typed adapter 是否
  仍承担转换、以及测试是否仍使用 Conversation-backed fixture。不能因为第一项为
  零就把后两项抹掉，也不能把 adapter 单测当成产品级 canonical Session 证据。
- 本阶段对 `SL-S3-10` 的迁移判定是：真实 consumer crate 只能依赖自己的 typed
  port；Conversation/Runtime registry 的转换必须集中在 app-owned
  `NomiCoreSessionOwner` composition，或明确标注的 integration-test support；
  不允许在领域 crate 中保留第二份生命周期、identity map 或隐藏 fallback。
  因此“生产 legacy 清零 + 共享 host owner + 行为回归通过”可以关闭本阶段
  host-boundary 任务，而不等同于删除 Nomi-core 内部实现。
- Companion archive 的消息窗口和上下文清理已经可以由 `NomiCoreSessionOwner`
  直接提供 typed host contract；其 Conversation 元数据投影只保留在测试兼容边界。
- Channel 的生产 crate 现在只保留 Channel-owned receipt/event port；其
  Conversation-backed bridge 已移到测试支持目录。未来 canonical Session 若提供带
  operation/Session identity 的直接事件流、完整 terminal receipt、消息页和精确
  cancel contract，再评估是否删除 app host 内的转换实现。
- IDMM 的生产 crate 现在只保留 IDMM-owned `SupervisionTurnScope` 和
  `ConversationSessionPort`；Conversation scope 与 hook 的转换由 app composition
  wrapper 完成。未来 canonical Session 若提供活动 turn scope、作用域化
  continuation/failover 和 live event subscription，再评估是否进一步下沉或删除
  host conversion。
- 当前 audit 的非零退出仍是有意的阻断信号；本次审计已报告
  `production_legacy_files=0`、`transitional_adapters_with_legacy_dependencies=0`、
  `candidate=none`，所以 `SL-S3-10` 的 host-boundary 任务可以关闭。测试兼容文件
  和未来 canonical contract 缺口继续显式记录，不通过 alias 抹平。

理由：把“生产 legacy 清零”“边界转换收口”“本阶段 host-boundary 完成”和
“canonical Session 已具备完整产品语义”分开，能让并行 lane 共享同一事实，在关闭
`SL-S3-10` 的同时不掩盖 Channel/IDMM 的未来合同缺口。

### D-032：真实 Provider smoke 与凭据边界

- 状态：`已确认（2026-09-05）`
- 真实 Provider evidence 必须走当前产品的 `NomiCoreApplication`、
  `NomiCoreSessionOwner`、真实 AgentSession、Provider/Model route 和实际工具
  调用；单独的 Broker 请求、synthetic observation、fixture 或 adapter 单测不够
  关闭产品 owner。
- Windows live credential 通过本机 Credential Manager 保存和读取。secret 仅短暂存在
  于受控 Bun runner 进程；Cargo/build 阶段使用无凭据环境，构建完成后 runner 只向
  目标测试进程的 stdin 发送一次 secret，并在应用关闭后执行明文持久化审计。secret
  不得进入源码、文档、fixture、argv、日志、Cargo/build/test/application 子进程环境
  或 Git。
- 真实 smoke 的覆盖范围必须按 TODO 分项解释：本次 StepFun smoke 关闭
  `SL-S3-07` 和 `SL-S3-11`；Cron 的同时通过只作为 `SL-S3-10` 后续合同的输入，
  不能绕过其 canonical Session/adapter 缺口或当时尚未执行的 `SL-S4-02` 人工验收；
  两者后来均由独立证据关闭。Remote
  REST/MCP 的 installation Bearer、owner JWT 兼容和旧 selector query 拒绝另由
  route-gap 回归证明。
- 凭据一旦不再需要应从本机 Credential Manager 删除并向 Provider 轮换；任何
  失败只记录首个 typed phase/code/status，不通过放宽证据检查来制造 PASS。

理由：真实 Provider 验证同时涉及产品语义和 secret 生命周期。将 credential
transport 与产品 evidence 分离，既能复用本机并行验证效率，又不会把一次可运行的
provider 请求误当成完整迁移或跨传输发布证明。

### D-033：Nomi-core Remote MCP transport

- 状态：`已确认（2026-09-05）`
- MCP 的 Streamable HTTP handshake、transport `mcp-session-id` admission、
  installation Bearer 验证、四个固定工具 schema 和 bounded transport lifecycle
  由公共 `nomifun-public` transport 统一提供。
- Nomi-core 只实现 `CanonicalRemoteOperations` 适配，将 `open`、`turn`、`observe`、
  `cancel` 调用转交既有 Nomi-core Remote handler；REST 与 MCP 因而共享同一
  `NomiCoreSessionOwner`、Remote repository、owner/provenance、idempotency、
  event cursor 和关闭语义。
- 不把 Fresh-v4 `AgentPlatform` 塞入 Nomi-core，也不通过 MCP transport session
  id 创建第二个产品 Session identity。transport session 只表达连接生命周期，
  `agent_session_id` 始终是显式产品身份。
- 真实 StepFun smoke 已通过 MCP `initialize/tools/list/open/turn/observe/cancel`
  和 token revoke；transport 回归仍必须保留错误、超时、session header 和
  owner boundary 的 fail-closed 测试。

理由：公共 transport 统一协议和 admission，host adapter 只负责产品行为，能同时
降低 Fresh-v4/Nomi-core 的重复代码和后续替换成本；显式 operation trait 也让
真实 Nomi-core 语义不会被一个“看起来能连通”的伪 Platform 掩盖。

### D-035：Agent 工作台公共入口与旧路由迁移围栏

- 状态：`已修订（2026-09-06）`
- 用户可见的 Agent authoring、能力选择、保存、试用和继续使用只属于一个
  **Agent 工作台**；公共 UI 路由是 `/agent`。
- 侧边栏名称必须显示“Agent 工作台”，不能只显示“Agent”。
- 首页 Guid 是选择已保存 AgentPreset 并启动会话的产品入口，不承载 Agent authoring。
  `/agent-sessions/:agentSessionId` 可以继续提供直接 Session projection；标准首页启动
  完成后也可以使用既有 `/conversation/:id` 展示同一底层会话。
- `/presets`、`/settings/agent-presets` 和 `/settings/agent` 不再是产品入口。
  本次实现已删除这些 authoring 路由；不保留长期 redirect、旧 API 或兼容分支。
- `/settings/execution-engines` 只负责 Runtime Manager、网络和系统级设置，不承载
  AgentPreset 内容。

理由：用户需要的是一个清晰的 Agent 产品入口，而不是同时理解“设定”“Agent 设定”
和“运行时设定”。旧书签不能成为长期保留第二套入口和数据合同的理由。

### D-036：`agent_snapshot`、Revision payload/locks 与 Fresh-v4 clean cut

- 状态：`已修订（2026-09-06）`
- Conversation、Cron、Agent Execution participant 和 Execution Template participant
  的持久化执行投影统一命名为 `agent_snapshot`。该字段表示消费方无关的、不可变的
  Agent 执行快照；它不是旧 Preset resolver 的别名。
- `crates/backend/nomifun-db/migrations/061_agent_snapshot_naming.sql` 对四张表执行
  物理 `RENAME COLUMN preset_snapshot TO agent_snapshot`。不增加 compatibility
  view、双读写或按版本猜测。
- `crates/backend/nomifun-db/migrations/062_agent_preset_contribution_locks.sql`
  在 `nomi_agent_preset_revisions` 增加 `contribution_locks_json`，并要求合法 JSON
  数组。Compiler 生成的 lock 集合必须参与 `revision_digest`；不能只保存 payload
  digest。
- `preset_id` 与 `preset_revision` 在当前过渡实现中只承担 AgentPreset provenance。
  它们不能恢复旧 `PresetService`、旧 target/override 或旧 snapshot projection。
- Revision 的 canonical 内容列是 `payload_json`。Nomi 数据线通过
  `064_agent_preset_revision_payload.sql` 做物理重命名，
  Fresh-v4 baseline 直接使用该名称；不保留 `editor_document_json` alias 或双读写。
- `065_agent_preset_retirement.sql` 增加 owner-scoped `retired_at_ms` tombstone；
  `066_retire_resource_bound_presets.sql` 扫描旧 Revision/Snapshot 的资源实例字段，
  删除活动 Agent/Remote Binding 并退役对应 Preset，但保留不可变 Revision、
  Snapshot 和历史 Session。`nomifun-db/build.rs` 让新增 migration 触发 SQLx 重编译，
  当前 migration head 为 66。
- Fresh-v4 `agent_preset_templates` 只保存 `source_kind=official` 的创建 seed，不带
  `source_package_id/source_package_version`，bootstrap 不自动创建官方 AgentPreset 或
  Revision。ContributionLock 由 canonical lock 存储持有，旧 capability/skill/resource
  preset projection 表不在当前 schema 中。
- 2026-09-06 Fresh AgentPlatform E2E 暴露旧 bootstrap 仍访问 Package template source
  与旧投影表；该漂移必须直接删除，不通过兼容列、临时旧表或 fallback 修补。
- `001_v3_baseline.sql` 中旧列名是历史 schema bytes；只有 061 之后的物理 schema
  才是当前 clean-cut 运行面。

理由：snapshot 是执行事实，命名必须直接表达其消费语义；ContributionLock 是 Revision
可复核性的依赖事实，若不进入 digest，重启或来源变化后就无法判断 Revision 是否仍是
同一份用户设计。

### D-037：AP-7 验证与 06 admission

- 状态：`已修订（2026-09-06）`
- `scripts/gate-agent-v2.mjs -- ap-7` 是 AP 阶段的 admission preflight；它不运行
  provider smoke，也不因单 crate、单 UI 页面或 Gate self-test 通过而签署 AP-7。
- Gate 必须分别记录：
  1. 真实 Cargo dependency/import；
  2. 活动 API、UI、DTO、override 和 snapshot residual；
  3. 历史 baseline、删除合同和回归断言；
  4. generated API/schema inventory 是否已同步；
  5. 061/062/064/065/066 migration 与 Fresh-v4 canonical schema 是否存在且形状正确；
  6. 真实 Agent 与非 Agent consumer 的行为证据；
  7. 干净提交和签署 admission evidence。
- 已删除的 `nomifun-preset` 不再作为通用“legacy/product dependency”规则扫描对象。
  Gate 只在 Cargo dependency key、lockfile package entry 或活动 import 出现时阻断；
  root `exclude` 删除 tombstone、历史 deletion contract 和注释引用单独分类，不得
  被误报成生产依赖。
- 旧 `8e3f1eee8`/`c7f67eefb` evidence 在 2026-09-06 人工走查发现首页缺少真实
  AgentPreset selector 后被撤销。旧测试结果只保留历史审计，不能继续表示当前
  admission。
- 纠偏实现已经完成真实 Tauri Desktop 产品走查、broad checks 和当前 StepFun smoke；
  `SL-S4-02` 已关闭。新 evidence 的 `implementation_commit` 固定为
  `ef5f5380915e0d5c06004f03b66cbd1302b3fe03`，使用 `admission=admitted` 与
  `signed=true`，并在干净签署提交上通过 Gate 后关闭 AP-7。
- 手机模式不属于 `nomifun-desktop` 本期服务范围；06 仍保持独立二期边界。

理由：AP-7 的职责是防止“有类型/有 fixture/有 self-test”被误报为产品合同已经闭合。
把删除包的历史文字与真实依赖分开，既保留 clean-cut 的安全断言，也避免旧 Gate 因
删除对象本身的历史记录失真。

### D-038：首页 AgentPreset 选择与高层 Session 创建

- 状态：`已修订（2026-09-06）`
- 首页 Guid composer 上方的 pill bar 必须可见列出当前 owner 在 Agent 工作台中保存的、
  具有 `current_stable_revision` 的可执行用户 AgentPreset；它选择的是 AgentPreset，
  不是模型、Runtime 或 execution engine。
- pill bar 的 `+` 只打开 `/agent`。Agent 工作台的“使用 Agent / Start conversation”
  进入 `/guid` 并携带 `selectedAgentPresetId`，Guid 必须预选对应 Preset。
- 普通 Guid 只负责会话级消息、附件和明确属于会话的 AutoWork/IDMM 状态。
  模型、能力模式、Skills 和 MCP Capability 由选中 AgentPreset 的稳定 Revision
  拥有；Guid 不提供第二套能力覆盖控件。具体 Knowledge、Workspace、MCP/Connector
  实例属于消费目标，必须在当前会话中选择，并且不同会话互不继承。
- 标准 Session 创建请求只有 `preset_id` 与可选 `title`。客户端不得提交
  `agent_binding`、Revision、Snapshot、digest、model route 或 typed resources。
- 服务端在 owner scope 内读取 `current_stable_revision`、该 Revision 的持久化
  Snapshot，构造初始无具体资源的 `binding_version=1` AgentBinding 后创建 Session。
  目标级资源在 Session 创建后由会话交互绑定。缺少稳定 Revision、Snapshot 不匹配
  或 owner 不匹配必须 typed fail，不选择 latest、不 fallback。
- Session 创建后冻结当时的 Revision/Snapshot；之后保存新 Revision 只影响新 Session。
- Snapshot 同时冻结精确模型与 `required_resource_kinds`。Preset Session 的 UI 不显示
  模型选择器，公开 Conversation PATCH 也拒绝改写顶层模型；普通 Nomi Session 继续
  允许显式选择模型。桌面 Workspace/Knowledge 入口只读取冻结资源种类，不回查当前
  Catalog 按 Capability ID 猜测版本。
- 删除用户 Agent 时，工作台和所有新 admission 立即隐藏/拒绝它，活动
  AgentBinding/RemoteBinding 被解除；不可变 Revision/Snapshot 与已有 Session
  历史保留，已有 Session 继续从冻结产物读取能力，不依赖已退役工作台条目。
- execution engine 是 Runtime 基础设施，只能在对应系统设置中检测和管理，不得作为
  Agent pill、AgentPreset ID 或“新建会话”产品入口。

理由：用户选择的是一份完整、可复现的 Agent 能力设计，但具体知识库和工作区属于
每次使用的场景。把模型或能力重新放进 Guid 会制造第二份 Agent 配置；把资源实例
冻结进 Preset 又会阻止同一 Agent 在不同会话中使用不同资源。两者都必须避免。

### D-039：Preset Session 模型与资源种类冻结

- 状态：`已确认（2026-09-06）`
- `ResolvedSnapshotContent.required_resource_kinds` 是 canonical Snapshot 内容的一部分，
  由 Compiler 从实际 capability authority policies 汇总并参与 Snapshot digest。
- Nomi Conversation projection 将该冻结集合保存到 `AgentResolvedSnapshot`；历史
  Session 的桌面资源入口直接读取该集合，不从当前 Catalog 按 ID 合并不同版本。
- `resolved_model` 是 Preset Session 的 lead model authority。UI 不显示
  `NomiModelSelector`，自动模型 heal 不运行，公开 Conversation PATCH 修改顶层模型
  必须返回 typed 4xx；切换模型需要保存新 Revision 并创建新 Session。
- 普通 Nomi 会话不是 AgentPreset，因此继续保留模型选择器、默认模型 heal 和显式模型
  PATCH。协作模型、Workspace 和 Knowledge 等消费目标配置仍可在 Preset Session 中按
  各自边界修改，但不得替换 lead model。

理由：只锁 Revision 而允许会话改写模型，会让实际 Provider/Model 与 Snapshot
provenance 分叉；只保存 Capability ID 再回查当前 Catalog，会让升级后的资源入口重写
历史 Session。模型和资源种类都必须成为同一冻结执行事实。

## 当前阅读与实施规则

1. 先完整读取 `05-system-capability-replacement-foundation.zh.md`，再用本文追溯 D-001～D-039 的决策理由。
2. 领取和关闭工作只看 `GLOBAL-CLOSURE-TODO.zh.md`；不得从本文推断某项已经实现或通过 Gate。
3. Browser/Computer 实施必须先落 Role/Provider seam，再接具体 owner；不能在旧直连上叠加 adapter。
4. Codex Sidecar 只按未来宿主研究维护；不能继续围绕不存在的私有 patch 扩大当前
   Nomi-core Host contract，也不能把研究 evidence 当作产品完成。
5. 新实现优先删除重复 Compiler、全局 Snapshot 事实、通用 Effect 状态机、虚假 zero proof 和旧生产旁路。
6. 任何需要恢复旧固定 ROM、在线 canary、五平台首发、全量 exact-zero/evidence 或复杂 handoff 的变化，都必须重新提出产品理由并获得明确决策。
7. 读取 automation audit 时同时记录 production legacy、transitional adapter 和真实
   blocker；不得只看一个数字判断是否已经完成。
8. 06 的 Plugin/MiniApp 代码不属于本次提交；后续启动 06 时必须重新读取本 admission
   evidence 和新的 N1 checklist，不得把 06 fixture 反向当作 05 的实现来源。
