# NomiFun Agent Capability Platform v2 决策记录

> 本文件记录架构评审中的最终裁决。D-001～D-028（含 D-019）已经闭合并经用户整体确认；当前为 IMPLEMENTATION READY，本设计提交未包含生产代码，下一任务从 Contract Closure/G0 启动。

## 状态约定

- `已确认`：用户明确选择，或依照用户授权完成最优方案裁决；
- `被取代`：被后续决策替换，仅保留审计原因。

## 决策队列

| ID | 决策 | 状态 | 已确认裁决 / 当前推荐 |
|---|---|---|---|
| D-001 | 产品与内部领域命名 | 已确认 | 产品统一“Agent 设定”；内部使用 `AgentPreset` |
| D-002 | Agent 执行模式 | 已确认 | 唯一 FullAuto；YOLO 仅作旧数据/研发别名 |
| D-003 | 能力、会话、Preset 与 Runtime 的所有权 | 已确认 | NomiFun 持有平台事实；Runtime 只是受管执行器 |
| D-004 | Codex 基座与 Nomi Runtime 替换范围 | 已确认 | 唯一 Codex-derived Runtime；完整 Coding Pack；达标后删除 Nomi |
| D-005 | 第一方与第三方插件运行隔离 | 已确认 | 方案 C：普通第一方与第三方插件统一允许进程内，按 trusted code 处理 |
| D-006 | Kernel 与可插件化业务边界 | 已确认 | 方案 A：薄功能 Kernel；全部业务域进程内插件化 |
| D-007 | Package、Capability、Skill 与 MCP 的领域分层 | 已确认 | 方案 A：轻量四层；Capability 是唯一可执行能力主线 |
| D-008 | Agent 动态扩展能力的范围 | 已确认 | 方案 A：初始能力 + 按需能力；按需范围内自动激活 |
| D-009 | 内置 Agent 设定模板与场景全集 | 已确认 | 精简 A：7 个角色型模板；Research/Requirement/AutoWork 复用 Capability 与任意 Preset |
| D-010 | Agent 设定编辑器与产品导航 | 已确认 | 方案 A：单页渐进式编辑器；正确拆分设定、插件、能力、Skill、MCP 导航 |
| D-011 | 首个端到端 Vertical Slice | 已确认 | 方案 A：零工具问答 + 完整 Coding 双切片 + CI/test-only 插件 fixture |
| D-012 | v4 数据代际与 Breaking Migration | 已确认 | 方案 C：全新 v4 clean start；不迁移任何旧数据，不开发 Converter |
| D-013 | 旧数据目录处理与 Clean Cutover | 已确认 | 方案 A：同文件系统整体原子 rename 归档，再创建空 v4 root |
| D-014 | Legacy API/表/模式的删除期限 | 已确认 | 方案 A：按迁移波次同改同删；首个 v4 Stable 零兼容面 |
| D-015 | Session Event Store 与历史重放 | 已确认 | 方案 A：规范化语义 Event 为事实、Projection 可重建、Codex checkpoint 可丢弃 |
| D-016 | 第三方插件正式支持的 Phase N 范围 | 已确认 | 方案 A：Stable 冻结同链契约；Phase N1 本地安装 + 单 SDK MVP；市场最后实施 |
| D-017 | Remote 调用与 Agent 设定映射 | 已确认 | 方案 A：服务端 RemoteBinding 固定 exact revision/resources，显式 open/turn/observe/cancel |
| D-018 | 轻量 Preset 与 Coding 完整性边界 | 已确认 | 收窄 A：结构保证轻量 Chat 与完整 Coding；本次不做量化测量、baseline、benchmark 或性能 RC |
| D-019 | 实施并行工作流与 ROM 规模 | 已确认 | 方案 A：五条稳定 owner 流；213/314 EW；6–8 coding agents；29/42 active engineering weeks + 计划内 HP-1/HP-2 与必要整候选 recheck wave 的实际等待 |
| D-020 | Codex Runtime 最终切换与 Nomi 删除门禁 | 已确认 | 方案 A：内部功能 canary → Nomi 硬删除 → Nomi-free RC → 同 digest Stable；无双 Runtime fallback |
| D-021 | 旧 Conversation 概念与 AgentSession 身份关系 | 已确认 | 改良 A：唯一 aggregate/UUIDv7 为 `AgentSession/AgentSessionId`；新架构彻底删除 Conversation 技术术语 |
| D-022 | Agent 设定 Test Revision 与真实 Effect | 已确认 | 方案 A：dirty draft 自动保存普通可见 immutable revision；普通持久 AgentSession 使用真实资源执行真实 FullAuto Effects |
| D-023 | 七模板 initial/on-demand Seed 政策 | 已确认 | 改良 A：role-complete but context-minimal；精确 manifest 在实施 G0 inventory 后冻结，不再逐 Capability 向用户确认 |
| D-024 | Session 删除与 Effect 历史保留 | 已确认 | 方案 A：删除全部 Session 内容，只保留不可恢复的最小 tombstone；领域 Effect 事实不级联 |
| D-025 | 旧 Snapshot 在 v4 升级后的可执行性 | 已确认 | 方案 A：完整兼容时原 Session 继续；不兼容时历史只读并显式 fork 新 Session |
| D-026 | Remote Token 撤销语义 | 已确认 | 方案 A：request-admission fence；提交后旧 token 的新请求统一失败，不改变既有 Session |
| D-027 | Internal Canary Session 排空 | 已确认 | 方案 A：stop admission；idle 立即清理删除；accepted operation 到自身与祖先 deadline 最小值后 cancel/dispose/kill/uncertain handoff/zero/delete |
| D-028 | 正式运行平台矩阵 | 已确认 | 方案 A：Windows 连续完成 C1～C7 + pre candidate full Gate → macOS arm64 整体 Gate → 三机并行；只在 platform-stage exit 暂停 |

## 全局实施优先级

以下要求由 D-005 的用户裁决提升为本次重构全局硬约束，适用于所有后续决策和实施计划：

1. **交付速度和逻辑简单是最高优先级。** 架构只有在直接减少主链代码、适配层、状态机和调试面时才成立。
2. **安全性仍是本次开发的最低优先级，只保留系统正常运行不可缺少的最小权限边界。** 不得因为防御、合规、未来第三方威胁模型或追求完备权限体系延迟功能交付。
3. 必要权限必须是同步、确定性、无交互的一次检查：满足即继续，不满足即失败；不得引入 pending、审批队列、临时授权或恢复状态机。
4. 本期不建设插件 WASI runtime、通用 subprocess plugin ABI、sandbox、审批、Grant/Consent/Lease/Permit、插件权限引擎、签名验证、供应链信任或多层 Secret Broker。
5. 普通第一方与第三方插件都按 **trusted in-process code** 处理。插件拥有宿主进程权限是本代际的明确取舍，不声称能够隔离恶意或缺陷插件。
6. Capability/Snapshot scope 表达 Agent 当前看得到、可以组合和调用哪些功能，并保留必要的业务资源绑定检查；它不承担隔离恶意插件代码的安全承诺。
7. 不实现“Manifest 声明了权限但运行时无法强制”的安全表象；无真实执行价值的 permission/risk 字段直接删除。
8. Codex-derived Runtime sidecar 是 D-004 已确认的底层 Runtime/依赖隔离形态，不属于普通 Capability Plugin，不因 D-005 C 改为进程内。
9. 功能正确性、数据一致性、可恢复性和普通回归验证仍是交付质量要求；D-018 已明确本次不建设量化性能、统计质量或性能 RC 测量，它们也不能被包装成扩大安全架构的理由。
10. 若未来出现超出最小白名单的明确安全需求，应在本次核心重构稳定后以独立需求重新设计，不在当前 schema、API 或代码中预埋占位安全体系。

### 本期最小必要权限白名单

只允许以下权限概念进入新架构；新增任何其他权限对象必须先证明没有它系统就无法正确工作：

1. **Principal/Ownership**：已登录用户、AgentSession 与业务对象的直接归属检查；不建设通用 ABAC/policy DSL。
2. **Agent Capability Allowlist**：`CompiledAgentSnapshot` 精确列出模型可见/可调用的 Tool、Context 和 Capability；不建设运行时扩权 Grant。
3. **Typed Resource Binding**：使用明确的 workspace root、knowledge id、companion id、channel id、robot id 等业务资源绑定；只做 exact id/root 匹配，不建设复杂规则语言。
4. **Remote Ingress Authentication**：远程入口保留一个最小身份凭据并映射到同一 AgentPreset；不再叠加 query scope、token scope 与临时授权的交集系统。
5. **Provider Credential Storage/Route**：沿用集中保存与按 model route 使用凭据的必要路径；本期不抽象多层 Secret Broker 或面向 trusted plugin 的隔离协议。

除此之外的权限模式、风险等级、审批、确认、授权期限、权限继承、信任层级和插件隔离均进入删除范围。

## 已确认决策

### D-001：产品与内部领域命名

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 用户产品对象：**Agent 设定**。
- 内部可编辑领域对象：`AgentPreset`。
- 不可变版本：`AgentPresetRevision`。
- 运行实例：Agent / `AgentSession`。
- 当前执行实现：Nomi Agent Runtime；目标执行实现：唯一 Codex-derived Runtime。
- 产品和 Agent 设定不提供 Engine catalog 或 Runtime 选择器；内部只保留稳定 `AgentRuntime` Host/sidecar contract。
- 设计影响：运行实例与用户配方使用不同名称；不再使用“系统 Agent”或 `AgentDefinition` 指代完整 Agent 设定。

### D-002：Agent 执行模式

- 状态：`已确认`
- 用户裁决：只使用 YOLO/全自动模式，删除其他权限模式、审批队列、确认卡和 AgentExecution plan approval；审批未来有真实需求时再通过新 schema 引入。
- 设计解释：FullAuto 不保留审批状态机。Agent 自动使用启动前由 Snapshot 选择的 capability/tool/resource 组合；未选择的能力不注册或返回 unavailable。
- 数据影响：v4 不保存 `session_mode`、`permission_mode`、`autonomy`、Grant、Consent、Capability Lease 或 Invocation Permit。
- 实施影响：Phase 0A 在新 `AgentRuntime` contract 前删除全部 mode/approval/confirmation surface。

### D-003：能力、会话、Agent 设定与 Runtime 的所有权

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 所有权：Capability、Session Event、`AgentPreset`、`RuntimeAuthority`、Secret 与业务数据全部由 NomiFun 持有。
- 执行边界：迁移期 Nomi Agent Runtime 与目标 Codex-derived Runtime 都只能通过受管 Host ports 使用当前 Snapshot 的 Model、Context、Tool 与 Event，不得直接取得全量 DB、Secret 或 `AppServices`；迁移结束后删除 Nomi 路径。
- 数据边界：Knowledge、Memory、IM、Browser、Robot 等领域数据保持 NomiFun 唯一权威；Runtime 的内部 thread/session id 只能作为运行绑定，不能成为产品会话主键或事实源。
- FullAuto 边界：Runtime 接受已经编译冻结的能力与资源范围；范围内自动执行，范围外失败，不建立第二套审批、Grant 或权限状态。

### D-004：Codex 基座与 Nomi Runtime 替换范围

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 最终目标：以 Codex 源码二次开发形成唯一 Codex-derived Runtime，完成全场景迁移后完全删除当前 Nomi Agent 执行循环。
- 产品边界：不提供 Runtime/Engine 选择器；Pi 与 DeepSeek Harness 仅作研究参照。
- 所有权边界：此决策不改变 D-003；平台事实和业务能力始终归 NomiFun。

源码调查已经确认：Codex 适合作为新的 turn/runtime 基座，但不是可原样接入所有 NomiFun 场景的成品。它具有成熟的 Thread/Turn/Item、流式事件、steer/interrupt、恢复、compaction、并行/延迟 Tool 与跨平台进程治理；同时仍存在 Coding 默认上下文、Responses-only 模型协议、私有 rollout 以及实验性 dynamic tools 等边界。

| 方案 | 替换方式 | 长期结果 |
|---|---|---|
| A（已确认） | 在独立上游跟踪仓库维护浅层 Codex fork，构建固定版本的本地 Runtime sidecar；Coding 设定保留完整 Codex-native Coding Pack，其他设定按 Snapshot 裁剪；NomiFun 再通过版本化 stdio 协议、Capability MCP Gateway 和本机 Responses Bridge 接入系统能力与非 Responses 模型。先完成全场景 conformance/canary，达标后删除 Nomi Agent Runtime | 最终只有一个 Codex-derived Runtime；产品不提供 Runtime/Engine 选择器；Codex Coding 能力不被通用化重写削弱，同时依赖、崩溃和升级与 NomiFun 主进程隔离 |
| B | 将 `codex-core` 及其 workspace 直接嵌入 NomiFun Rust 进程并深度修改 ThreadStore、Model、Tool 与 Context | 控制力最强，但 native SQLite 依赖冲突、workspace 规模、构建体积和高频上游合并会形成新的核心债务 |
| C | Codex 只负责 Coding，Nomi 继续负责普通会话、伙伴、客服、Robot 等 | 短期风险较低，但永久保留两套 Runtime、会话恢复、工具协议和故障路径，不满足“完全替换”目标 |

方案 A 中“完全替换”的精确范围：

- 最终删除 `nomi-agent` 的执行循环、Bootstrap、Manager/Factory、私有 session 与 approval/mode 路径；
- 保留并重命名可复用的 provider 协议适配、Capability Gateway、RuntimeAuthority 与领域服务；它们不是旧 Runtime；
- Robot 音频链路、IM 长连接、AutoWork DAG、IDMM、创意任务队列仍由各自 NomiFun 插件持有，Codex-derived Runtime 只替换它们调用的推理/工具 turn loop；
- Codex thread id/rollout 只作可丢弃或可重建的 Runtime binding/checkpoint，不成为 AgentSession 或业务事实源；
- 迁移期 Nomi 只经 disposable migration coordinator 参与 fresh-v4 internal baseline/replay/functional canary Session；Nomi 或 Codex 可以是整个 Session 固定的唯一真实 primary，secondary 只能只读 shadow 或消费 recorded/simulated 结果。问题 cohort 只能停止接收新 Session，不能切换已运行 Session 或在 Effect 后 fallback。D-020 A 要求全场景后先硬删除 Nomi，再生成 RC；
- Pi 与 DeepSeek Harness 不再建设产品 adapter，只保留 loop、scope、插件生命周期和测试语义的研究参照。

### Codex Coding 能力完整迁移是硬性目标

选择 Codex 的首要目的之一就是获得其成熟 Coding Agent 能力，不能只抽取一个通用 Tool Loop 后丢掉最有价值的部分。Coding Agent 设定必须尽可能原生保留并持续跟进：

