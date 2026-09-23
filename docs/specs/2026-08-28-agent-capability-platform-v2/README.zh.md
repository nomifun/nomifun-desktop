# Agent Capability Platform v2 文档入口

> 更新日期：2026-09-22
>
> 范围：NomiFun Agent 核心的 Package、Capability、Role、Preset、Session 与 Kernel 合同。

本目录只保留仍被 Agent 核心使用的设计与证据。原 Plugin N1 / MiniApp M1 作者合同、
Project/Candidate/Release/Publish 流程、跨机 Prompt 和候选门禁已经退役并物理删除。

Plugin 架构唯一权威是
[`../2026-09-22-unified-plugin-core/README.zh.md`](../2026-09-22-unified-plugin-core/README.zh.md)。
全局 Agent Package/Action 合同仍可继续使用，但不得据本目录恢复 PluginMount、Shared
Extension Host 或旧 Plugin 生命周期。

## 当前阅读顺序

1. `05-system-capability-replacement-foundation.zh.md`
   - Role/Provider 基础与完成定义。
2. `GLOBAL-CLOSURE-TODO.zh.md`
   - 非 Plugin Agent 核心的历史实施证据；不再承载 Plugin 状态。
3. `01-current-state-and-harness-findings.zh.md`
   - Agent 平台现状与可复用接缝。
4. `02-capability-catalog-and-agent-presets.zh.md`
   - Capability/Preset 领域边界。
5. `03-target-architecture.zh.md`
   - Thin Kernel、AgentSession 与 Runtime 边界。
6. `04-migration-and-validation-plan.zh.md`
   - Agent 核心迁移与验证纪律。
7. `DECISIONS.zh.md`
   - 仍有效的 Agent 决策记录。

发生冲突时：

```text
canonical Rust / SQL / generated schema / behavior tests
> Unified Plugin Core（仅 Plugin）
> 05-system-capability-replacement-foundation.zh.md（Agent 核心）
> 01～04 与 DECISIONS 中仍有效的 Agent 设计
```

本目录中的阶段 JSON 只作为 Agent 核心历史自动化证据，不能被解释为当前 Plugin 合同、
机器分工或实时状态。需要查看已删除阶段材料时使用 Git 历史，不得将其恢复为开发入口。
