# 机器 2 单 SSH Prompt 已废止

> 状态：**SUPERSEDED / DO NOT START**
>
> 修订日期：2026-09-02
>
> 原分支：`rf/m2-w2-ssh-owner`
>
> 替代分支：`rf/m2-phase1-batch-a`
>
> 替代 Prompt：
> `docs/specs/2026-08-28-agent-capability-platform-v2/MACHINE-2-PHASE1-BATCH-A-START-PROMPT.zh.md`

该分支原本只承载 `SL-S3-09` SSH 单项任务。经阶段复核，单独启动另一台机器的
交接、同步和合并成本高于可获得的并发收益，因此旧任务不再启动。

机器 2 只有在能够完整承担 Phase 1 Batch A 时才启用。Batch A 同时包含：

- `SL-S2-05` SessionEvent/Projection 收缩；
- `SL-S2-06` 三类 Effect 策略收缩；
- `SL-S2-10` official app-server upstream spike；
- `SL-S3-09` 精简 SSH owner。

请不要在本分支继续开发、提交或创建新的 SSH-only worktree。请改用
`rf/m2-phase1-batch-a`，并严格遵循替代 Prompt 中的独占写集、提交边界和测试要求。

本分支保留为历史审计，不删除、不 reset、不 force-push。
