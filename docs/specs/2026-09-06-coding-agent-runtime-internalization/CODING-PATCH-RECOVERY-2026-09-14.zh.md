# Patch 失败回执与重新观察约束（2026-09-14）

状态：源码实现，未验证。分支 `rf/agent-capability-platform-v2`，未 commit/push。
Coding `host2-coding-loop33`，Nomi `host18`。任务 CAR-03 / CAR-05 / CAR-06。
本文不宣称 CAR 或全部 Engine 能力完成。

## 背景及参考

本地 Codex `codex-rs/apply-patch/src/lib.rs` 的 `apply_hunks_with_options` 保留
`AppliedPatchDelta`，即使应用或结果输出失败也通过 `ApplyPatchFailure` 携带已发生的变化。
借鉴的是“错误不能抹去部分效果”的契约，不是将 Codex 文件执行器/权限体系移入 Engine。
本轮仅阅读本地源码，未运行参考项目或重新核验其版本基线。

原实现有两个缺口：当前目标发布后再发生清理/目录同步错误，该目标不进入回滚列表；
回滚跳过/错误被丢弃，宿主只返回普通错误。现在平台负责效果观察，Coding 自己负责恢复策略。

## 平台实现

- FileService 新增 `apply_patch_with_observation_for_agent_session`，保留旧 AppError API。
  验证/准备错误仍发生在写入前；生产 Wave2 fs.patch 改用带回执的 API。
- 内部发布错误记录 rename/hard-link 是否已成功及临时文件清理是否未确认；标记发生在
  后续清理/目录同步之前。已发布的当前失败目标也进入恢复观察。
- 恢复现有文件仍要求内容符合本次发布字节；恢复成功、恢复已发布但同步/清理未确认、
  内容变化/不可读而跳过、恢复失败分别记录。不将这些历史观察视为当前文件状态。
- 新建文件不再自动 check-then-delete，保留并报告 `retained_created`。平台没有通用原子
  compare-and-unlink 能力；不能通过一次旧内容检查证明随后删除是安全的。
- 临时文件遗留风险另记 `temporary_cleanup_unconfirmed`；该观察不授权自动清理。
- 错误回执使用原始 `request.files` 的零基索引分组，不复制路径/源代码；有界诊断可省略，
  发布/恢复索引不截断。`workspace_patch_failed` v1 JSON 经既有 Kernel 错误通道持久化及重放，
  不增加数据库迁移或平台全局错误类型。
- 成功或失败的 journal 结算错误不再被吞掉。失败回执标注 `journal_settlement=unconfirmed`；
  成功发布但日志未确认则明确报告所有目标已发布、不可自动重试。相同 fs.patch journal 范围
  内只要仍有 started 记录，新幂等键也被拦截；这不是整个 workspace 所有工具的全局锁。

## Coding 执行循环

- 实际 invoker 调用的 Patch 失败才创建待观察目标；规划/指令/steering 延后不产生该状态。
  不解析错误字符串推断“没有写入”，也不因回滚成功记录就跳过新观察。
- 同批次后续调用仍延后；此前读取不算失败后的观察。进程 poll/cancel/close_stdin 可用于
  收束，但其所在批次旧读取不解除约束；活动进程期间的读取也不解除约束。
- 失败后允许授权只读工具及规划，阻止新的效果调用与不透明进程执行。每个目标需经真实
  fs.read 文本首分页获得匹配路径/版本/游标，或 `missing_ok=true` 的 typed absence。
  instruction_scope、搜索、图像、任意文本及失败结果不能解除目标。
- 首分页含平台对完整文件计算的摘要，但只证明新的版本观察，不证明阅读了后续分页；
  模型仍需按任务阅读相关范围。这不是运行测试，也没有新增 fs.read 授权。
- 状态在独立、压缩不可丢弃的上下文槽保留；变化使完成报告/Provider 续接失效，并要求
  重新规划。未处理待观察状态的回合不能发布任务完成。
- 目标数量最多 64、路径总预算 16 KiB；超过预算保守阻止该回合继续效果/完成，不静默省略。

Nomi/源码社区 Engine 共享平台效果回执，不被强制使用 Coding 的循环策略。Agent 工作台 →
immutable revision → exact Session binding 不变；Engine 仍只能源码二次开发后随应用打包注册。

## 尚未完成

这不是多文件事务、操作系统 CAS 或外部编辑/重命名完全隔离。新建文件保留是安全降级，
不是完整回滚；仍需独立的人工处置/受控恢复入口。重新观察队列目前是 turn-local，
跨回合/重启的结构化待处理恢复还需接线；持久化历史和 uncertain-effect fence 不等于该队列。
同一文件被外部进程再次修改不会由摘要变成锁。没有运行构建、测试、故障注入、模型调用、
服务/设备或迁移；所有新增行为仍待验证。
