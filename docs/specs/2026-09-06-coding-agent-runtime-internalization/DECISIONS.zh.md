# CAR 阶段决策记录

> 文档状态：**DESIGN DECISION LEDGER / 2026-09-06**
>
> 本文只记录 CAR 阶段决策，不记录代码完成状态。状态以 `STATUS.zh.md` 为准。

## 已锁定决定

| ID | 决定 | 当前结论 |
|---|---|---|
| CAR-D-001 | Runtime 归属 | Coding Agent Runtime 内化为 NomiFun 核心进程内能力 |
| CAR-D-002 | Codex 使用方式 | 选择性抽取源码和行为，不复制整个 `codex-core` crate graph |
| CAR-D-003 | 外部进程 | 生产不运行 `codex-app-server`、Codex Sidecar 或 app-server protocol |
| CAR-D-004 | 模型入口 | 所有模型请求只经过 `nomifun-chat-model-broker` |
| CAR-D-005 | Tool 入口 | 所有 Tool 调用只经过 NomiFun Capability Kernel 和 owning domain |
| CAR-D-006 | 产品事实 | 唯一生产 Session owner 保有事实与恢复权，Runtime cache 可丢弃；本地落地按 CAR-D-019 使用现有 Conversation 链，不另建 SessionStore |
| CAR-D-007 | 文件与进程 | File、Patch、VCS、Process、PTY 由 NomiFun owner 管理 |
| CAR-D-008 | 安全语义 | FullAuto + Snapshot/ThinAuthority；不迁移 Codex Guardian/Approval |
| CAR-D-009 | 范围排除 | voice、realtime、audio、TUI、CLI、Codex Auth/Provider/rollout 永久排除 |
| CAR-D-010 | 生态边界 | Plugin、MiniApp、MCP、Skill 是平台能力供给或消费者适配，不是 Runtime 私有插件 |
| CAR-D-011 | 多引擎并存 | Legacy Engine 与 Coding Engine 可并存；每个 AgentSession 只绑定一个 exact Engine Build |
| CAR-D-012 | 灰度语义 | Stable/Canary 是 Catalog 指向 immutable Build 的别名，不是 Build 自身属性 |
| CAR-D-013 | 切换边界 | 按 CAR-D-020 在 Agent 工作台配置，引擎变更作用于后续新会话；Fork 继承父绑定；同一 Session 不切换、不静默 fallback |
| CAR-D-014 | 本地施工形态 | 源分支隔离交付已取入；后续开发统一在本地 `rf/agent-capability-platform-v2`，不 push；原远程接线安排由用户新指令替代 |
| CAR-D-015 | 隔离 Catalog 范围 | `CodingEngineCatalog` 只管理 Coding family Build；最终异构 Registry 属于 Agent Platform |
| CAR-D-016 | Codex 源基线 | CAR 独立固定 `../codex` commit `6af345407d9c2a568da9d01b6c4b81a9e61495c0`；一期旧 Sidecar 合同中的其他 frozen SHA 不得复用为新 Engine provenance |
| CAR-D-017 | 标准 Tool 层级 | Inspect/Edit/Execute/Full 只是工作台/ToolPlan 筛选；Compiled Snapshot 仍是能力上限 |
| CAR-D-018 | Process 复用 | Coding Engine 提供 `ManagedCodingProcessOwner` 适配 `nomi-process-runtime`，但模型调用仍必须经过 Kernel/Wave2 owner |

## 默认建议、实施前可拍板

以下事项不改变总体架构，但在对应任务开始前可以由产品负责人明确调整：

1. 首发 Coding route 是否要求 Reasoning。默认建议：Tool Calls 和文本输入/输出为
   必需；Reasoning 按 route 能力和 Revision 选择启用。
2. Subagent 是否进入 Coding P0。默认建议：先完成 AgentSession fork seam，完整
   Subagent workflow 后置。
3. PTY 是否纳入所有首发平台。默认建议：保留 Port 和 Windows/Unix 行为测试，
   平台不支持时返回 typed unavailable。