- Codex 针对 Coding 模型优化的基础指令、上下文组织、模型特性与 Responses 语义；
- workspace/repository 识别、AGENTS.md 分层规则、Git 状态与 worktree 工作流；
- shell/terminal、统一进程执行、stdin、文件读取/搜索、`apply_patch`、图片输入与结果截断；
- plan/goal、长任务、取消、steer、resume、fork、rollback、compaction 与错误恢复；
- Skills、Plugins、MCP、Hooks、Web/Browser/Computer 等 Coding 工作中可选扩展；
- Code Mode、Tool Search/deferred tools、并行工具调用、子 Agent/多 Agent、代码审查和验证反馈循环；
- Codex 上游后续新增且通过 NomiFun conformance 的 Coding 能力。

这些能力以第一方 `coding.codex-native` Capability Pack 的形式由 Agent 设定整体选择，底层优先复用 Codex 原生实现和原生事件语义，不为了形式统一而全部降级重写成 MCP 工具。NomiFun 负责声明、范围编译、workspace lease、生命周期、审计和产品投影；Codex Runtime 负责原生执行。

非 Coding 设定使用同一个 Runtime 的精简 Profile：完全替换基础指令，并关闭未选择的 workspace、AGENTS、Git、Shell、Patch、Coding Skills 与子 Agent。这种“同一 Runtime、不同能力 Profile”不是多 Engine，也不产生新的用户权限模式。

模型通道也必须保真：OpenAI/Codex 原生 Responses 模型优先使用不丢失 reasoning、tool-call、prompt-cache 与 stream item 的原生通道；Anthropic、Gemini、OpenAI Chat、Bedrock 等再经兼容 Bridge。不得为了统一 provider 而降低 Codex Coding 模型的效果。

实施文档必须加入以下硬门禁：全 Provider 协议兼容、零工具轻问答、所有生产场景、FullAuto 无等待状态、取消/恢复/崩溃、Effect 幂等、上下文压缩、跨平台进程清理，以及 Coding capability/native-feature/Responses exact-set 与正常功能 conformance。D-018 已删除性能和统计质量盲评。任何未被 Snapshot 选择的 Runtime 内建工具仍可见或可执行，均视为架构级失败。

### 与 Agent Preset / Capability 插件化共同实施

- Codex Runtime 替换与 Agent Preset/Capability 平台是同一个重构计划，不先围绕旧 Nomi Runtime 建成长期插件体系，也不先把 stock Codex 作为第二套能力平台接入；
- 先锁 `AgentPresetRevision`、`CompiledAgentSnapshot`、`RuntimeProfile`、`CapabilityDescriptor`、`SessionEvent`、Model Route 和 Runtime Command/Event 公共契约；
- 并行建设两个首批端到端切片：完整 `coding.codex-native` 与零工具普通问答，分别验证能力上限和最小固定成本；
- 新功能只进入新 Capability/Runtime 路径；Nomi 在迁移期只经 disposable coordinator 参与 fresh-v4 internal baseline/replay/functional canary，且只有 session-sticky primary 可真实执行，secondary 只读或消费 recorded/simulated 结果；问题 cohort 只停止新 Session admission，不构成 Runtime fallback；
- Agent 设定编译器正向生成 Codex Runtime Profile，Runtime 不得自行扫描或加载 Snapshot 外的系统能力。

### D-005：第一方与第三方插件运行隔离

- 状态：`已确认`
- 用户裁决：采用方案 C。
- 首要理由：早期保障开发进度，以最少策略、最小运行模型和最快交付完成重构；除上文最小必要权限白名单外，安全不得增加主链复杂度或降低交付效率。

| 方案 | 运行模型 | 主要影响 |
|---|---|---|
| A | 按信任与故障域分层：Rust Kernel 和轻量、强耦合的第一方领域插件可进程内；重型第一方组件使用 sidecar；第三方默认 WASI/subprocess | 隔离更强，但需要 Host ABI、ProcessSupervisor、State/Secret Broker 与多套打包、调试和生命周期机制 |
| B | 除最小 Kernel 外，第一方和第三方插件全部 WASI/subprocess | 隔离最强、卸载清晰，但本地 RPC、数据复制、调试、事务和高频 Context/Tool 调用成本最高，许多第一方领域服务需要被远程化 |
| C（已确认） | 普通第一方和第三方插件统一允许进程内执行并视为 trusted code；本期只维护一套 Plugin Host、注册、调用和生命周期路径 | 开发、调试和调用最直接，交付最快；接受插件可访问宿主进程且故障可能影响主程序，不为此增加隔离系统 |

方案 C 的实施影响：

- 删除目标设计中的 WASI Plugin Host、通用 subprocess Plugin Host、插件 sandbox、permission enforcement、Grant/Consent/Lease、签名校验和多层 Secret Broker；
- `PackageManifest` 只保留加载、依赖、贡献、配置、版本和生命周期所需字段，不承载本期不执行的安全声明；
- Plugin Manager 在应用启动时直接完成 discover → load → register → start，应用退出时 stop/drop；本次 Stable 的 bundled inventory/config 变化通过构建或重启生效，不维护热卸载事务。用户安装、替换、停用和移除由 D-016 延后到 Phase N1，并同样只在重启后生效；
- Capability Registry 负责模型可见能力和产品组合，不充当恶意插件隔离层；
- 插件安装即表示用户信任其代码；本期不尝试限制其宿主访问能力；
- Codex Runtime 继续使用已确认的 sidecar，原因是复用其完整上游产物和解决依赖边界，而不是本期插件安全策略。

### D-006：Kernel 与可插件化业务边界

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：只保留无法由普通插件自举的薄功能 Kernel，其余业务系统统一迁为进程内插件。

| 方案 | Kernel 范围 | 主要影响 |
|---|---|---|
| A（已确认） | 薄功能 Kernel：App bootstrap、DB/migration、最小 Principal/Ownership、AgentPreset Compiler、Capability Registry/allowlist、Agent 会话唯一事实与基础事件、Codex Runtime Client/Supervisor、ChatModelBroker/集中凭据路由、最小 Remote auth、基础 Event Bus；Knowledge、Memory、Browser、Computer、IM、Robot、Creative、Requirement、AutoWork、IDMM 等全部作为进程内插件 | 满足全面插件化目标，同时只保留一套进程内调用模型和一组同步权限检查；需要一次性拆开当前 `AppServices`/Factory 循环依赖 |
| B | 极薄 Kernel：除 bootstrap 和最小 Plugin Manager 外，DB、Session、Preset、Model、Event 也全部插件化 | 理论组合性最高，但启动图、跨插件事务、身份和循环依赖最复杂，反而降低交付速度 |
| C | 厚 Kernel：Conversation、Knowledge、Memory、Browser、Computer、IM、Robot、Creative、AutoWork 等继续作为固定核心服务，只把少量外部扩展插件化 | 改造量最小，但 Agent 设定无法真正组合大部分系统能力，旧 Factory/AppServices 耦合会继续存在 |

已确认采用 A，因为它是满足“系统能力全面插件化”的最小结构：Kernel 只保留无法通过普通插件自举的功能事实、启动骨架和五项最小必要权限检查；所有业务域共享同一进程、同一类型和直接函数调用，不引入远程化、审批或隔离层。

已确认的 Kernel 清单：

- App bootstrap 与 Composition Root；
- SQLite 连接、migration 和基础事务；
- 五项最小同步权限检查；
- AgentPreset Compiler；
- Capability Registry；
- Agent 会话唯一事实与基础事件；唯一 aggregate 为 `AgentSession`，不另设聊天容器、映射或第二历史事实源；
- Codex Runtime Client/Supervisor；
- ChatModelBroker、Provider 路由和集中 credential reference；
- 基础 Event Bus；
- Plugin Manager 自身。

必须迁为统一进程内插件的业务域：Knowledge、Memory、Companion、Browser、Computer Use、IM/Channel、Customer Service、Robot、Creative Studio、Requirement、Auto Work、Cron、IDMM、AgentExecution、SSH、Office、Webhook，以及后续新增业务系统。

迁移完成后，旧 Factory、`GatewayDeps`、`AppServices` 业务 service bag 和各入口手工装配路径必须从正常运行依赖图删除；不能在薄 Kernel 中重新建立等价的 God Service。

### D-007：Package、Capability、Skill 与 MCP 的领域分层

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：保留 Package、Capability、Skill、MCP 四个真实概念，但只有 Capability 代表 Agent 可执行能力；唯一 Codex-derived Runtime 已由 D-004 固定。

| 方案 | 对象模型 | 主要影响 |
|---|---|---|
| A（已确认） | 轻量四层：`Package` 是安装/版本/依赖/分发单位；`Capability` 是 AgentPreset 可选择的稳定原子能力；`Skill` 是模型可读的说明/工作流及配套 references/templates/scripts 资源，可声明所需 Capability；`MCP` 是外部工具来源与传输，发现的 Tool materialize 为 Capability。Codex-native feature 也直接注册为第一方 Capability/Pack | 保留真实语义差异，同时只有 Capability 进入 Agent 可执行能力主链；实现 wiring 只用 exact typed `ServiceKey`，不建设 RuntimeContribution、独立 Service catalog、Provider/Consumer graph 或 Engine catalog |
| B | 全部合并为单一 `Plugin` 对象，Package、Skill、MCP server/tool 和 Capability 使用同一个 ID、版本和生命周期 | 表面对象最少，但一包多能力、Skill 无执行实现、MCP 动态工具等事实会被迫塞入大量 optional 字段，Preset 与安装状态重新耦合 |
| C | Package、Capability、Skill、MCP、Runtime Contribution、Service Definition/Provider/Consumer 全部建立独立版本目录和依赖图 | 表达力最强，但 catalog、resolver、UI、迁移和调试面最大，会重新引入已删除的抽象复杂度 |

方案 A 的简化规则：

1. AgentPreset 只直接选择 Capability、Skill 和资源绑定，不直接选择 Package 或 MCP transport；
2. Package 安装后把 contributions materialize 到 Capability/Skill/MCP source 三类轻量目录；
3. 一个 Package 可贡献多个 Capability，一个 Capability 也可以由内置代码或一个已安装 Package 提供；
4. Skill 提供 instructions、references、templates、examples 和可选 script 资源，但 script 只能由 Agent 通过已选择的 Shell/Process/专用 Capability 执行；Skill 不注册 Tool、不自动运行代码，也不因为被选中而获得未选择的 Capability；
5. MCP server 是连接配置和工具来源；每个发现/固定的 MCP Tool 使用 canonical key 注册为 Capability，避免 Native/Gateway/MCP 三套工具身份；
6. `coding.codex-native` 是第一方 Capability Pack，内部原生 handler 不形成新的 Runtime Contribution catalog；
7. 不建设 virtual provides、provider/consumer graph、条件依赖 DSL 或跨目录复杂求解；依赖只允许 Package required dependencies 与 Capability required keys。
8. Package 间若需要直接 Rust service，只声明 exact `provides_services/requires_services: ServiceKey<T>`，启动期要求单 Provider；它是实现 wiring，不是第五类用户产品对象或独立 catalog。

### D-008：Agent 动态扩展能力的范围

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：AgentPreset 固定“初始能力 + 按需能力”两个集合；按需集合内由 Agent 自动激活，集合外立即失败。

| 方案 | 动态行为 | 主要影响 |
|---|---|---|
| A（已确认） | AgentPreset 将能力分为“初始能力”和“按需能力”。初始能力创建 Session 时直接加载；按需能力在 Snapshot 中预先解析但不进入初始 Prompt/Tool 面，Agent 命中需求后自动激活。Snapshot 外能力立即失败并提示编辑 Agent 设定 | 保留 Agent 自主扩展和低上下文成本，同时 Preset 仍能准确表达这个 Agent 的能力范围；需要一个极小 `capability_search` 和 turn-boundary generation 更新 |
| B | Agent 可以从所有已安装 Capability 中任意搜索和激活，AgentPreset 只定义默认能力 | 自由度最大，但 AgentPreset 不再真正定义 Agent，资源绑定和依赖要在运行中临时解析，行为、性能和复现更难预测 |
| C | 不允许运行中扩展；所有能力必须在 Session 创建时一次性固定和加载，修改后新建 Session | 实现最简单、运行最确定，但会让复杂 Agent 初始工具面重新变大，也不满足用户要求的 Agent 自主按需扩展 |

方案 A 的最小实现：

1. `AgentPresetRevision` 只保存两个集合：`initial_capabilities[]` 与 `on_demand_capabilities[]`；
2. Compiler 在创建 Snapshot 时一次性校验两个集合的 Package 可用性、required Capability、模型兼容性和资源绑定；运行中不重新求完整依赖图；
3. 初始 Prompt 只包含 initial Tool/Context；按需集合只生成名称、短描述和用途标签的紧凑搜索索引；零工具问答可以连 `capability_search` 都不加载；
4. Agent 通过 `capability_search` 找到按需能力后自动激活，不询问用户、不写 Grant/Lease/Permit；
5. 激活只在当前 turn 结束与下一次模型请求之间生效，更新 `active_set_generation`，避免同一次响应中工具列表突变；
6. 激活后的能力保持到 Session 结束或 Runtime 重建；本期不建设模型侧 release/降权状态机；
7. 按需集合以外的能力返回 `CAPABILITY_NOT_IN_PRESET`，Agent 可提示用户编辑或 fork Agent 设定，但不能自行安装 Package、修改 Preset 或扩大集合；
8. `coding.codex-native` 可把 Browser、Computer、外部 MCP、Creative 等放入按需集合；普通问答保持空集合或极小集合。

已确认采用 A，因为它能保持产品可控和结构轻量：AgentPreset 继续代表一个明确的 Agent，同时避免把所有可能能力都注入简单任务；实现只需要两个静态集合、一个短索引和一次 turn-boundary 激活，不引入审批、授权平台或量化性能工作。

### D-009：内置 Agent 设定模板与场景全集

- 状态：`已确认`
- 用户裁决：采用方案 A，但删除 Research、需求分析和 AutoWork 执行三个能力重复的模板，最终保留 7 个官方角色型 Agent 设定。
- 归并原则：Research 改为 Capability Pack；Requirement 平台和 AutoWork/Cron 选择并执行任意 exact AgentPreset revision，不再拥有专属 Agent 类型。

