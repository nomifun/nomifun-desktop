# NomiFun Coding Agent Runtime 内化阶段文档入口

> 阶段代号：`CAR`（Coding Agent Runtime Internalization）
>
> 文档日期：2026-09-06
>
> 文档状态：**IMPLEMENTATION IN PROGRESS / 本地隔离切片已启动**

本目录是 NomiFun 下一阶段 Coding Agent Runtime 内化的独立设计与实施入口。
它不属于 `2026-08-28-agent-capability-platform-v2` 一期文档，也不修改一期文档
中的任何设计、状态、Manifest 或 Gate。

本阶段的目标是：把 `../codex/` 中成熟的 Coding Agent 机制选择性抽取、二次开发，
内化为 NomiFun 自己的可选进程内 Coding 执行引擎；模型、认证、Session、Capability、
File、Process、VCS、MCP、Plugin 和 MiniApp 继续使用 NomiFun 自己的所有权和主链。
当前先以独立 `nomifun-coding-engine` crate 开发和验证，暂不接入一期主应用；远程主
工作进程只负责后续合并、统一 Registry/AgentSession 接线、Broker 原生取消、
SessionEvent 投影、联调和验收。

## 最重要的执行规则

1. `docs/specs/2026-08-28-agent-capability-platform-v2/` 在本阶段视为只读背景。
   不修改其中的 01～06、`DECISIONS`、`GLOBAL-CLOSURE-TODO`、既有机器 Manifest
   或一期 Prompt。
2. 本目录拥有自己的文档事实、任务状态、Prompt 和验证门禁；本阶段不复用一期
   `GLOBAL-CLOSURE-TODO` 作为状态源。
3. 本阶段不运行 `codex-app-server`，不引入 Codex Sidecar，不复制 app-server
   protocol，不把 `../codex` 作为 Cargo path dependency。第二个 Coding Engine
   与现有 Legacy Engine 可以并存，但本阶段不切换旧生产路由；多个 Engine Build
   可以并存，每个 Session 只绑定一个 exact Build。
4. voice、realtime、audio、TUI、CLI、Guardian、Codex 登录/认证产品层、
   Codex Plugin Manager 和 Codex rollout store 不进入本阶段生产实现。
5. 任何代码任务都必须只领取一个 `CAR-*` 原子任务，遵守该任务的写集、禁区、
   完成定义和停止条件。
6. 本目录的设计字段不是第二套 Rust/SQL/API 合同。落地后以 canonical code、
   schema、行为测试和 `TASK-MANIFEST.json` 的实现结果为准。

## 阅读顺序

1. `00-stage-charter-and-rules.zh.md`
   - 阶段目标、边界、前置条件、决策层级和原子任务规则。
2. `01-audit-and-source-baseline.zh.md`
   - NomiFun 与 Codex 的代码质量、可抽取模块、排除模块和风险基线。
3. `02-target-architecture-and-port-contracts.zh.md`
   - 进程内 Runtime 架构、crate 边界、Port 和所有权。
4. `03-model-provider-adaptation.zh.md`
   - NomiFun ChatModelBroker 适配、模型事件、Credential、重试和 compaction route。
5. `04-tool-loop-and-coding-capabilities.zh.md`
   - Coding Tool Loop、Capability 映射、Process/Patch/VCS 和并行规则。
6. `05-session-context-and-lifecycle.zh.md`
   - SessionEvent、Context、AGENTS.md、Compaction、Resume、Steer、Cancel 和 Dispose。
7. `06-implementation-plan-and-atomic-tasks.zh.md`
   - `CAR-00`、`CAR-00A`～`CAR-10` 的依赖图、写集、交付物和验收条件。
8. `07-validation-release-and-cutover.zh.md`
   - 测试、发布、旧 Wrapper 删除、平台验证和回滚边界。
9. `STATUS.zh.md`
   - 本阶段唯一任务状态源。
