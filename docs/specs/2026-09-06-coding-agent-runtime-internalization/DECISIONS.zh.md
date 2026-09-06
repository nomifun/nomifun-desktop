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
| CAR-D-006 | 产品事实 | `AgentSession`/`SessionEvent` 是唯一产品事实，Runtime cache 可丢弃 |
| CAR-D-007 | 文件与进程 | File、Patch、VCS、Process、PTY 由 NomiFun owner 管理 |
| CAR-D-008 | 安全语义 | FullAuto + Snapshot/ThinAuthority；不迁移 Codex Guardian/Approval |
| CAR-D-009 | 范围排除 | voice、realtime、audio、TUI、CLI、Codex Auth/Provider/rollout 永久排除 |
| CAR-D-010 | 生态边界 | Plugin、MiniApp、MCP、Skill 是平台能力供给或消费者适配，不是 Runtime 私有插件 |
| CAR-D-011 | 多引擎并存 | Legacy Engine 与 Coding Engine 可并存；每个 AgentSession 只绑定一个 exact Engine Build |
| CAR-D-012 | 灰度语义 | Stable/Canary 是 Catalog 指向 immutable Build 的别名，不是 Build 自身属性 |
| CAR-D-013 | 切换边界 | Engine 选择只作用于新建 Session 或显式 Fork；同一 Session 不切换、不静默 fallback |
| CAR-D-014 | 本地施工形态 | 当前电脑先交付未接生产组合根的 `nomifun-coding-engine`；远程主工作进程负责统一 Registry、联调和验收 |
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

如果新的决定会改变以下任一项，必须新建或修改一个 CAR 任务，并在本文件追加记录：

- Runtime 是否进程内；
- 模型或 Capability 是否出现旁路；
- SessionEvent 是否仍是唯一事实；
- 是否引入新的持久状态机；
- 是否把 Plugin/MiniApp 变成 Agent 专属系统；
- 是否恢复旧阶段文档或旧 `/api/presets`；
- 是否允许同一 Session 切换 Engine 或将 Engine failure 改为自动 fallback。

不得通过修改旧阶段文档来“隐式”改变 CAR 决策。