| 方案 | 内置模板策略 | 主要影响 |
|---|---|---|
| A（已确认并精简） | 预装 7 个角色型模板：轻量问答、通用助理、Coding、伙伴、Robot、客服、创意工坊。Research、Requirement、AutoWork、IDMM、IM、Cron、Remote 等使用 Capability/业务绑定/trigger，不建设专属 Agent | 覆盖真正有独立 Persona/交互体验的主要场景，删除能力重复模板；用户可 fork 模板自定义 |
| B | 只预装 4 个通用模板：轻量问答、通用助理、Research、Coding；伙伴、Robot、客服、创意、需求、AutoWork 全由业务页临时组合或用户自己创建 | 模板最少，但业务开箱体验弱，各入口容易重新手拼能力并产生不一致默认值 |
| C | 每个业务系统和技术能力都预装独立 Agent，包括 IDMM Agent、IM Agent、Cron Agent、Remote Agent、Browser Agent、Knowledge Agent、Memory Agent 等 | 覆盖最显式，但导航和模板数量膨胀，transport/middleware/capability 与 Agent 角色混淆，重新形成“系统 Agent”概念债务 |

方案 A 最终确认的是下列 7 个 **key 与角色边界**，并未在 D-009 中确认 initial/on-demand seed。D-023 后续确认了 role-complete but context-minimal 的 Seed 政策；精确 Capability ID、binding schema 与 initial/on-demand partition 仍须在实施开始完成 inventory 后写入唯一 `OfficialPresetSeedManifest`，不得从本表推导具体集合。

| Key | 用户角色 | D-009 已确认的角色边界 | 与 D-023 的关系 |
|---|---|---|---|
| `chat.minimal` | 轻量问答 | 确定性零工具 Chat | exact-empty 已由 D-018 固定 |
| `assistant.general` | 通用助理 | 非 Coding 的可组合通用角色 | 服从 D-023 Seed 政策；exact manifest 在实施 G0 冻结 |
| `coding.codex` | Coding Agent | 完整 Codex Coding 角色 | `coding.codex-native` union 不得退化；exact partition 在实施 G0 冻结 |
| `companion.default` | 伙伴 | Companion Persona/Memory 角色 | 默认覆盖角色常用 Knowledge、Memory、IM 等；exact manifest 在实施 G0 冻结 |
| `robot.default` | 实体机器人 | Robot turn/device 角色 | 服从 D-023 Seed 政策；exact manifest 在实施 G0 冻结 |
| `customer-service.default` | 客服 | 客服对话/知识角色 | 服从 D-023 Seed 政策；exact manifest 在实施 G0 冻结 |
| `creative-studio.default` | 创意工坊 | Canvas/Asset/Generation 角色 | 服从 D-023 Seed 政策；exact manifest 在实施 G0 冻结 |

不作为独立 Agent 模板的对象：

- **IDMM**：监督任意 AgentSession/AutoWork run 的 middleware/Capability；
- **IM/Channel**：把某个 Channel/customer/group 绑定到上述某个 AgentPreset revision；
- **Cron**：定时触发任意 exact AgentPreset revision；
- **Remote**：为任意 AgentPreset 提供远程 ingress 或 execution environment；
- **Research**：`research.core` Capability Pack，可放入通用助理、Coding 或自定义 Agent 的 initial/on-demand 集合；`research.web` 只保留为待删除的 legacy AgentPreset key；
- **Requirement**：业务插件和资源绑定；需求平台选择任意 exact AgentPreset revision，并要求该 revision 包含所需 Requirement Capability；
- **AutoWork**：持久 runner/workflow；执行用户为该 run 选择的 exact AgentPreset revision，不拥有 `autowork.executor` 专属 Agent；
- **Browser、Computer、Knowledge、Memory、MCP、SSH、Office、Webhook**：Capability/Package/resource，不是人格或角色。

采用精简 A 的理由是消除重复：官方模板只表达存在明显 Persona、交互方式或默认能力差异的角色；Research、Requirement、AutoWork 的能力可以被通用、Coding 或用户自定义 Agent 复用，不再复制模型、Skill、Memory、Browser 和 Coding 配置。

### D-010：Agent 设定编辑器与产品导航

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：使用单页渐进式编辑器完成创建、fork、测试和保存 Revision；Package、Capability、Skill、MCP 回到各自管理入口，永久删除“设定市场”概念。

| 方案 | 编辑器与导航 | 主要影响 |
|---|---|---|
| A（已确认） | 单页渐进式编辑器：顶部编辑身份/Persona/模型，中部用“初始能力/按需能力”双栏管理 Capability，并选择 Skill/资源；底部固定 Preview/Test/Save Revision。高级 Inspector 默认折叠。`设定` 入口只管理 Agent 设定；Package、Capability、Skill、MCP 分别进入插件/能力/Skill/MCP 管理页 | 操作路径最短，适合频繁调试和本次快速交付；需要控制单页信息密度 |
| B | 四步向导：基本信息 → 模型/Persona → Capability/Skill → 资源/Preview，每次编辑按步骤前进 | 新手引导较清晰，但频繁调试和修改单项配置需要反复切步骤，复杂场景容易形成很长向导 |
| C | 高级 YAML/JSON 编辑器为主，图形界面只做只读预览 | 开发最快、表达最完整，但普通用户难以使用，字段演进和错误定位成本高，不符合产品化目标 |

方案 A 的页面结构：

1. **Agent 设定列表**：内置 7 个模板、我的设定、业务正在使用的绑定；内置模板只读，可一键 fork；
2. **身份与模型**：名称、头像、用途说明、Persona/instructions、Chat model route；不显示 Runtime/Engine 或权限模式；
3. **能力**：左侧 initial、右侧 on-demand；支持搜索 Capability/Pack、查看来源 Package、最终 Tool/Context contributor exact-set、数量和 digest；不展示预计性能或 token 成本；
4. **Skills**：独立选择 instructions/resources，显示 required Capability 是否已经在两组中；缺失只提示，不自动改写；
5. **资源绑定**：按已选 Capability 动态显示 workspace、KB、companion、channel、robot、canvas、customer、remote connection 等必要字段；
6. **Preview**：显示最终 initial/on-demand/active-at-start、Tool/Context 摘要、缺失依赖和资源；不展示权限风险或安全评分；
7. **Test**：dirty draft 先自动保存一个普通、可见的 immutable `AgentPresetRevision`，clean draft 复用当前 Revision；随后通过普通 `POST /api/agent-sessions` 创建持久 UUIDv7 AgentSession，经正式 Session execution port 与 Runtime/Event contracts 使用真实 typed resources、真实 FullAuto Tool/Effects 运行一条消息，并展示流式输出、Tool 调用、实际 Tool/Context contributor、Snapshot digest 和 `CAPABILITY_NOT_IN_PRESET` 等配置错误；Test 历史的删除与保留服从 D-024；
8. **Save Revision**：每次保存创建不可变 revision；当前业务绑定不被静默改写，用户再显式选择是否跟随新 revision。

产品导航固定分工：

- **设定 → Agent 设定**：只管理 AgentPreset/template/revision/binding；不再出现“设定市场”；
- **插件**：Package 安装、启停、版本、来源；
- **能力**：统一 Capability/Pack catalog 与来源诊断；
- **Skills**：Skill 内容、资源和 required Capability；
- **MCP**：Server connection、tool discovery 与 materialized Capability；
- **各业务页面**：只选择 exact AgentPreset revision 和业务 resource binding，不再拼模型 + Skill + Knowledge + bool 开关。

已确认采用 A，因为开发和使用路径都最短：同一页面可快速对照 initial/on-demand、Skill 和资源，符合本次“逻辑简单、调试快速”的优先级；高级细节折叠后不会强迫普通用户理解 Snapshot、RuntimeProfile 或 ServiceKey。

### 第三方插件扩展缝：本期预留，Phase N 正式交付

用户要求最终支持第三方插件的挂载、配置、选择和使用，但不得因此拖慢本次核心重构。固定采用以下边界：

**本期必须完成的低成本预留：**

1. `PackageManifest`、`PluginRegistration`、`PackageRef`、Capability/Skill/MCP contribution 不得使用 first-party-only enum 或硬编码业务 switch；
2. Plugin Manager 接受通用 factory registration，第一方内置 Package 必须完整走与未来第三方相同的 registration/materialization 路径；
3. Package 支持通用 `config_schema + config_json` 和 namespaced `PluginStateStore`，AgentPreset/Runtime 不读取插件私有结构；
4. materialized Capability/Skill/MCP 卡片携带 source Package metadata，AgentPreset 的选择与 Runtime 调用不区分第一方/第三方来源；
5. 保留稳定的挂载链：`Package mount/register → config → materialize contributions → AgentPreset select → Snapshot → Runtime invoke`；
6. 仓库内提供一个 compiled sample plugin fixture，证明上述链路可运行；它不是用户插件产品功能，也不进入普通 UI；
7. 数据/API 不允许通过 `match builtin_package_id`、逐业务字段或 Package source 判断改变执行语义。

**D-016 已把整体重构 Stable 之后的 Phase N 收束为依赖有序的三段：**

- Phase N1：用户显式选择本地目录/压缩包并安装到唯一 managed root，schema 配置、安装/替换/停用/移除在重启后生效，只发布一个 SDK/entrypoint profile 与 exact host-contract conformance；
- Phase N2+：根据真实插件反馈增加第二 SDK、调试器、依赖获取/更新、namespaced state migration compatibility 与兼容/弃用政策；
- Marketplace 最后：只有 N1/N2 的安装、更新和兼容 contract 稳定后，才建设 catalog/search/download/publisher/distribution/market；
- Hot reload 不属于任何默认阶段，最后单独评估，也可以永久不做。

Phase N 不得反向改变已经稳定的 AgentPreset、Capability、Skill、MCP、SessionEvent 或 Runtime 协议；如果当前设计只能通过重写这些核心对象才能支持第三方插件，就说明本期扩展缝设计失败。

### D-011：首个端到端 Vertical Slice

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：并行交付 `chat.minimal` 零工具问答与 `coding.codex` 完整 Codex Coding，并以 CI/test-only sample Package 验证第三方就绪契约。

| 方案 | 首批交付 | 主要影响 |
|---|---|---|
| A（已确认） | 两个用户切片并行：`chat.minimal` 零工具问答 + `coding.codex` 完整 Codex Coding；另加一个非用户可见 sample Package fixture，贯通 register/config/materialize/select/invoke | 同时证明最小固定成本、最大 Coding 能力和第三方就绪契约；工作面较大，但最早暴露 Runtime/Profile 两端的问题 |
| B | 只先交付 `coding.codex`，零工具与通用场景随后迁移；sample fixture 仍做 contract test | 最快展示核心 Coding 价值，但可能把 Codex 重上下文和 native tools 假设固化进通用 Runtime，之后再拆成本更高 |
| C | 先只交付 Kernel、Package/Capability 数据模型和设定编辑器，Codex 用户会话下一阶段再接入 | 平台代码可先稳定，但缺少真实 Runtime 闭环，容易产生无法运行的抽象和返工，用户也看不到早期价值 |

方案 A 的完成定义：

1. `chat.minimal` 使用最终 AgentPreset Revision、Snapshot、Codex sidecar、ChatModelBroker 和 SessionEvent，模型可见 Tool/Capability index 精确为 0；
2. `coding.codex` 使用相同主链并完整展开 `coding.codex-native`，覆盖 workspace/AGENTS/Git/Shell/PTY/File/Patch/Skill resources/Tool Search/子 Agent/Review/验证/恢复；
3. 两者都使用最终单页 Agent 设定 Preview/Test/Save Revision 流程，不创建测试专用配置格式；
4. compiled sample Package 使用与内置 Package 相同的 `PluginRegistration`，提供一个可配置 Capability 和一个 Skill/resource，能在设定编辑器测试 fixture 中被选择并由 Runtime 调用；
5. sample fixture 不提供用户安装入口、不建立市场或 SDK 发布承诺，删除 fixture 也不影响生产功能；
6. 两个切片和 fixture 均不经过 Nomi Factory、`GatewayDeps`、业务型 `AppServices` 或 legacy `conversation.extra`；
7. 失败、重启、取消、resume、on-demand activation、Provider error 和配置错误均使用最终 SessionEvent/错误模型。

已确认采用 A，因为它用最少的三个哨兵同时验证本次重构最重要的三个目标：简单问答保持结构轻量、Codex Coding 足够完整、未来第三方 Package 不需要重写核心；这里不增加 D-018 已删除的量化性能测量。

三个哨兵都是正式架构门禁：必须使用最终单页 Editor、Preview/Test/SaveRevision、v4 数据结构、AgentPreset Compiler、Snapshot、Capability Registry、Codex Runtime、ChatModelBroker、SessionEvent 与错误模型；不得创建测试专用 Preset schema、mock Runtime 主链、临时表或 legacy Factory/GatewayDeps/AppServices 捷径。客服和其他业务域只能在双切片通过后进入后续 Domain Wave。

### D-012：v4 数据代际与 Breaking Migration

- 状态：`已确认`
- 用户裁决：采用方案 C，以最快交付为优先。
- 目标：创建全新 v4 空数据代际，只支持 fresh start；不开发 v3→v4 Converter，不导入旧用户数据。

| 方案 | 数据策略 | 主要影响 |
|---|---|---|
| A | 创建独立 v4 data generation 和干净 baseline schema；冻结 v3 为只读迁移输入，使用一个 whole-dataset converter 把所有受管数据写入 v4，校验通过后切换。Stable Runtime 只读写 v4，不长期 dual-read/dual-write | 能删除旧表、旧字段、旧事件和多事实源，最终模型最干净；需要完整 inventory、转换器、校验和切换演练 |
| B | 在当前 v3 SQLite 和 side stores 上继续追加 migration、兼容列、映射表与 facade；迁移期长期 dual-read/dual-write，逐模块慢慢切 | 单步变更较小，但旧 Preset/Agent/session/permission/Factory 语义会渗入新模型，兼容层难以删除，数据真相继续分裂 |
| C（已确认） | 新 v4 只支持全新安装；升级用户启动空数据根，旧数据库、文件和 Agent 数据不进入新系统 | 开发最快、schema 最干净；接受用户需要重新配置和旧数据不可在新版本使用 |

方案 C 的实施影响：

1. 新建独立 v4 baseline schema 和新 data root，直接表达 Thin Kernel、四层 Package/Capability/Skill/MCP、七模板、initial/on-demand、SessionEvent、Codex binding/checkpoint 与业务插件 ownership；
2. v4 首次启动只 seed 七个官方模板、bundled Package registrations 和必要系统配置；不读取旧 DB、Nomi session、Preset、Skill、MCP、Knowledge、Memory 或业务 side store；
3. 不实现 whole-dataset inventory、Converter、ID mapping、conflict report、legacy import、dual-read、dual-write、compatibility view、旧字段 fallback 或 migration replay；
4. 旧 Conversation/Message、AgentPreset、Knowledge、Memory、Companion、Provider connection、Requirement、AutoWork、Cron、IDMM、AgentExecution、Robot、Channel、Creative、Terminal/SSH 等数据不会出现在 v4，用户需要重新创建和配置；
5. 现有 published migration 文件保持原样，只作为旧版本历史；v4 Runtime 不修改、不执行也不链接旧 migration lineage；
6. 旧 Nomi session JSON/index、legacy SQLite 和其他 side files 在正常 v4 Runtime 中完全不可达；不得保留只读 decoder、import API 或隐藏兼容任务；
7. 开发、测试、Beta 和 Stable 都以 fresh v4 data root 为准；测试不得使用转换后的旧 fixture 冒充新数据；
8. D-013 已确认只执行同文件系统 whole-root atomic rename archive；不得重新引入删除分支、Converter、旧数据浏览器或任何 archive 运行/恢复路径。

