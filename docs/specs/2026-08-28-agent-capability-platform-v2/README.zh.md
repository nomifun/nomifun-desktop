# Agent Capability Platform v2 当前文档入口

> 更新日期：2026-09-02
>
> 适用分支：`rf/agent-capability-platform-v2`

本目录只保留当前实施需要的人工文档和仍被 Gate/Generator 使用的机器文件。
已被止损方案取代的长篇设计、状态日志和旧 handoff 已从工作树删除，避免模型把历史
描述误判为当前合同。

## 阅读顺序

1. `05-system-capability-replacement-foundation.zh.md`
   - 当前一期架构、止损和完成定义的最高优先级人工指令。
2. `GLOBAL-CLOSURE-TODO.zh.md`
   - 当前唯一执行台账，记录状态、owner、依赖、测试和剩余工作。
3. `MACHINE-2-PHASE1-BATCH-A-START-PROMPT.zh.md`
   - 仅在机器 2 能完整承担 Batch A 时使用。

发生冲突时：

```text
canonical Rust / SQL / generated schema / behavior tests
> 05-system-capability-replacement-foundation.zh.md
> GLOBAL-CLOSURE-TODO.zh.md
> 当前 Machine Prompt
```

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

它们是 Gate 输入或历史阶段的机器检查点，不是当前人工任务入口。其内容与 05 冲突时，
必须修改机器合同/Gate，不能恢复已删除的旧 Markdown 作为第二事实源。

## 历史审计

旧 01～04、DECISIONS、IMPLEMENTATION-STATUS、START-PROMPT、旧 macOS handoff、
旧单 SSH Prompt 和 C8 migration batch manifests 只存在于 Git 历史。

需要审计时使用 `git show <commit>:<path>`；不要把历史文件恢复到当前工作树，也不要
据此新增任务或覆盖 GLOBAL TODO 状态。
