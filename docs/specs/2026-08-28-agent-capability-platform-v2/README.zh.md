# Agent Capability Platform v2 文档入口

> 更新日期：2026-09-02
>
> 适用分支：`rf/agent-capability-platform-v2`

本目录保留经当前修订仍有效的核心设计、唯一执行台账，以及仍被 Gate/Generator 使用的
机器文件。设计文档不能因为形成时间较早而整体删除；发生方向修订时，应在原设计中删除
或改写错误条款，并保留仍有效的目标、边界、理由和演进依据。本轮所有实现、修复、测试
编排和 merge 均由当前主机负责，以互斥写集的本机并发 lane 推进。跨机开发分配、第二台
开发机入口、远端 SHA 同步、专用 Prompt/manifest/result template 和跨机 attestation
均不是当前执行要求；这些概念只可作为明确标注已废弃的历史理由出现在核心文档或 Git
历史中。

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
发生冲突时：

```text
canonical Rust / SQL / generated schema / behavior tests
> 05-system-capability-replacement-foundation.zh.md
> 01～04 与 DECISIONS 中经修订的设计
> GLOBAL-CLOSURE-TODO.zh.md 中的当前主机任务分配
```

`GLOBAL-CLOSURE-TODO.zh.md` 只决定任务状态，不反向改写架构合同；当前主机的并发 lane
只按其中声明的依赖和写集推进，不能覆盖 05 或核心设计。

## 单机多并发规则

当前主机是唯一实现与集成主机。多个 lane 可以在本机并行；每个 lane 必须有明确且互斥
的路径写集，不得同时编辑同一文件，也不得并发运行会争用同一数据库、固定端口、构建
目录或进程树的重测试。临时本地 worktree 只是一种可选隔离手段，不形成长期分支或独立
交付入口。中央合同、组合根、Gate、锁文件和 GLOBAL TODO 由主机串行合流。

## Gate/Generator 输入文件

以下 JSON 仍由仓库脚本读取，不能仅因名称较旧而删除：

- `C0-WRITE-MANIFESTS.json`
- `C1-WRITE-MANIFESTS.json`
- `C2-C5-WRITE-MANIFESTS.json`
- `C6-WRITE-MANIFESTS.json`
- `C6-CLOSURE.json`
- `C7-WRITE-MANIFESTS.json`
- `C7-CLOSURE.json`
- `C8-WIN-PRE-MANIFEST.json`

它们是 Gate 输入或历史阶段的自动化检查点，不是人工设计入口，也不是跨机交接材料。
其内容与 05 或 canonical 代码冲突时，必须修改自动化合同/Gate，不能让过期生成物反向
覆盖设计。

## 已删除的过期执行文件

`IMPLEMENTATION-STATUS`、旧 `START-PROMPT`、旧 macOS handoff、旧跨机批次 Prompt/清单/
结果模板和不再消费的旧 C8 migration batch manifests 只存在于 Git 历史。这些名称仅
用于说明已撤销的执行方式，不属于当前主机的执行材料，也不得据此恢复跨机任务。

需要审计时使用 `git show <commit>:<path>`；不要据此恢复过期任务状态或覆盖 GLOBAL TODO。

## 维护规则

1. 核心设计发生修订时，直接删除或改写错误条款，并保留仍有效的设计依据。
2. 不用笼统的“历史文档”免责声明掩盖正文冲突，也不因局部错误整体删除核心设计。
3. 实施状态只更新 GLOBAL TODO；设计文档不复制 closed/open 数量和临时 commit 进度。
4. 一次性执行说明和临时测试记录在失效后删除，长期设计理由回写核心文档。