采用 C 的理由是极限压缩交付路径：数据团队只建设最终 v4 baseline 和新功能，不投入旧模型理解、转换、对账和兼容工作。代价是现有用户数据不迁移且必须重新配置，这一代价由本决策明确接受。

### D-013：旧数据目录处理与 Clean Cutover

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：只用一次同文件系统 whole-root rename 隔离旧数据，随后在原 canonical path 建立全新空 v4；归档不是迁移输入、运行 generation、恢复点或产品功能。

| 方案 | 旧数据处理 | 主要影响 |
|---|---|---|
| A（已确认） | 首次启动 v4 时，将整个旧 canonical data root 在同一文件系统原子重命名为带时间戳的 sibling archive，随后在原 canonical path 创建全新空 v4 root。应用不提供浏览、恢复、导出或导入入口 | 实现只需一次目录重命名和新建，几乎不增加架构；archive 与 v4 运行路径彻底断开 |
| B | 首次启动 v4 时直接删除旧 data root，随后创建空 v4 | 代码和磁盘最干净、步骤最少，但数据不可恢复；删除失败、文件占用和部分删除需要处理 |
| C | 保留旧版本数据并在新应用中提供只读 Legacy Viewer/Export，允许用户浏览或导出旧会话、Preset、Knowledge 等 | 用户体验最好，但必须保留旧 schema decoder、文件解析、UI、API 和维护路径，实质重新引入被 D-012 删除的兼容工程 |

方案 A 的唯一 cutover 流程：

1. 若 canonical data root 不存在，则视为 fresh install，直接创建 v4 root；不得扫描其他路径猜测旧数据；
2. 若 canonical data root 存在，先停止会持有该目录的 NomiFun 进程、Runtime sidecar 和后台 worker，并校验源路径、同卷 sibling archive 目标与目标不存在；
3. 使用操作系统同文件系统 rename 原语将 **整个 root** 改名为 `<canonical-name>.legacy-v3-YYYYMMDD-HHMMSS`；禁止以 copy/delete、逐文件 move 或跨卷 fallback 模拟成功；
4. rename 成功后，才允许在原 canonical path 创建新的空目录，执行 v4 自己的 baseline migration、七模板和 bundled Package seed，最后写入 v4 ready marker；
5. rename、路径校验、跨卷检查或 archive 目标冲突失败时，旧 root 必须保持原样，canonical path 不得出现半成品 v4，启动以明确错误终止；
6. 若 rename 已成功但 v4 baseline/seed 失败，archive 保持不动；重试只可清理或重建新生成且带 initializing marker 的不完整 v4 root，不自动把 archive 改名回来；
7. v4 Runtime、Kernel、插件、API 和 UI 永远不得枚举、打开、解析或索引 archive；产品不提供 Legacy Viewer、Export、Import、Restore 或 rollback generation；
8. archive 仅是用户可在宿主文件系统手工保留或删除的不透明目录。产品日志可以报告其最终路径，但数据库不记录可恢复关系，也不建立长期设置项。

这项方案把误删保护限制为一次常数成本的文件系统操作，不理解任何旧 schema，也不增加兼容主链。故障注入必须覆盖文件占用、目标冲突、跨卷、rename 失败、baseline 失败与 seed 失败，并证明旧 archive 从未被 v4 打开。

### D-014：Legacy API、表与模式的删除期限

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：按 Vertical Slice/Domain Wave 同改同删，首个 v4 Stable 不携带任何 legacy 产品兼容面。

D-012/D-013 已经决定 v4 不迁移、不读取旧数据。本项确定旧 API、DTO、数据库表、配置字段、执行模式与装配路径从生产代码和构建产物中消失的期限；它不改变 D-004/D-020 对临时 Nomi Runtime baseline/canary 的独立删除门禁。

| 方案 | 删除策略 | 主要影响 |
|---|---|---|
| A（已确认） | **按波次同改同删，首个 v4 Stable 零兼容面。** 每个 Vertical Slice/Domain Wave 在新主链可用且直接消费者切换的同一个变更中，删除对应 legacy route、DTO、表映射、配置字段、Factory wiring、mode/approval 分支和测试；v4 从第一天不发布 alias、旧 endpoint、compatibility view、dual read/write 或 deprecated facade。迁移期唯一例外是 D-004 明确的内部 Nomi baseline/replay/canary adapter，它不暴露旧产品 API、不读取 archive，并由 D-020 单独裁决最终死亡门禁 | 最符合 clean start 与减债目标；实现、调试和验收都只维护一条产品主链。需要每个波次把直接消费者一起迁完，不能用长期 shim 拆账 |
| B | v4 Stable 保留一个发布周期的旧 endpoint alias、只读 DTO/table adapter 和 deprecated config 映射；下一发布再统一删除 | 可降低一次切换对旧 UI/脚本的影响，但需要维护双契约、兼容测试和集中清理项目，且与不迁移旧数据的价值冲突 |
| C | 不设期限，旧 API/表/模式长期保留，由调用方逐步停用 | 单次迁移压力最低，但双权威和 God Service 会永久存在，后续每项功能都继续承担兼容成本 |

已确认采用 A。这里的关键不是追求激进删除，而是 **v4 没有旧数据可服务，因此兼容层没有产品价值**：保留旧 endpoint、DTO 或表只会让前后端继续误用旧语义。按波次同改同删还能把删除工作放在上下文最完整、验证成本最低的时刻，避免 Stable 后再启动一次长期清债工程。

方案 A 的实施规则：

1. 每个新 Slice/Domain Wave 必须列出它替代的 legacy producer、consumer、route、DTO、repository/table mapping、配置字段、Factory/Gateway/AppServices wiring、mode/approval 分支、测试和依赖；
2. 新主链及全部直接消费者在同一个变更中切换，随后立即删除对应 legacy 实现；不得以 deprecated alias、compatibility facade、双路 adapter、feature flag 或“下一版本再删”完成当前波次；
3. v4 schema 从 baseline 开始只包含 canonical 新表；未来 v4 仍使用自己的 append-only migration lineage，但不得创建 legacy compatibility view、shadow table、mapping table 或 dual-read/dual-write trigger；
4. v4 OpenAPI、前端 client、事件词汇和配置 schema 从第一天不发布旧名称或旧字段；内部与外部消费者必须随领域波次同步修改；
5. 每个波次的退出门禁包含 symbol、route、schema、dependency、runtime reachability 和 UI residual exact-zero 检查；仅“代码不可达”不算删除完成；
6. 首个 v4 Stable 构建中的旧产品 API、DTO、表映射、模式、审批、Factory wiring、兼容测试和依赖残留必须为零；旧 published migration 文件仅作为不可修改的源码历史保留，并被 v4 runner 排除；
7. D-004 的内部 Nomi baseline/replay/canary adapter 是唯一例外：它只经 disposable migration coordinator 参与 fresh-v4 internal Session，Nomi 或 Codex 可以是 session-sticky 的唯一真实 primary，secondary 只能只读 shadow 或消费 recorded/simulated 结果；它不暴露 legacy 产品 API、不读取 D-013 archive、不进入正常用户 composition。D-020 已确认它必须在 Nomi-free RC 前硬删除。

## 决策闭合与实施依赖

全部设计决策已经闭合，固定依赖图如下：

```text
D-001～D-028（含 D-019）已确认
        |
        +--> 用户整体审阅
                 |
                 +--> Contract Closure / G0
                              |
                              +--> production implementation
```

- D-025～D-028 已把升级兼容、Remote token、Nomi drain 和正式平台矩阵纳入最终合同；
- D-019 已据此完成五工作流、`213/314 EW`、`29/42 active engineering weeks + HP-1/HP-2 与必要 whole-cohort C8/C10 recheck 实际等待`的规划日历与 Gate/commit 设计；
- 用户整体确认已完成；下一任务进入 Contract Closure/G0，Production schema、公共 Rust contract、Plugin SPI、Runtime protocol 与业务迁移实现只能在相应阶段门禁通过后开始；
- D-015 固定恢复门禁，D-017/D-026 固定 Remote contract，D-018 明确不使用性能 baseline、P50/P95 或统计质量分，D-020/D-027 固定 Nomi 删除前排空语义。

## 已确认决策（续）

### D-016：第三方插件正式支持的 Phase N 范围

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：本次 Stable 只冻结并 dogfood vendor-neutral 插件主链；整体重构完成后的 Phase N1 以本地安装和单 SDK MVP 尽快交付真实第三方闭环，双 SDK、在线更新和市场后置。

这项决定的是：本次核心重构 Stable 只冻结什么扩展缝，以及整体重构完成后，第三方插件支持应以多大的首期范围进入产品。它不改变 D-005：普通第三方插件仍作为 trusted in-process code，不为 Phase N 增加 sandbox、审批、签名链或权限平台。

| 方案 | Phase N 第一期 | 后续阶段 | 主要影响 |
|---|---|---|---|
| A（已确认） | **本地优先、单 SDK MVP。** 用户显式从本地目录或压缩包安装到唯一 managed Package root；安装、启停、替换、卸载在重启后生效；用 `config_schema` 生成配置表单；materialized Capability/Skill/MCP 进入已有目录和 Agent 设定编辑器；只正式支持一种 executable entrypoint/SDK profile，并提供 schema/types、validator、scaffold、reference package 和 conformance runner；只接受 exact host-contract version | 根据真实第三方使用反馈，再增加第二语言 SDK、调试器、依赖获取/更新、state version migration、兼容/弃用政策；这些稳定后才建设 catalog/search/download/publisher/market。Hot reload 最后考虑，也允许永久不做 | 最快形成真实的“挂载→配置→选择→使用”闭环，不让双 SDK、在线分发或市场拖住首期。SDK 使用 Rust 还是 embedded JS/TypeScript 在 Stable `PluginRegistration` 原型完成后用有界 spike 决定，本题不提前锁死 |
| B | **SDK 完整优先。** Phase N1 在本地安装闭环之外，同时发布 Rust + TypeScript 两套 SDK、脚手架、调试工具以及首版 compatibility/state-migration policy；暂不做市场 | 再增加下载、更新、依赖获取与市场 | 开发者体验更完整，但第一期必须同时维护两种 entrypoint、打包链和兼容矩阵，明显延后第一个可用闭环 |
| C | **生态一次性交付。** 本地/URL 安装、双 SDK、依赖获取、自动更新、在线目录、下载、发布者后台、兼容政策和插件市场一起上线 | 后续主要做运营扩展 | 一次覆盖最广，但会让安装器、SDK、分发、市场和兼容政策相互阻塞，并迫使本次 Stable 提前预建大量表、API 和占位分支 |

无论选择哪一项，本次核心重构 Stable 都只完成并冻结以下内部 canonical contract 和验收，不交付用户第三方安装产品：

1. 开放、vendor-neutral 的 `PackageManifest`、exact Package dependency 与四层 contribution envelope；不使用 first-party enum；
2. `PluginRegistration` 的 `validate → create/start → publish` 与 `unpublish → stop/drop` 生命周期，任一步失败都不得发布半套 contribution；配置、启停、替换和移除以重启生效；
3. 同一 `PluginConfigSchema` 驱动默认值、后端校验与未来设置表单；内置 Package 不得使用第二套手写结构；
4. `PluginStateNamespace` 统一冻结为 `(package_id, mount_id, scope_key, state_key)`；第三方不获得任意数据库 migration，本期只预留窄 state-version 入口；
5. Capability/Skill/MCP materialization 携带 source Package metadata，但 Preset、Snapshot、Runtime 和 SessionEvent 不得根据 first-party/third-party 来源走不同语义；
6. CI/test-only `sample.echo` 与至少一个 production built-in 通过同一 mount/config/state/materialize/Editor/Preview/Test/Preset/Snapshot/Runtime/Event/Effect/restart conformance；fixture 在生产 inventory、seed、API 和 UI 中为零；
7. Stable schema、OpenAPI、route、UI、bundle 和依赖图中的用户 loader、public SDK、market、distribution、hot reload、compatibility shim、sandbox、permission/risk/signature 字段必须为零。

已确认采用 A。它最早交付真正有用的第三方插件闭环，同时不会为了一个尚未验证的生态同时维护两套 SDK、在线市场和兼容平台。市场必须排在 installer、SDK、真实插件试用和兼容政策之后，不能倒置。

方案 A 的阶段边界：

1. **本次核心 Stable：**只冻结 `PackageManifest`、`PluginRegistration`、`PluginConfigSchema`、`PluginStateNamespace(package_id,mount_id,scope_key,state_key)`、source metadata、四层 materialization 和同链 conformance；first-party Package 与 CI/test-only `sample.echo` 走同一主链；
2. Stable 中用户 loader、public SDK、任意代码 dynamic discovery、URL/registry install、market、distribution、auto-update、hot reload、compatibility shim 与第三方 DB migration API 的生产 schema/OpenAPI/route/UI/bundle/dependency residual 必须为 0；
3. `apply migrations` 只表示随产品构建发布的 first-party v4 append-only migrations；不形成第三方任意 SQL/migration contract。Phase N1 的第三方 migration surface 仍为零；Phase N2+ 最多增加 namespaced state 的窄 version callback；
4. **Phase N1：**用户显式选择本地目录或压缩包，校验后安装到唯一 managed Package root；配置、启停、替换、卸载均通过重启生效；materialized Capability/Skill/MCP 出现在既有目录和 Agent 设定编辑器；Preview/Test/Save Revision、Snapshot、Runtime、SessionEvent/Effect 全部复用 Stable 主链；
5. Phase N1 只支持一个正式 executable entrypoint/SDK profile，并提供 schema/types、validator、scaffold、reference Package 和 conformance runner；只接受 exact host-contract version，不做范围求解或 compatibility shim；
6. Rust native entrypoint 与 embedded JavaScript/TypeScript 之间不在本决策提前锁定；等最终 `PluginRegistration` 原型可运行后执行一个有界 loader/ABI spike，选择开发、打包、调试和跨平台总成本最低的一种，不能进入当前 Stable 关键路径；
7. **Phase N2+：**根据真实插件反馈增加第二语言 SDK、调试器、依赖获取/更新、state migration 与兼容/弃用政策；这些稳定后才建设 catalog/search/download/publisher/market。Hot reload 最后考虑，也允许永久不做；
8. 安装界面只需明确“安装即信任该代码在 NomiFun 进程内运行”；这不是 permission checklist、审批、Grant 或可续期授权。

