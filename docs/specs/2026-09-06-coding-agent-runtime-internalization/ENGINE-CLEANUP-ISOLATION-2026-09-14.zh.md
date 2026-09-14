# Engine 清理异常隔离（Slice66，未验证）

本轮在 `rf/agent-capability-platform-v2` 本地继续实现，不运行构建、测试、服务、
模型调用或迁移，不 commit/push。历史通过记录不覆盖本轮修改，整体 CAR 未完成。

## 缺口与实现

原共享效果 scope 和生产资源收尾会汇总普通错误，但某个清理 callback 的 panic
可直接跳过后续 owner。SDK 外层捕获只能拒绝终态，不能补回这些清理尝试。

- 新增共享 `guard_effect_settlement`，分别捕获 callback 创建 future 和轮询
  future 时的 unwinding panic，返回保守错误，不将 panic payload 写入返回错误。
  这不是新任务 owner，也不启动、重试或证明任何外部效果。
- `EngineEffectScope` 逐项保护 retained task join 与 settlement witness。
  一旦观察到错误或 panic，在下一次 await 前保存不可清除的失败状态；取消等待
  或后续清理 callback 成功不能抹去失败。后续调用仍尝试清理，但不能返回成功
  证明或开启新回合。正常无失败的关闭/清理仍可成功。
- `EngineKernelSession` 分别保护进程、工具任务、资源任务、Git、MCP 和 hosted
  effects 的收尾。保留原顺序，全部尝试后传播错误；原持有的 cleanup completion
  保留结果，未知状态仍禁止新回合、清理证明及后续资源释放。
- `EngineProcessScope` 逐句柄保护 cancel。发生 panic 后继续清理其他句柄，并在
  下一次 await 前保存 scope 的 panic 状态。之后即使其他句柄已 reaped 或再次
  cancel 返回成功，scope 仍不能宣称 quiescent 或通过 cleanup 从 owner 表移除。
- Coding steering inbox 的关闭也使用同一隔离方法；inbox 关闭失败或 unwind
  不再跳过实际资源清理，但仍禁止发布 `host_cleanup_proven`。

## 约束

只捕获可展开 panic；abort、进程崩溃和跨启动无清理证明仍走原隔离路径。
不保证永久挂起的 callback 后续能执行，保留原超时和 retained owner 合同。
不声称调用返回等于物理停止，不增加强制放行、效果重放、动态 Engine 挂载，
不改变 Agent 选择 Engine / Session 冻结 exact binding 的关系。
未修改全局 panic hook；这里的错误净化不代表全局日志已脱敏。

Coding 构建为 `host2-coding-loop66`，Nomi 为 `host43`。Coding digest 纳入共享
effect scope 源码（原已覆盖 Kernel/Process）；Nomi 原已覆盖 effect/process。
社区仍需源码集成并重新编译打包，策略和执行循环仍由各自 Engine 定义。

## 剩余工作

本切片只补齐清理异常边界，不代表其他缺口已完成：Git 网络凭据 owner/绑定、
未知外部效果的人工核对、跨启动无 witness 的进程恢复、部分 MCP/Provider/生态
生命周期以及实际验证仍待处理。按用户要求，仅对两个小模块运行 rustfmt，未
执行验证命令，不能据此声明可发布或质量已超过 Codex。
