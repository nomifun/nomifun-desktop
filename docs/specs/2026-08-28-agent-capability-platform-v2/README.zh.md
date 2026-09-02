# Agent Capability Platform v2 文档入口

> 更新日期：2026-09-02
>
> 适用分支：`rf/agent-capability-platform-v2`

本目录保留经当前修订仍有效的核心设计、唯一执行台账、当前机器 Prompt，以及仍被
Gate/Generator 使用的机器文件。设计文档不能因为形成时间较早而整体删除；发生方向修订时，
应在原设计中删除或改写错误条款，并保留仍有效的目标、边界、理由和演进依据。

## 阅读顺序

1. `05-system-capability-replacement-foundation.zh.md`
   - 2026-09-02 一期止损修订、Role/Provider 基础和完成定义，优先于更早的设计条款。
2. `GLOBAL-CLOSURE-TODO.zh.md`
   - 当前唯一执行状态源，记录 owner、依赖、测试、阻塞和剩余工作。
3. `01-current-state-and-harness-findings.zh.md`
   - 经修订的现状审计、问题来源和可复用接缝；其中数字仅代表对应审计时点。
4. `02-capability-catalog-and-agent-presets.zh.md`
   - 经修订的产品术语、领域模型、Capability/Preset 边界和候选目录。
5. `03-target-architecture.zh.md`
   - 经修订的目标架构、Thin Kernel、AgentSession、Runtime 与数据边界。
6. `04-migration-and-validation-plan.zh.md`
   - 经修订的迁移纪律、依赖顺序和验证方法；不记录实时完成状态。
7. `DECISIONS.zh.md`
   - 经修订的决策及理由；被 05 撤销的旧要求不再作为可执行设计保留。
8. `MACHINE-2-PHASE1-BATCH-A-START-PROMPT.zh.md`
   - 仅在机器 2 能完整承担 Batch A 时使用。

发生冲突时：

```text
canonical Rust / SQL / generated schema / behavior tests
> 05-system-capability-replacement-foundation.zh.md
> 01～04 与 DECISIONS 中经修订的设计
> 当前 Machine Prompt
```

`GLOBAL-CLOSURE-TODO.zh.md` 只决定任务状态，不反向改写架构合同；Machine Prompt 只分派
任务，不能覆盖 05 或核心设计。

## 机器文件

以下 JSON 仍由仓库脚本读取，不能仅因名称较旧而删除：

- `C0-WRITE-MANIFESTS.json`
- `C1-WRITE-MANIFESTS.json`
- `C2-C5-WRITE-MANIFESTS.json`
- `C6-WRITE-MANIFESTS.json`
- `C6-CLOSURE.json`
- `C7-WRITE-MANIFESTS.json`
- `C7-CLOSURE.json`
- `C8-WIN-PRE-MANIFEST.json`

它们是 Gate 输入或历史阶段的机器检查点，不是人工设计入口。其内容与 05 或 canonical
代码冲突时，必须修改机器合同/Gate，不能让过期生成物反向覆盖设计。

## 已删除的过期执行文件

`IMPLEMENTATION-STATUS`、旧 `START-PROMPT`、旧 macOS handoff、旧单 SSH Prompt 和
不再消费的 C8 migration batch manifests 只存在于 Git 历史。这些文件记录过期状态、
交接或已撤销执行批次，不属于需要持续维护的核心设计。

需要审计时使用 `git show <commit>:<path>`；不要据此恢复过期任务状态或覆盖 GLOBAL TODO。

## 维护规则

1. 核心设计发生修订时，直接删除或改写错误条款，并保留仍有效的设计依据。
2. 不用笼统的“历史文档”免责声明掩盖正文冲突，也不因局部错误整体删除核心设计。
3. 实施状态只更新 GLOBAL TODO；设计文档不复制 closed/open 数量和临时 commit 进度。
4. 临时 handoff、一次性 Prompt 和测试结果在失效后删除，长期设计理由回写核心文档。