## 已确认决策（续）

### D-015：Session Event Store 与历史重放

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：以规范化语义 `SessionEvent + bounded payload` 作为唯一执行与历史事实；所有查询/UI 状态可重建；Codex rollout/checkpoint 只是可丢弃缓存。

本项决定 NomiFun 的会话执行历史究竟以什么为唯一事实，以及 Codex Runtime checkpoint 损坏、丢失或版本不兼容时能够恢复到什么程度。D-003/D-004 已排除“Codex rollout 直接成为产品事实源”，因此真正选择是语义 Event、关系状态表或全量原始 trace 三种权威模型。

| 方案 | 唯一事实与恢复保证 | 主要影响 |
|---|---|---|
| A（已确认） | **规范化语义 `SessionEvent + bounded payload` 是执行与历史事实；Projection 可重建；Codex rollout/checkpoint 只是可丢弃缓存。** 持久化用户/助手消息、turn 状态、实际模型可见的变化型 Context、Tool call/result、Effect receipt、capability activation、compaction、fork provenance 和 Runtime binding digest。UI Message/Tool card/Session head 是同事务更新的 Projection。Checkpoint 有效则快速高保真 resume；无效则丢弃，产品历史/Projection 由 Event 重建；只有 D-025 兼容性 admission 接受 exact Snapshot 时才由最新 completed compaction 与后续 Event 创建新执行 binding。保证语义终态和产品历史精确重建，不承诺逐 token/SSE 字节级重现 | 比只存当前状态多一条统一 event/payload/projection 主链，但能删除 legacy chat 历史库、Nomi session JSON 和 Runtime rollout 多事实源；数据量与实现复杂度适中，足以支撑 crash、resume、fork、canary 和 Nomi 删除门禁 |
| B | **关系状态表是事实，Event 只作 outbox/audit。** `sessions/turns/messages/tool_invocations/effects` 等可变表保存当前终态；checkpoint 优先，丢失后从当前消息和 Tool 终态做 best-effort 文本恢复，不保证历史状态转换可重建 | 第一屏 CRUD 最快、数据最少，但每张表都需要自己的 crash/reconciliation 逻辑；activation、compaction、uncertain Effect 和 Nomi/Codex 对照更难复现，容易重新形成多事实源 |
| C | **全保真原始 Event Source。** 持久化每个 Codex item、provider stream item、token/chunk、完整 Tool I/O、原始 checkpoint/blob 与精确时序，Projection 全部重建 | Inspector 和 trace 最完整，但写放大、数据量、schema/version/retention 工程最大；即使保存全部原始事件，也不能安全地重新执行外部 Effect，不符合交付速度优先 |

方案 A 的最小持久化结构只有三张事实表和两张投影表：

```text
agent_sessions       -- identity, owner, exact preset/snapshot, parent/fork base, next_seq
session_events       -- session_id + seq + kind/version/correlation + inline_json|payload_id
session_payloads     -- bounded body/blob, media_type, byte_len, digest
session_heads        -- rebuildable projection: status, active turn/generation, runtime binding
message_projection   -- rebuildable projection: UI message/tool/effect cards
```

共同失败语义：

1. `append event + payload + 更新 projection + last_seq` 在同一个 SQLite transaction；core/session outbox 为 0，commit 后基础 EventBus 只发送 best-effort wakeup，客户端按 cursor 补读；可靠领域事实由 owning plugin 在自己的事务中写 typed domain event + outbox；
2. Runtime 使用稳定 `event_id/correlation_id` 幂等追加；重复事件返回原 cursor，不重复投影或执行；
3. state-changing Tool 先记录 `effect/started` 再 dispatch；结果未知时记录 `effect/uncertain` 并明确失败，绝不自动重试或在 replay 中重新执行，由 owning plugin 使用同一 idempotency key reconcile；
4. checkpoint/rollout 缺失、损坏，或 `runtime_bound_event_ref` 指向的 Runtime build identity/protocol/Snapshot/`through_seq` 任一不匹配时直接丢弃，不开发 converter；canonical SessionEvent 始终可重建产品事实/Projection，只有 D-025 兼容性 admission 接受 exact Snapshot 时才创建新执行 binding；
5. compaction 只有 `completed` 才生效，只改变 Runtime context projection，不删除产品历史；fork 持有自包含 child base payload，不依赖父 Session 永久存在；
6. 逐 token delta、raw SSE/provider wire、typing/heartbeat、重复 progress、中间 reasoning、未进入模型的完整 stdout/stderr 和已被替代的 checkpoint 可以丢弃；已展示文本使用有界 chunk 聚合持久化；
7. Replay/debug/shadow 使用记录的 Tool result/Effect receipt 或 disposable fixture，永不重新改变外部世界。

已确认采用 A。它以一套语义事件同时服务产品历史、恢复、投影、调试和 Runtime 切换，又避免 C 的原始 token/trace 数据平台；相比 B，它更早消除当前产品会话、Nomi session 与 Runtime checkpoint 三套事实互相漂移的根因。

方案 A 的最终边界：

1. `agent_sessions/session_events/session_payloads` 是事实表；`session_heads/message_projection` 是可删除并由 Event 全量重建的投影；
2. 实际业务状态与 Effect idempotency/reconciliation 仍归 owning plugin；SessionEvent 只保存调用事实、bounded model-visible result、receipt/reference/digest，不复制业务表；
3. 大文件、diff、终端日志和媒体实体归 Artifact/资源插件；Event 只保存稳定引用、digest 与模型实际看到的有界内容；
4. Event append、payload、Projection 更新与 `last_seq` 在同一 SQLite transaction；core/session outbox 为 0。基础 EventBus 只在 commit 后发送 best-effort wake-up，客户端按 cursor 补读；可靠业务动作使用 typed command，可靠领域事实由 owning domain 写自己的 outbox；
5. `effect/uncertain` 使当前 turn 明确失败且绝不自动重试；只有 owning plugin 可以按同一 idempotency key reconcile，重放永不重新执行外部 Effect；
6. Codex checkpoint/rollout 仅保存在 Runtime 专用 root；NomiFun 只保存 locator、digest、`runtime_bound_event_ref`、protocol、Snapshot digest 与 `through_seq` binding，实际 Runtime build identity 只存在于被引用的 canonical `runtime/bound` Event。任一不匹配即丢弃，不开发 converter；产品事实/Projection 重建与是否创建新执行 binding 分离，后者服从 D-025 compatibility admission；
7. Compaction 只改变 Runtime context projection，不删除 canonical 产品历史；fork 生成自包含 child base payload，不依赖父 Session 或父 checkpoint 永久存在；
8. 不建设逐 token/raw SSE event sourcing、全局内容寻址仓库、独立 Runtime event DB、Effect Coordinator、checkpoint converter、加密 CAS 或 legal-retention 平台；
9. D-020 的 Nomi 删除门禁必须证明：删除 Nomi private session、全部 Codex checkpoint 或任意兼容 checkpoint 后，仍可由 canonical SessionEvent 恢复产品语义与 Projection；新 Runtime binding 只有在 D-025 兼容性 admission 接受 exact Snapshot 时创建，不能由本项提前决定旧 Snapshot 可执行性。byte-exact provider replay 不作为门禁。

## 已确认决策（续）

### D-018：轻量 Preset 与 Coding 完整性边界

- 状态：`已确认`
- 用户裁决：选择方案 A，但删除本次重构阶段的量化性能测量、baseline、benchmark、统计质量评测和性能 RC 工作。
- 原则：`chat.minimal` 等轻量 Preset 通过正向最小装配在结构上保持轻量；`coding.codex` / `coding.codex-native` 通过完整能力清单和正常功能验收确保不退化。理论收益不需要在本次以 P50/P95 或 paired benchmark 再证明。

轻量 Preset 的架构硬约束：

1. `chat.minimal` 的 `initial_capabilities=[]`、`on_demand_capabilities=[]`、active set、Tool、Tool Search/compact index、Skill catalog、MCP、workspace、AGENTS、Git、Shell/Patch、Memory/Knowledge 和业务 Context 全部为空或不初始化；
2. 最终 Provider request 必须 `tools=[]`，不能为了统一 Runtime 偷放搜索控制工具、占位 Tool schema 或 deferred stub；
3. AgentPreset Compiler 只正向解析并构造 Snapshot 明确选择的内容；禁止“全量扫描/连接/构造后再过滤”，禁止为未选择能力启动 Provider、MCP、Browser、Computer、SSH、Office、worker、watcher 或资源连接；
4. 非 Coding Runtime Profile 彻底替换 Codex Coding instructions，并关闭 repo/worktree/AGENTS/Git/Shell/Patch/Skills/Plugins/MCP warmup、Code Mode、Review 和子 Agent；
5. 轻量性的验收仅使用确定性的结构/调用图/最终请求检查，属于普通正确性测试，不统计 tokens、bytes、TTFT、端到端时延、冷/热启动时间、P50/P95、请求分布或资源占用。

Coding 不得退化的架构硬约束：

1. `coding.codex-native` 的 canonical Capability/Runtime feature/原生 Responses 语义清单必须完整，优先复用 Codex 原生实现而不是降级为通用 MCP；
2. 必须保留 workspace/repository、AGENTS 规则、Git、File read/search/write/edit/apply_patch、Shell/PTY/stdin/process、Skills、Plugins、MCP、Tool Search、Code Mode、计划/目标、子 Agent、多 Agent、Review、验证、steer/cancel/resume/fork/rollback/compaction、错误恢复和跨平台进程清理；
3. OpenAI/Codex 原生 Responses 通道不得为了统一 Provider 而丢失 reasoning、tool-call、prompt-cache、stream item 或 Coding 模型特性；
4. 用能力 exact-set、协议/conformance、现有上游测试、正常构建/测试任务和少量代表性 E2E 做功能验收；这些是实现正确性门禁，不建设大规模 Coding corpus、paired runs、统计显著性或 non-inferiority 评测；
5. 不允许用“轻量化”为理由删除 Coding 能力、把必需 initial 能力机械移入 on-demand、缩短 Coding instructions，或把 Codex 原生能力全部包成能力更弱的通用适配层。

本次明确删除的测量工作：

- 删除 Nomi/Codex matched baseline 和 `chat-minimal.v1` benchmark corpus；
- 删除 tokens/bytes cap、Provider request distribution、TTFT/E2E、cold/warm bind、sidecar reuse P50/P95 SLO；
- 删除 reference device runner、200-turn/provider cell、30 cold starts、100 warm binds、paired Coding corpus 和 `-2pp` non-inferiority 统计；
- 删除仅为测量新增的 telemetry 字段、性能 JSON artifact、Prometheus/Grafana 或独立性能平台；
- 删除以性能为目的的 7/14 天 Nomi-free RC observation window，以及“两发布周期”性能观察；
- 删除 D-019 ROM 中对应的 benchmark、统计评估、reference runner 和性能优化 reserve。若未来实际使用出现性能问题，再以独立需求测量和优化。

D-020 的 Nomi 删除门禁不再依赖性能 baseline、P50/P95 或统计质量分，只依赖最终功能/结构、全场景接入、Coding 完整性、SessionEvent 恢复、Effect 正确性、崩溃/取消/进程清理和 legacy residual 为零。

## 已确认决策（续）

### D-017：Remote 调用与 Agent 设定映射

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：服务端 `RemoteBinding.agent_binding` 复用 canonical AgentBindingValue，冻结 exact AgentPreset revision、Snapshot 与 typed resources；Remote 客户端只通过显式 `open/turn/observe/cancel` 使用产品 Session。

Remote 永久只是 ingress/transport plugin，不是 Agent 类型、官方模板、RuntimeProfile 或权限模式。本项决定 Remote 如何选择 exact AgentPreset revision、绑定资源并创建/复用产品 Session。

| 方案 | Preset/资源选择位置 | Session 语义 | 主要影响 |
|---|---|---|---|
| A（已确认） | 用户在本地管理面创建服务端 `RemoteBinding`，其 `agent_binding` 直接复用唯一 canonical `AgentBindingValue`；远程请求只提交 `remote_binding_id` | `open` 先持久化 canonical AgentSession 为 `opening`，sidecar admission 后再推进 `ready|failed`；响应返回唯一 `agent_session_id`，后续 `turn/observe/cancel` 显式复用该 ID | MCP/REST 共用同一模型，认证、Preset、资源职责清晰；客户端不理解内部 Preset schema，也没有隐式 latest/default 漂移或跨 SQLite/sidecar 假原子事务 |
| B | 每次 `open` 请求直接携带 exact Preset ref 与完整 typed resource bindings；创建后仍用 Session ID，后续禁止再次传 Preset/资源 | 显式创建、显式复用 | 后端少一个 Binding 对象，但每个 Remote 客户端都要理解内部 revision/digest/resource schema，MCP 配置复杂且多客户端容易漂移 |
| C | Remote 不能创建 Session，只能向本地 UI/其他业务预先创建的 `agent_session_id` 发 turn | 仅显式复用预建 AgentSession | 实现最少，但失去 headless 自动化和远程创建任务的核心价值，Preset 映射依赖带外流程 |

已确认采用 A；最小 `RemoteBinding` 为：

```text
remote_binding_id
owner_user_id
name
agent_binding: AgentBindingValue {
  preset_revision_ref
  resolved_snapshot_ref
  typed_resource_bindings[]
  binding_version
}
```

固定语义：

