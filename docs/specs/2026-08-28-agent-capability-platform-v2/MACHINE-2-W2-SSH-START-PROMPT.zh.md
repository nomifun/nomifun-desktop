# 机器 2 单 SSH Prompt 已废止

> 状态：**SUPERSEDED / DO NOT START**
>
> 修订日期：2026-09-02
>
> 原分支：`rf/m2-w2-ssh-owner`
>
> 替代方案：`MACHINE-2-PHASE1-BATCH-A-START-PROMPT.zh.md`

单独为 `SL-S3-09` 启动另一台机器，交接、同步和合并成本高于可获得的并发收益。
因此旧的单 SSH lane 不再启动，也不能继续作为当前机器 2 工作指令。

机器 2 只有在能够完整承担以下四项 Batch A 时才启用：

- `SL-S2-05` SessionEvent/Projection 收缩；
- `SL-S2-06` 三类 Effect 策略收缩；
- `SL-S2-10` official app-server upstream spike；
- `SL-S3-09` 精简 SSH owner。

请改用：

`docs/specs/2026-08-28-agent-capability-platform-v2/MACHINE-2-PHASE1-BATCH-A-START-PROMPT.zh.md`

旧分支保留作审计，不删除、不 force-push、不继续开发。