10. `TASK-MANIFEST.json`
    - 机器可读的任务、写集、依赖和检查清单。
11. `CODEX-SOURCE-MANIFEST.json`
    - 固定 Codex 源快照、选择性抽取映射、排除项和许可证门禁。
12. `DECISIONS.zh.md`
    - 本阶段独立决策记录。
13. `PROMPT-DOCUMENT-MAINTENANCE.zh.md`
    - 维护本系列设计文档的可复制 Prompt。
14. `PROMPT-PHASE-START.zh.md`
    - 启动本阶段代码实施的可复制 Prompt。
15. `PROMPT-WORKER-TASK.zh.md`
    - 启动单个 `CAR-*` 原子任务的可复制 Prompt 模板。
16. `PROMPT-CAR-00-START.zh.md`
    - 第一个源码/许可证基线任务的可直接执行 Prompt。
17. `HANDOFF-REMOTE-INTEGRATION.zh.md`
    - 当前本地隔离实现、远程合并顺序、联调责任和禁止事项。
18. `PROMPT-REMOTE-INTEGRATION.zh.md`
    - 在远程主工作进程电脑启动合并和中央接线的可复制 Prompt。

## 阶段主链

```text
CAR-00 源码/许可证/边界冻结
   ↓
CAR-00A 多 Engine Catalog/Binding/灰度合同
   ↓
CAR-01 Coding Engine 隔离 Core
   ↓
CAR-02 ChatModelBroker Coding 适配
   ↓
CAR-03 Tool Loop 与 Capability Mapping
   ├─→ CAR-04 Process/PTY/取消/清理
   ├─→ CAR-05 Patch/File/VCS/Workspace
   └─→ CAR-06 Context/AGENTS/Compaction/Resume
             ↓
        CAR-07 AgentSession 主链切换
             ↓
        CAR-08 旧 Wrapper/Sidecar/打包残留删除
             ↓
        CAR-09 Plugin/MCP/Skill/非 Agent Consumer 扩展
             ↓
        CAR-10 三平台发布与 Stable admission
```

`CAR-04`、`CAR-05`、`CAR-06` 只有在其依赖的 Port 和 Tool Loop 合同稳定后才可并行。
`CAR-09` 不重新定义 Plugin/MiniApp，它只验证平台贡献可以被 Agent 和非 Agent
消费者按各自合同消费。

## 当前本地开发切片

当前工作区已经启动一个不接入生产主链的独立 crate：

```text
crates/backend/nomifun-coding-engine/
  Engine Family/Build Catalog
  Stable/Canary + exact EngineBinding
  provider-neutral model Port
  bounded Tool Call continuation
  read-only parallel / effectful serial dispatch
  cancellation / dispose / one-active-turn admission
  Kernel Snapshot/active-set Tool adapter
  standard Inspect/Edit/Execute/Full Coding Tool surface
  nomi-process-runtime owner adapter
  AGENTS.md/context/compaction/checkpoint contracts
```

这不是一期主应用的路由切换，也不是第二套产品 Session 或权限事实。它只为远程
主工作进程提供可合并、可测试的 Coding Engine 实现切片；Broker 的原生取消传播、
SessionEvent/AgentSession 主链和异构 Registry 仍需在远程联调阶段按 `CAR-02`～`CAR-07`
完成，Kernel/Process adapter 不应被重复实现。

## 本阶段完成后的用户结果

用户可以在现有 Agent 工作台中：

```text
选择全面 Coding 能力
→ 绑定 Workspace
→ 读取/搜索代码
→ 修改或 Patch 文件
→ 运行测试/构建命令
→ 查看 Diff
→ 显式 Commit
→ 取消运行中的命令
→ 恢复 Session 并继续工作
```

上述流程由所选的 NomiFun Coding Engine Build、AgentSession、Snapshot、Capability
allowlist 和 NomiFun owner 主链完成。旧 Engine 与新 Engine 可以在早期并存；
一个 Session 始终只使用一个 exact Engine Build。