1. `RemoteBinding` 是 Remote transport 配置，不是授权记录；它只增加 Remote id/owner/name，并在 `agent_binding` 中复用 canonical AgentBindingValue，不能定义第二套 Preset/Snapshot/resource schema，也不保存 token hash、capability scope、模型 override、运行 mode、grant、expiry、approval 或 caller role；
2. 唯一 installation token 只认证 installation owner；`binding_id` 不是秘密，也不能扩大 principal 权限；不恢复 per-companion/per-preset token 或 token scope DSL；
3. 协议层只有 `open / turn / observe / cancel` 四个语义操作，REST/MCP 只是适配器；
4. `open` 先认证、读取 exact Binding，并完成 ownership/resource preflight 与 Compiler；第一笔 SQLite transaction 持久化 immutable Snapshot、幂等键和 `status=opening` 的 Session，跨 sidecar Runtime admission 完成后再由第二笔 transaction 推进为 `ready`，失败则推进为可诊断的 `failed`。不伪造跨 SQLite/sidecar 原子事务，非 `ready` Session 不执行；
5. Remote 没有 IM 的自然 chat key，禁止按 token、IP、MCP connection、最近 Session 或客户端名称隐式复用；客户端必须保存并显式提交 `open` 返回的唯一 `agent_session_id`；
6. MCP transport session id 只做连接相关性，不能成为产品 Session 主键；网络断开不改变 AgentSession 事实；
7. Binding 更新只影响之后创建的新 Session；既有 Session 永远使用创建时冻结的 Snapshot。删除 Binding 只阻止新建，停止既有 Session 必须显式 `cancel`；
8. opening transaction 从 canonical AgentBinding 冻结 exact Preset/Snapshot/model route/config revision、initial/on-demand、Package/MCP/schema digest、RuntimeProfile、所需 Runtime protocol/features/release constraint 与 typed resources；实际 Codex build 只在 RuntimeReadyAck 后由第二 transaction 的 `runtime/bound` Event 记录。后续 turn 不接受 Preset、model、capabilities、profile、domains 或资源覆盖；
9. 若保留直接 MCP/REST Capability projection，每次调用也必须绑定产品 Session 并经过其 Snapshot dispatch；删除“installation token → 全局 Registry”直通旁路；
10. 唯一行为是 FullAuto；删除 Remote confirmation、`needs_confirmation`、danger approval 和等待状态。`409` 只表达幂等、busy 或版本冲突；
11. REST、MCP 和 SessionEvent 使用同一 canonical error code：认证失败、Binding/Session 不存在、幂等/busy/digest 冲突、resource missing/owner mismatch，以及已有 Capability/Provider 错误；旧 `profile/domains/confirm/remote_agent_id` 字段直接 schema failure；
12. Remote 本身不形成 AgentPreset、Capability Pack 或专属 Agent 编辑器；它只在“Remote/连接”管理页绑定任意 exact AgentPreset revision。

已确认采用 A。它只增加一个简单的服务端 Binding 配置表，却让 REST/MCP 客户端保持极简，并彻底消除 query `profile/domains`、Remote Agent、per-token scope 和“最近会话”隐式状态。

方案 A 的最终产品与数据边界：

1. `RemoteBinding` 是普通 owner-owned transport 配置事实；只保存 `remote_binding_id/owner_user_id/name/agent_binding:AgentBindingValue`，Preset/Snapshot/resources/version 复用唯一 canonical value schema；它不保存认证 token、scope、model override、mode、Grant、expiry、approval 或 caller role；
2. 安装级 token 只认证 owner，Binding 只选择运行配置；认证、Preset 和资源绑定是三件独立事情；
3. `open` 使用可恢复两事务状态机：认证、Binding/resource preflight 与 Preset 编译后，先提交 immutable Snapshot、幂等键和 `opening` Session；Runtime admission 后再提交 `ready` 或 `failed`。客户端不得把 `opening/failed` 当作 ready，系统也不声称 SQLite 与 sidecar 原子提交；
4. `turn/observe/cancel` 只接受 canonical `agent_session_id` 和必要 cursor/idempotency key；REST/MCP 不重新提交或覆盖 Preset/model/capability/resource；
5. Binding 更新或新 Preset revision 只影响之后创建的 Session；删除 Binding 只阻止新建，不隐式取消已存在 Session；
6. MCP transport session id、HTTP connection、token、IP、客户端名和最近 Session 都不能成为产品 Session 主键或隐式复用键；
7. 直接 Remote Capability projection 若保留，也必须绑定 AgentSession 并通过其 frozen Snapshot/active generation dispatch；全局 Capability Registry 直通为零；
8. Remote 全程 FullAuto，不存在 confirmation/approval/danger wait；所有错误使用 REST/MCP/SessionEvent 同一 canonical code；
9. 旧 `/mcp-agent` 特例、`profile/domains` query、per-companion/per-preset token、`remote_agent_id`、RemoteAgent、`needs_confirmation` 和 Remote danger-confirm 路径必须在 D-014 波次内物理删除；
10. D-020 的全场景门禁必须覆盖 REST/MCP × open/reuse、Binding 更新后旧 Session 不漂移、token rotate/revoke、resource owner/provider failure、FullAuto effect、断线后的 cursor/idempotency 恢复。

## 已确认决策（续）

### D-020：Codex 最终切换与 Nomi 硬删除门禁

- 状态：`已确认`
- 用户裁决：采用方案 A。
- 目标：internal functional canary 只存在于迁移期；全场景 Codex-only 后先物理删除 Nomi，再生成 Nomi-free RC，Stable 提升同一 digest；产品从不携带双 Runtime fallback。

本项决定 Nomi Runtime 何时从代码和产品制品中彻底消失，以及删除后发生问题时允许怎样回退。D-018 已删除性能与统计评测，因此这里仅使用结构、功能、数据、Effect、故障和全场景正确性证据，不设置固定天数、发布周期、turn 样本量或性能窗口。

| 方案 | Canary 与删除时点 | 删除后的回退 | 主要影响 |
|---|---|---|---|
| A（已确认） | 内部 Beta 按 `Scene + exact Preset revision digest + Domain Wave/cohort` 做 session-sticky 功能 canary；每个领域按 D-027 stop admission，idle Session 立即清理删除，accepted operation 到自身与全部祖先既有 finite deadlines 的最小值后执行 `cancel→dispose→kill→uncertain handoff→zero→D-024 delete`，再同步删除该域 Nomi wiring。C8 全场景 Codex-only/global drain 通过后，C9 硬删除剩余 Nomi，再生成 C10 Nomi-free RC；C11 直接提升同一 digest | 只允许回退兼容的同-v4 Host 或 pinned Codex sidecar 制品、回退 Preset/model route，或 forward fix；checkpoint 不兼容时按 D-015/D-025 重建事实并执行 compatibility admission。产品内不保留 Nomi fallback | 覆盖全部场景并确保 Nomi 不进入 Stable；canary/RC 只复用正常功能/故障验收，不建设测量平台或长期双 Runtime |
| B | 首个 Stable 仍携带 dormant Nomi fallback，运行异常时可由产品内路由切回；承诺下一发布再删除 | 产品内 Runtime fallback/双 Runtime | 表面稳妥，但会永久保留双 Runtime、双恢复语义、双依赖和双测试矩阵；“下一发布”极易失约 |
| C | `chat.minimal`、`coding.codex`、`sample.echo` 三联 Gate 通过后立即删除全部 Nomi，不等待其他 Domain Wave 和全场景功能 canary | 只能停止发布并 forward fix | 删除最快，但 Robot、IM、Customer Service、Creative、AutoWork、Remote、Provider Bridge 和跨平台进程路径尚未完成最终验证，容易用用户故障换取表面速度 |

方案 A 的精确执行顺序：

1. Nomi 进入冻结状态，不再接受新产品能力、数据模型或长期抽象；它只经 disposable migration coordinator 参与 fresh-v4 internal Session，作为 session-sticky single primary 时可以真实执行，作为 secondary 时只能消费 recorded/simulated result；
2. `chat.minimal`、`coding.codex`、`sample.echo` 三联最终主链 Gate 通过；
3. 各 Domain Slice/Wave 在内部 Beta 做 session-sticky canary。只读场景可以 shadow；有副作用的 Turn 只能有一个 primary 真执行，另一侧只能消费 recorded/simulated result，禁止双写或双 Effect；
4. 一个领域转到 Codex 后，按 D-027 停止新 admission；idle Session 立即 `cancel→dispose→kill descendants→zero→D-024 delete`，fence 前 accepted operation 只到自身与全部祖先既有 finite deadlines 的最小值，随后执行 `cancel→dispose→kill descendants→uncertain handoff→zero→D-024 delete`；handoff 不等待 reconcile。然后在同一个变更中删除该领域的 Nomi route/wiring/Factory field/test/dependency；
5. 七模板、Research Pack、Requirement/AutoWork/Cron、Companion、Robot、Customer Service、Creative/MiniApp、IDMM、IM/Channel、Remote、Browser/Computer、Provider Bridge、create/resume/fork/steer/cancel/compaction/crash/upgrade 与五项同步检查全部通过最终 Codex-only 结构、代表性 E2E 和 fault gate；
6. 全场景 Gate 与 D-027 最终定义的全局 drain gate 均通过，且 Nomi admission/new Session/active Session/model request/tool/Effect execution/file-session write/process/resource/fallback/reachability 全部为零后，才物理删除剩余 Nomi loop、Manager、Factory、Bootstrap、private session/index、adapter/shim、Cargo feature/package/dependency 和专属测试；
7. 从删除提交生成 Nomi-free RC，完整重跑普通 build/test、协议 conformance、代表性全场景 E2E、Projection rebuild、no-checkpoint rehydrate、Effect uncertain/reconcile、cancel/crash/process cleanup 与 legacy residual-zero；不运行性能 benchmark 或统计样本窗口；
8. Stable 直接提升已经通过的同一 RC digest，不重新构建另一份制品。

删除后的唯一 rollback 语义：

- 删除前 internal canary：只停止把**新 Session**分配给问题 cohort；已经运行的 Session 不迁移 Runtime，不在 Turn 中途或 Effect 后切换；
- 删除后 RC/Stable：可以停止 rollout、回退 exact Preset revision/model route，或发布上一兼容的 v4 Host/pinned Codex sidecar；
- 如果没有兼容的 v4 Codex 制品，则 halt rollout + forward fix；
- 禁止恢复 Nomi Engine selector、per-turn fallback、pre-v4/Nomi binary、old-binary rollback bundle、D-013 archive 读取或数据 downgrade。

已确认采用 A。它把 canary 限制在内部迁移阶段，把 Nomi 的死亡点放在 RC 之前；既不把双 Runtime 债务带进 Stable，也不会像 C 一样在大多数业务场景尚未接入时过早删除底层执行器。由于只复用正常功能和故障验收，它不会重新引入 D-018 已删除的性能测量工作。

## 已确认决策（续）

### D-019：最终实施工作流与 ROM

已确认采用 **A：五条稳定 owner workstream**。默认由 **6–8 个并行 coding agents** 执行闭合 slice；每个 Agent 有 disjoint write manifest，中央文件只有一个 integration owner。W4 在三联 Gate 后可临时拆 Domain pod，W5 是唯一共享 Gate/Release owner。P50/P80 是 gross engineer-weeks 的规划不确定性，不是 D-018 已删除的 Runtime 性能测量，也不是固定交付承诺。

#### 最终 ROM 校正

| 校正项 | P50 | P80 | 归属 |
|---|---:|---:|---|
| 原五流去重基线 | 202 EW | 294 EW | D-001～D-024 |
| D-025 Snapshot compatibility admission/fork closure | +5 | +8 | W1/W2/W3/W5 |
| D-026 request-admission token fence | +0 | +0 | 复用 Remote ingress transaction boundary |
| D-027 Nomi drain finite-deadline/cancel/kill closure | +2 | +4 | W2/W5 |
| D-028 required native platform cells | +4 | +8 | W2/W5 |
| **方案 A 总计** | **213 EW** | **314 EW** |  |

方案 A 的规划日历为 **29/42 个 active engineering weeks（P50/P80）**。它以 6–8 个 coding agents、五个 accountable owners、中央文件串行合流、低频共享 Gate，以及 D-028 所需真机在 handoff 后可及时使用为前提。HP-1/HP-2 是两次计划内整个平台阶段交接；后续只有当一整批平台验证全部返回、共享修复已经统一合入并冻结新候选后，C8-MERGE 或 C10-MERGE 收敛过程才能发起一次 whole-cohort recheck 通知/换机批次：影响集命中的 cell 跑完整受影响 Gate，未命中 cell 也必须在原生 Host 跑新 tuple scoped attestation，五格同批并行。任何等待都不计入 engineer-week 或 `29/42`，实际 wall-clock 按真实 HP/recheck 等待增加。禁止按单个改动、失败、功能或模块换平台，也不能让 Windows 代替其他原生平台验证来压缩日历。

被拒绝的组织方案只保留审计估算：**B** 为八支长期团队，`229/346 EW`、`31/46` 周，重复计价和合流成本最高；**C** 为三条巨流，`206/330 EW`、`36/52` 周，名义人员较少但关键路径和 bus factor 最差。A 在交付速度、责任闭合与返工风险之间最优。

#### 五条 Workstream

| Workstream | 唯一完整所有权 | P50 | P80 |
|---|---|---:|---:|
| **W1 Platform Foundation & Fresh-v4** | 四层 contract、D-015 Event/Projection、Thin Kernel、Compiler/two-set、PluginRegistration/`sample.echo`、fresh-v4/cutover、D-025 compatibility contract | 42 EW | 62 EW |
| **W2 Codex Runtime & Providers** | pinned Codex fork/sidecar、Runtime Protocol/Client、Broker/Responses Bridge、Provider、完整 Coding、D-025 executor admission、D-027 drain/process cleanup、D-028 sidecar cells | 46 EW | 68 EW |
| **W3 Product Control Plane** | 单页 Editor、七模板、Agent/Remote Binding、Preview/Test/SaveRevision/Inspector、D-025 continuation UX、D-026 singleton token UX、D-028 availability presentation | 19 EW | 26 EW |
| **W4 Domain Migration & Inline Demolition** | 五个业务 Wave、Remote REST/MCP、全部 direct consumers、functional canary、每个 Slice 同改同删 Nomi/legacy wiring/API/DTO/table/config/test/dependency | 74 EW | 108 EW |
| **W5 Shared Integration, Hard Delete & Release** | 唯一共享 Gate、residual/reachability、三联合流、recovery/fault、D-027 global drain、D-028 platform/release matrix、Nomi hard delete、RC/Stable | 32 EW | 50 EW |
| **总计** |  | **213 EW** | **314 EW** |

#### 阶段 Commit、Gate 与唯一关键路径

```text
C0 Contract Closure / Gate scaffold
  -> [Windows continuous implementation: no feature/module pause]
  -> C1 FullAuto physical deletion
       +-> C2 Fresh-v4 ownership ---------+
       +-> C3 Kernel / Plugin contract ----+ parallel
       +-> C4 Runtime / Model -------------+
       +-> C5 Preset Product --------------+
                                            |
                                            +-> C6 Chat + Coding + sample.echo triple Gate
                                                 -> C7 Domain slices in parallel, each same-change demolition
                                                 -> C8-WIN-PRE Windows pre candidate + all-feature/full Gate
                                                 -> HP-1 PAUSE: notify and hand off to real macOS arm64
                                                 -> C8-MA macOS arm64 native implementation/full Gate
                                                 -> HP-2 PAUSE: notify and dispatch three frozen-candidate native tasks
                                                 -> C8-MX || C8-LD || C8-LH
                                                 -> merge whole-batch fixes
                                                 -> (C8-RECHECK-n whole-cohort native batch)*
                                                 -> C8-MERGE five-cell final-cohort evidence + global zero-outstanding Gate
                                                 -> C9 Remaining Nomi physical hard delete
                                                 -> C10-WIN || C10-MA || C10-MX || C10-LD || C10-LH
                                                 -> merge whole-batch RC fixes
                                                 -> (C10-RECHECK-n whole-cohort native RC batch)*
                                                 -> C10-MERGE same-tuple signed Nomi-free RC evidence
                                                 -> C11 Promote exact same digest to Stable
```