4. 是否保留 Native Responses Items。默认建议：作为可选 Broker feature，不作为
   通用 Runtime 必需能力。
5. 远程集成时是否物理拆出 `nomifun-agent-runtime` 与 `nomifun-coding-runtime`。
   默认建议：先保持已验证的 public Port 和行为测试，再按真实复用边界拆分；禁止
   为匹配文档名称复制第二份 actor/turn loop。

## 决策变更规则

### CAR-D-019：保留当前生产 Session owner，开放 Runtime 接入（2026-09-13）

主重构分支当前默认运行 `NomiCoreSessionOwner → ConversationService →
AgentRuntimeRegistry`，并有回归测试禁止将 Fresh-v4/Codex Session 路由接到
默认产品链。原 CAR 交接中的 `AgentPlatform → AgentSessionStore/SessionEvent`
不是该生产链，不能仅通过它完成桌面 Coding Engine 嵌入。

结合用户要求“开放平台、可替换 Runtime、全部在本地主重构分支完成”，本地实施
采用当前 Nomi-core owner，在现有 runtime factory/handle 接入开放实现；不把
整体 Session 存储迁移作为引擎集成的隐含前置，不引入第二套 Session 事实。

已实现 `RegisteredAgentRuntime`、通用 `RuntimeEngineCatalog` 与
`CodingAgentRuntime` 适配器。Nomi 也使用 Registered 生产句柄；新增 Runtime
不增加具体引擎枚举分支。factory 是受信任进程内代码扩展点，不是沙箱或稳定
动态库 ABI。family/profile/channel 使用开放标识，exact Build 校验失败不回退。

CAR-07 仍是 in_progress：真实生产 owner ports、创建/Fork 持久绑定及默认路由
尚未接通，不把适配器 fixture 测试当作产品嵌入验收。下一步是实现接线，不再
等待同一架构选择的重复确认。证据见 `LOCAL-INTEGRATION-2026-09-13.zh.md`，
二次开发合同见 `RUNTIME-EXTENSIONS.zh.md`。不修改上一阶段文档。

如果新的决定会改变以下任一项，必须新建或修改一个 CAR 任务，并在本文件追加记录：

- Runtime 是否进程内；
- 模型或 Capability 是否出现旁路；
- SessionEvent 是否仍是唯一事实；
- 是否引入新的持久状态机；
- 是否把 Plugin/MiniApp 变成 Agent 专属系统；
- 是否恢复旧阶段文档或旧 `/api/presets`；
- 是否允许同一 Session 切换 Engine 或将 Engine failure 改为自动 fallback。

不得通过修改旧阶段文档来“隐式”改变 CAR 决策。

### CAR-D-020：Runtime 配置属于 Agent，而非会话创建页面（2026-09-13）

用户明确指出：首页引擎选择与产品设计不一致，runtime 应在 Agent 工作台由每个 Agent
分别配置。撤销首次生产接线中的首页选择器和创建／Fork 请求引擎覆盖设计。

- 产品关系：Agent 配置决定使用什么 runtime；Session 是某个 Agent 版本的运行实例。
  runtime 实现可被多个 Agent 复用，不是另一套用户可见 Agent 身份。
- 工作台在 Agent 设置中配置开放 family/build/profile，保存到不可变 Revision payload，
  参与摘要、草稿脏状态、预览与兼容性检查。旧版本无字段时继续使用 Nomi。
- 唯一 Session owner 依据已保存 Agent 版本解析并冻结 exact binding；各入口共享此规则。
  `CreateAgentSessionRequest`、`ForkAgentSessionRequest` 不再提供独立 runtime 字段。
- 修改 Agent 引擎只影响后续新会话，旧会话及其 Fork 保留精确绑定。会话内切换 Agent
  时，不允许借此替换引擎；使用不同引擎的 Agent 需要新建会话。
- 开放扩展接口、单一 Session 事实、禁止静默 fallback 和副作用清理证明保持不变。

CAR-07 仍是 in_progress，当前 Coding 的能力限制不因产品入口纠正而消失。