C0～C8 每一阶段都形成只包含闭合 slice/wave、并可按该边界规则整体 ordinary revert 的 staged commit，同时检查 staged diff、直接消费者切换、同改同删与 Gate evidence。**C8-MERGE 是进入 Nomi hard delete 前最后一个可逆全量门禁**。C9～C11 仍分别形成可定位、可审计的 staged commit，但不再称为可回退：C9 之后禁止通过 Git revert 恢复 Nomi，只能 halt rollout、使用兼容同-v4 制品、回退 exact Preset/model route 或 forward fix。

slice 内只运行直接相关的 targeted compile/test/schema/route/UI checks。workspace 级 `cargo test` 只属于 **C6、C8-WIN-PRE、C10-WIN** 三个 Gate 节点族，并由 validation coordinator 按 exact input tuple 去重；同一 tuple 只执行一次，整批 shared/forward fix 生成新 tuple 且使 Windows broad evidence stale 时，先合并修复，再在原节点族为最终 tuple 重跑。C8-MA/MX/LD/LH 与 C10 非 Windows cells 只跑 target-specific checks。禁止 6–8 个 Agent 重复触发测试风暴。

Gate 由 repo-local script/orchestrator 组合普通 unit/integration/E2E/fault、UI/build 与 release checks，不建设常驻服务、Dashboard、独立数据库或新的 GitHub Actions 假设。必须覆盖 exact-empty Chat、完整 Coding、`sample.echo` 同链、Event/Projection/Effect/recovery、Remote token fence、D-025 compatibility、D-027 drain/zero handles、D-028 required platform cells、D-014 residual 和 Nomi-free RC。计划内平台交接只有 `HP-1/HP-2`：Windows pre candidate full Gate 与 macOS arm64 整体 native Gate。后续必要换机只能由 C8-MERGE/C10-MERGE 收敛过程在一个完整验证批次结束、全部 fixes 合并且新 cohort tuple 冻结后，以 whole-cohort recheck 整批启动五格原生任务；affected cells 跑完整受影响 Gate，unaffected cells 跑 scoped attestation。功能、模块、verification point、单个失败或单个修复绝不触发换机。HP/recheck 只是实施任务中的 pause/notification/checklist boundary，不是产品状态、审批、automation、数据库字段、Event 或 Runtime gate。

Engineer-week 已包含普通实现、unit/integration/E2E/fault、评审修复和必要文档；不重复计价 Composition scanner、fixture、Provider/Coding/Remote tests、schema、文档、canary 或 release。D-005 安全平台、D-012 converter/import、D-016 Phase N、D-018 性能测量、raw trace/Effect Coordinator/checkpoint converter/retention、双 Runtime fallback 与 pre-v4/archive rollback 均为 `0 EW`。

每个 ROM item 必须同时交付 canonical producer、全部 direct consumers、同改同删、验证证据与 residual/reachability closure。各 Gate 用 actual 更新 ETC：`EAC = actual EW + open slices latest ETC`；不使用模糊完成百分比或统一 unknown reserve。

## 已确认决策（续）

### D-021：旧 Conversation 概念与 AgentSession 身份关系

- 状态：`已确认`。
- 用户裁决：采用**改良后的方案 A**。
- 结论：新架构只有一个 canonical aggregate、一个 UUIDv7 主键：`AgentSession/AgentSessionId`。

命名和产品契约固定为：

- 中文 UI 使用“会话”；
- 英文聊天类用户界面使用 **Chat**，执行、诊断和开发者界面使用 **Session**；
- 内部 Rust/TypeScript 类型、service、repository、Event、API 和 fresh-v4 schema 不再使用 `Conversation` 技术术语；
- canonical API 根为 `/api/agent-sessions`，数据库事实表为 `agent_sessions`；
- Remote `open` 返回唯一 `agent_session_id`，后续 `turn/observe/cancel` 显式复用该 ID，不再定义第二个产品 ID 或映射 handle；
- fork 创建新的 `AgentSessionId`，并以 `parent_session_id/fork_base_payload_id` 表达来源；
- 标题、归档、置顶、未读、SessionEvent、Projection、Runtime binding、Remote provenance 和删除生命周期全部归同一个 AgentSession；
- 删除 `ConversationId`、`conversations` 表、Conversation service/repository、映射表、双 ID API 以及两套创建、fork、归档和删除生命周期；
- 旧术语只能出现在当前状态证据、legacy 删除清单和本决策的历史说明中，不能进入任何 v4 产品英文文案或新 contract。

若未来出现多人或多 Agent 共用一个产品容器的真实需求，应另立需求设计新的 Group/Thread aggregate；当前不预埋 1:N 容器。

### D-022：Agent 设定 Test Revision 与真实 Effect

- 状态：`已确认`。
- 用户裁决：采用方案 A。
- 结论：Test 是普通保存与普通执行主链的 UI 编排，不是测试 Runtime、模拟器或第二种 Session。

固定行为如下：

1. dirty draft 点击 Test 时，先自动保存一个普通、可见、immutable `AgentPresetRevision`；clean draft 直接复用当前 Revision，不产生重复 Revision；
2. 保存或复用成功后，通过普通 `POST /api/agent-sessions` 创建一个持久、UUIDv7 `AgentSession`，不使用 test-only ID、表或 API；
3. Test 使用该 Revision 的真实 typed resource bindings，经普通 Compiler、Snapshot、Codex Runtime 与唯一 FullAuto 主链执行真实 Tool 和真实 Effect；
4. SessionEvent、EffectReceipt、Runtime binding、错误、取消、恢复和其他 lifecycle 与普通 AgentSession 完全相同；
5. 不建设 hidden test revision、test-only Session、disposable test resources、`DraftSnapshot`、ephemeral execution path、测试专用清理器或审批/确认弹窗；
6. UI 在 Test 操作旁静态、清晰地提示“将自动保存，并可能对当前真实资源产生 FullAuto 副作用”，但提示不创建等待、审批或二次确认状态；
7. Test AgentSession 的删除、tombstone、payload 与 EffectReceipt 保留全部服从 D-024，不单设测试历史例外。

选择 A 的理由是它只保留一条可追溯的 Revision/AgentSession/Effect 主链，测试结果与正式执行天然同构，也最符合交付速度优先、逻辑简单和 Coding/业务能力真实一致的要求。

### D-023：七个官方模板的 initial/on-demand Seed 政策

- 状态：`已确认`。
- 用户裁决：采用改良 A；本轮确认 Seed 政策，不逐项确认 Capability ID。
- 核心原则：**role-complete but context-minimal（角色能力完整，但每轮上下文最小）**。

固定政策如下：

1. `chat.minimal` 始终 exact-empty：initial/on-demand、Tool、Search/index、Skill、MCP、Workspace 与业务 Context 全部为零，最终 Provider request 为 `tools=[]`；
2. `coding.codex` 的 `coding.codex-native` capability/feature union、Coding instructions、原生 Responses 语义与代表性 Coding 工作流必须完整，不得以轻量化为理由降级；
3. 其余官方模板必须默认预置让该角色开箱成立的常用能力，而不是只留 Persona 空壳。角色所依赖的能力可以跨业务插件；例如 `companion.default` 默认覆盖 Persona、伙伴 Memory、Knowledge、IM/Channel 连接以及学习/演进等常用能力；
4. `initial_capabilities` 只放首轮或几乎每轮都必须直接投影的身份、核心 Context 与控制能力；`on_demand_capabilities` 不是“未安装”或“用户尚未选择”，而是已经写入 Preset、完成兼容性/依赖/resource binding 校验并冻结进 Snapshot，只在 Agent 需要时通过短索引 lazy 投影完整 schema/instructions，并按需启动对应 Provider；
5. 因而“默认预置”不等于“每轮全部注入”：伙伴可以默认拥有 Knowledge、Memory、IM 等完整角色能力，其中每轮必需部分进入 initial，低频或重型部分进入 on-demand；
6. 用户 fork 官方模板后，可以从当前 Host **已经安装并 materialize 的 Capability Catalog（能力目录/能力集市视图）**中，把任意兼容能力加入 initial 或 on-demand；Compiler 统一校验依赖、冲突、Host availability 和 typed resource binding；
7. Agent 运行时只能 search/activate Snapshot 已冻结的 on-demand ceiling，不能自行安装 Package、从 Catalog 扩大 ceiling 或修改 Preset；未来 Plugin Market 安装第三方 Package 后，其 materialized Capability 才会新增到 Catalog，第三方市场本身仍按 D-016 放在后续 Phase；
8. `Research` 继续作为可加入任意 Preset 的 Capability Pack，不恢复为官方 Agent 模板；Requirement、AutoWork、Cron、IM、Remote 仍是选择或运行 exact Preset 的 Host/Plugin。

七个模板的精确 Capability ID、binding schema 与 initial/on-demand partition **不由本轮候选表冒充生产合同**。实施开始时先对当前系统、`../codex/`、第一方插件和可复用能力完成 inventory，再生成唯一、机器可读的 `OfficialPresetSeedManifest`；G0 Contract Closure Gate 必须在任何 production seed/migration 之前冻结并审查该 manifest。只要 manifest 满足上述政策，就不再逐 Capability 向用户确认；只有准备偏离 `chat.minimal` exact-empty、Coding 完整或 role-complete/context-minimal 政策时，才升级为新的用户决策。

### D-024：AgentSession 删除、tombstone 与真实 Effect 历史保留

- 状态：`已确认`。
- 用户裁决：采用方案 A。
- 结论：删除 AgentSession 的全部内容，只留下不可恢复、不可继续执行的最小 deletion tombstone；已经发生的真实业务 Effect 事实由 owning plugin/domain 独立保留，不随 Chat/Session 级联删除。

固定行为如下：

1. 删除开始后立即停止该 AgentSession 的新 Turn/Effect admission，取消或结束既有 Runtime 工作并释放 handle；完成删除后，原 Session 不可 resume、observe、fork、restore 或重新绑定 Runtime；
2. 删除 `session_events`、`session_payloads`、UI Projection、消息、标题、附件/临时内容、Prompt、Tool 参数/结果、模型输出、Session 级 Effect view/receipt、Runtime binding、checkpoint 及其他可恢复执行上下文；
3. `agent_sessions` 只保留 `agent_session_id`、owner reference、`state=deleted` 与 `deleted_at`。该 row 不是 soft-delete 内容容器，只承担 ID 防复用、引用围栏和确定性删除状态；
4. 重复 delete、迟到的 Remote/Runtime request、Tool ACK、Effect ACK 或 callback 统一幂等返回 `SESSION_DELETED`，不得重建 Event、Projection、binding 或执行工作；
5. 已经作用于外部世界的 Effect 不因删除 Session 而被伪装撤销。owning plugin/domain 的 Effect、idempotency、receipt/reconciliation、业务记录和可靠 outbox/inbox 事实不级联删除，只可保留指向 tombstone `agent_session_id` 的最小来源引用，不得复制或保留已删除的 Session 内容；
6. 普通 Chat/Session、Coding、Remote、Robot、业务入口及 D-022 创建的 Test AgentSession 使用完全相同的删除主链；不建设 test-only 删除分支、retention/restore 平台、恢复 API、回收站或长期归档状态机。

方案 A 将“用户会话内容确实删除”与“外部业务事实不能随聊天消失”分开，只用一条极小 tombstone 处理重复请求、迟到回调与来源引用，不引入完整 soft-delete/restore 体系。

## 已确认决策（续）

### D-025：v4 升级后旧 Snapshot 的可执行性

- 状态：`已确认`；用户选择方案 A。
- 本项只处理 v4 自身 append-only 升级后仍存在的历史 AgentSession/Snapshot，不为 pre-v4 数据、D-013 archive 或 D-024 tombstone 提供兼容入口。

固定合同如下：

1. 未删除 AgentSession 的 canonical SessionEvent 与 Projection 始终可读；Runtime checkpoint 仍是可丢弃 cache，不开发 converter；
2. 每次 resume 或新 Turn 前执行 deterministic compatibility admission。Snapshot identity 不变；Runtime build 可以更新，但 release/hello manifest 必须完整支持 Snapshot 锁定的 schema、protocol、RuntimeProfile、native features/actions、完整 initial + on-demand ceiling、Package/Capability/Skill/MCP materialization/schema identity、model route/config 和 typed resource binding contract，不能只检查 active set；
3. checkpoint 的 `runtime_bound_event_ref` 所指 build identity、protocol、Snapshot 与 `through_seq` 任一不匹配就丢弃。admission 通过后，从 exact Snapshot、latest completed compaction 与后续 canonical Events 创建新 Runtime binding，继续原 `AgentSessionId`；
4. 结构不兼容时，原 Session 在该执行栈下只读，Turn/Tool/Effect 确定性返回 `SNAPSHOT_EXECUTOR_UNAVAILABLE`；不得自动 upcast、重新 resolve、改写 Snapshot、换 latest capability 或隐式 rebind。Provider/网络/credential/resource 的临时不可用属于普通运行错误；
5. continuation 只发生在 completed-turn boundary。active Turn、Tool/Effect dispatch 或部分模型输出中途不得换 Runtime；遗留 Effect 必须先落入 succeeded/failed/uncertain，且 resume/fork/rehydrate 不重放 Effect；
6. 用户显式选择可用 AgentBinding/resources 后可 fork 新 `AgentSessionId`。child 使用自包含、有界 fork base，不依赖父 checkpoint、不复制整份 transcript、不迁移 Runtime-private PTY/process/handle；原 Session 永不 rebind；
7. Coding 只有完整 `coding.codex-native` contract 可执行时才能 continuation；不兼容时 fork 到当前完整 Coding Snapshot，不允许静默降级，也不承诺永久保存所有旧 Runtime/Package 制品。

方案 B 的永久多版本制品会重建兼容平台债务；方案 C 的自动 upcast 会让 immutable Snapshot 和 Effect 语义静默漂移，因此均被拒绝。

### D-026：Remote installation token rotate/revoke 语义

- 状态：`已确认`；依照用户授权选择方案 A：**request-admission fence**。

固定合同如下：

1. rotate/revoke 在 Remote ingress auth record 的单笔事务中提交新的 generation/status；该 commit 是唯一 fence，不增加第二个 revoke coordinator；
2. fence commit 之后，旧 token 发起的每个**新** `open/turn/observe/cancel` admission 都立即、确定性返回 `REMOTE_AUTH_REQUIRED`；transport 在认证完成前不得读取、创建或改变 AgentSession；
3. fence commit 之前已经 durable accepted 的请求不被追杀：已接受的 Turn 运行到它本来就有的普通 finite boundary，已接受的 observe/cancel/open 按普通协议完成。这里是 admission linearization，不是按时间延续的 grace period；
4. token rotate/revoke 不改变、删除、fork 或级联 cancel 任何 AgentSession，也不修改其 frozen Snapshot/Binding。它只拒绝旧凭据的新 ingress admission；
5. replacement token 认证为同一 installation owner 后，客户端可携带显式 `agent_session_id` 继续现有 Session，并继续服从 D-025 compatibility、D-024 deletion 和 Session 自身 lifecycle；
6. 不建设 token scope、TTL/expiry、grace window、kill switch、per-token Session provenance、token→Session 索引、后台 revoke worker 或审批状态。

该方案复用已有 request admission transaction，ROM 增量为 `0/0 EW`。把 revoke 解释成 Session cancel 会混淆认证事实与 Session lifecycle；保留 TTL/grace/scope 则会重新引入本次已删除的授权平台，均被拒绝。

### D-027：Internal canary sticky Nomi Session 排空

- 状态：`已确认`；依照用户授权选择方案 A：**finite-boundary drain 后强制归零**。

固定合同如下：

1. 每个 Domain Wave 或全局删除先提交 Nomi new-admission fence；commit 后不再创建或分配任何 Nomi Session/Turn；
2. 没有 durable accepted operation 的 opening/ready/idle Session 立即执行 `cancel → dispose Runtime → kill descendants → zero handles → D-024 delete`；fence 前已经 durable accepted 的 Turn/operation 只允许运行到其自身与全部祖先**既有 finite deadlines 的最小值**。缺少、已过期或无法解析 deadline 时立即进入第 3 步；不得为 drain 延长 deadline，也不增加 Session 级或可配置 canary drain timeout；
3. deadline 到达或 Turn 提前结束后，Supervisor 依序执行 `cancel → dispose Runtime → kill descendant process tree`，停止新的 Runtime/Tool/Effect callback producer；
4. 已记录 `effect/started` 但最终外部结果不可证明时，必须在删除 Session 内容前 durable 写入 `effect/uncertain`，并用原 effect/idempotency identity 把 reconciliation 交还 owning plugin。Nomi deletion 不等待业务 reconcile 完成，也不自动 retry/replay Effect；
5. Handoff durable 后才关闭剩余 task/queue/writer/lease/resource handle并证明 Nomi outstanding exact-zero，再对 sticky Nomi AgentSession 走普通 D-024 delete closure，并在同一个 Slice/Wave 删除 Nomi wiring；领域 Effect/idempotency/reconciliation 事实继续 non-cascade；
6. 禁止把原 AgentSession 切到 Codex Runtime，禁止 per-Turn fallback、continuation converter、产品可配置 drain timeout、固定观察期或长期 coordinator。需要继续工作时按 D-025 显式 fork 新 Codex Session。

纯自然等待可能永久阻塞 hard delete；立即无界强杀会跳过既有正常边界。所选方案复用请求 deadline、Supervisor dispose 与 D-024 delete，逻辑最小且给 C8/C9 一个确定终点。

### D-028：正式运行与发布平台矩阵

- 状态：`已确认`；依照用户授权选择方案 A：**分层、有限的 required native matrix**。

#### 首个 Stable required local cells

| Product cell | Host | Codex sidecar | Release Gate |
|---|---|---|---|
| Windows Desktop x64 | Windows x64 | Windows x64 | 本机 build/package、完整 Coding、生命周期/fault/process-tree cleanup |
| macOS Desktop x64 + arm64 | 单个 Universal App | x64 与 arm64 两套 sidecar | 两种架构分别在真实设备运行完整 Gate；不能用 Universal 包替代双架构执行证据 |
| Linux Desktop x64 | GNU Host | musl x64 sidecar | GNU Host + musl sidecar 组合的 build/package、完整 Coding、lifecycle/fault Gate |
| Linux Headless x64 | GNU Host | musl x64 sidecar | headless install/service、完整 Coding、Remote/lifecycle/fault/process cleanup |

每个 required local cell 都必须提供完整 `coding.codex-native`，不得把 Linux/headless 降级为轻量问答 Runtime。Browser/Computer Capability 按 Host 实际 availability 编译并由 Snapshot 精确表达：Linux Computer 如保留，必须注册为独立、canonical、明确标注 partial 的 Capability，不能伪装成跨平台完整能力；Headless 的 Browser 和 Computer 均为 exact-unavailable，不注入 schema、provider 或 fallback。

#### 开发、原生验证与人工 handoff 顺序

D-028 同时固定 D-019/C8 的平台实施顺序，目标是先用当前 Windows 环境快速闭合全局功能，再把每个平台专属验证交给真实原生 Host：

1. **Windows 连续完成 C1～C7：**Windows Desktop x64 是唯一主开发和首个全功能验证平台。C1～C7 的 FullAuto 删除、Fresh-v4、Kernel/Plugin、Runtime/Model、Preset Product、三联 Gate、全部 Domain slices、同改同删和中央集成必须在 Windows 阶段连续推进；不得因完成某个功能、模块、Capability、Domain Wave 或发现某个跨平台待验点而暂停/通知换机；
2. **跨平台代码只累计待验点：**C1～C7 期间可以在 Windows 编写和调试 portable Rust/TypeScript、接口、feature/target wiring、条件编译和平台 adapter，但只把尚未在目标 Host 执行的行为持续累计到 `PlatformVerificationPoint` ledger，不逐项验证、不逐项 handoff，也不因此中断 Windows 主开发。Windows cross-compile、静态检查、VM/模拟器、Rosetta 或包结构检查只能作为 preflight，不能把 macOS/Linux native cell 标记为 `pass`；
3. **C8-WIN-PRE / Windows pre candidate：**C1～C7 全部功能开发与集成闭合后，生成一个可复现的 Windows pre candidate，对完整产品而非单个模块执行 Windows package/install、完整 Coding、全场景 E2E、lifecycle/fault/process-tree cleanup 与 Windows pre-version full Gate。平台内发现的问题必须集中记录、批量修复并重跑受影响 Gate；修复循环内部不设 feature-level/module-level pause；
4. **HP-1 / 第一次强制暂停：**只有 Windows pre candidate 的全功能/full Gate 整体通过后，当前实施任务才必须 `PAUSE` 并通知用户。通知至少包含 current pre-candidate source SHA、contract/platform manifest digest、Windows evidence、累计且未关闭的 cross-platform verification points，以及在真实 macOS arm64 上继续的准确入口；此前不存在任何 HP；
5. **C8-MA / macOS arm64 整体阶段：**用户切换到真实 Apple Silicon macOS 后，以完整 Windows pre candidate 为输入，连续完成整个产品的 macOS arm64 适配、Universal packaging arm64 leaf、完整 Coding、Browser/Computer availability、lifecycle/fault/process cleanup，以及全部 arm64 verification points 和 full native Gate。平台内发现的问题同样批量修复并重新验证整个受影响集合，不按功能、模块或单个 verification point 暂停；
6. **HP-2 / 第二次计划内暂停：**只有整个 macOS arm64 pre candidate 的平台适配与 full native Gate 整体通过后才再次 `PAUSE` 并通知用户。中央 owner 随后冻结 canonical cohort tuple：`candidate_source_sha + confirmed_decision_contract_digest + platform_validation_manifest_digest + runtime_release_digest`，并给出三条互相独立、可在其他电脑并行执行的 native validation task：真实 Intel macOS Desktop x64、Linux Desktop x64、Linux Headless x64。只要 C8-MA 的 canonical cohort tuple 任一字段不同于 C8-WIN-PRE，同一 HP-2 批次就必须包含 Windows：命中影响集时跑完整受影响 Gate，未命中时跑新 tuple scoped attestation；只有四字段 exact-equal 才可直接沿用 Windows pass。所有任务从同一个 frozen tuple 开始；
7. **C8-MX / C8-LD / C8-LH 三机并行原生验证：**每个任务只能对自己的真实 Host/cell 出具 PASS，并把结果汇回中央 integration owner。macOS x64 必须在真实 Intel Mac 执行，Rosetta 不能替代；两个 Linux cell 必须分别在真实 GNU Desktop 和 GNU Headless 环境完成。每个任务也以整个 candidate/cell 为单位连续完成适配、累计问题、批量修复和 full native Gate，不按功能/模块暂停或立即要求其他平台换机。Windows、交叉编译、静态检查、容器模拟或另一 cell 的成功都不能替它们签发证据；
8. **修复失效与批量收敛规则：**平台任务只登记本轮发现的 local/shared fixes 与 `affected_cell_ids`，不因单个修复要求其他平台切换。等当前整个平台批次全部完成后，中央 owner 一次合入该批全部 shared/platform fixes并冻结新 cohort tuple；凡 contract、Runtime、UI、packaging 或 build-closure 变更触达的 cell，其既有 PASS 立即 `stale`，必须回对应原生 Host 重验，Windows 只能重验 Windows。平台局部修复也至少使该 cell 失效。未受影响 cell 仍须在新 tuple 的原生 Host 产出 scoped attestation，不能由中央代签；
9. **C8-MERGE / 中央收口与 `C8-RECHECK-n`：**若五格尚未指向同一最终 tuple，中央 owner 在批次边界一次启动 whole-cohort 原生复验：affected cells 跑完整受影响 Gate，unaffected cells 跑新 tuple scoped attestation；五格能并行的同时执行。现有任务/主机仍可用时直接复用，否则一次提醒用户准备缺失的真实平台。只有整轮 recheck 完成后若又产生 shared fix，才允许合并后开启下一轮，绝不按单个改动循环换机。中央 owner 最终收齐五个 cell 的同 tuple evidence、关闭全部 verification points，并完成 all-scene/global zero-outstanding Gate 后，才发送非阻塞完成通知并直接进入 C9，不等待审批。
10. **C9/C10 边界与 `C10-RECHECK-n`：**五个 native cells 的 final-source evidence 任一缺失或失效，C9 Nomi hard-delete 都不得开始。C9 完成后，C10 仍须在正式 RC artifacts 上重跑各 required cell 的最终 package/install/launch/smoke、完整 Coding smoke、sidecar lifecycle/process cleanup 与签名/digest 检查；C8 的开发期原生 Gate 不能替代 C10 的 RC 制品验证。任一 RC cell 失败只登记 fix/affected cells，等待当前五格 RC 轮次全部返回后一次合入整批 forward fixes并冻结新 RC tuple；C10-RECHECK-n 在五格真实 Host 同批执行 affected full RC checks + unaffected new-SHA scoped attestation。只有整轮又产生 shared fix 才开始下一轮，绝不按单修复换机；C10-MERGE 同 tuple 全绿后才允许 C11。

Repo-local `PlatformValidationManifest` 是该工程 Gate 的唯一 schema；状态 exact-set 为 `pending_native_verification | pass | fail | stale`。每个 `PlatformVerificationPoint` 至少记录 `verification_point_id`、owning module、target cell、必须在目标 Host 观察的行为、exact Gate/check、状态与 evidence ref。每个 `PlatformCellEvidence` 至少记录：

- `cell_id` 与 native Host OS/arch/toolchain fingerprint；
- `candidate_source_sha`、`confirmed_decision_contract_digest`、`platform_validation_manifest_digest`、`runtime_release_digest`；
- Host/package artifact digest 与 Codex sidecar digest；
- `coding.codex-native` exact-set digest/result、Host availability manifest digest；
- gate-suite digest/result、已关闭 verification-point IDs、evidence bundle digest；
- 若发生失效，记录 invalidating fix SHA、affected-cell set 与 native revalidation evidence。

Tuple digest 生成顺序固定为无自引用单向链：immutable pre-run `CodexRuntimeReleaseManifest` input payload → `runtime_release_digest` → immutable pre-run `PlatformValidationManifest` input payload → `platform_validation_manifest_digest` → 原生 `PlatformCellEvidence`/post-run `PlatformValidationEvidenceSummary`。两个 input digest 都排除自身字段、status、日志、evidence 与 merge summary；C8/C10-MERGE 只生成独立 post-run summary/envelope，绝不回写 input manifests或改变本轮 tuple。最终 signed release content digest 也是 post-run 输出，不参与本轮 tuple。

这些记录属于 repo-local Gate evidence 和人工任务交接材料，不进入产品数据库、SessionEvent、Runtime protocol、Plugin SPI 或用户 UI，也不建设调度/通知 automation。

#### Remote-only clients

Mobile、Web browser client、Robot firmware 与 IM clients 只经 D-017/D-026 Remote contract 调用 required Host；它们不内嵌 Codex sidecar，不复制 Agent Runtime、Capability Registry 或 Session 事实。

#### 首个 Stable explicitly unsupported

Windows ARM64 与 Linux ARM64 明确为 `unsupported`，不得进入产品 platform selector、download candidate、自动检测 fallback 或“实验可用”状态。未来支持时必须作为新的 required cell，完整通过相同的 Coding、package、protocol、lifecycle/fault 和真机 Gate，不能仅通过交叉编译声明完成。

该矩阵用四个 required 产品单元覆盖正式 Desktop/Headless 主路径；其中 macOS Universal 拆成 x64/arm64 两个原生执行证据，因此机器 Gate 的 exact-set 是五个 native execution cells。未承诺架构从产品候选中彻底拿掉；Windows 连续完成 C1～C7 与 pre candidate full Gate → macOS arm64 整体 candidate Gate → 三机并行的顺序避免在开发早期等待全部 Host，同时又禁止用一个平台的模拟结果冒充另一个平台。计划内 platform-stage handoff 只有 `HP-1/HP-2`；C8-MERGE 收敛过程中，只有整批验证完成且 shared fixes 合并为新候选时才能触发 whole-cohort `C8-RECHECK-n`，其中 affected cells 完整重验、unaffected cells 原生 scoped attestation；单功能、单修复和平台内开发过程换机数为零。真实 HP/recheck 等待时间单独计入 wall-clock，不改变 `213/314 EW`，也不会牺牲 required cells 的 Codex Coding 能力。

设计决策已经闭合并经用户整体确认；下一任务从 Contract Closure/G0 启动。本设计提交只固化合同与启动入口，不表示 production code 已经实现。
