# Nomi 的 Kernel 工具任务归属与收敛接口

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`。
状态：源码实施中，未运行构建、测试、评测或 E2E。

## 本次实际接入

- 公共 SDK 增加 `EngineEffectScope` 和 `EngineEffectSettlement`。
  scope 复用 `EngineTaskGroup`，提供逐轮准入、同步关闭、保留任务完成句柄和 owner 收敛检查。
  它不是权限授予接口，也不是持久化副作用收据。
- 当前产品的 Nomi Plugin Tool Session 安装 scope，Kernel 工具 invoker 被包装为受管理任务。
  引擎调用方取消时只丢弃结果接收端，不丢失已接受调用的执行和完成句柄。
  目前包装的是 Kernel Tool invoker；MiniApp、Robot 动态工具、ContextContributor 和
  非工具 Lifecycle invoker 未因此自动获得新的收敛实现。
- Nomi 在 accepted turn 后、与 Stop 相同的生命周期锁下打开准入；Stop 同步关闭，kill 永久关闭。
  正常 Finish/Error、确定性回答、完成裁决、取消清理、异常 unwind、空闲 kill 和 kill_and_wait
  都增加 scope 的收敛检查。正常结束后可开始下一轮，失败的 scope 不重新接受调用。
- 等待任务清理有 10 秒上限；超时不 abort 任务、不删除句柄、不当作成功。
  异常清理仍尝试原生 MCP、进程和 Browser owner 的已有清理，未确定时保留 quarantine。
- 取消后的会话恢复先等受管理 Kernel 调用收敛；不能先恢复临时会话状态，
  再让已经脱离调用方的工具继续执行。Provider 失败后的恢复 helper 也采用该顺序。
- 产品 host 安装真实 MCP owner 的 settlement witness，核对当前用户/Session 是否存在活跃或未知调用。
  witness 不读取模型参数，不把“任务已返回”替代为“远端副作用已确认”。

## 尚未开放的能力

Nomi 对新的逐工具 MCP capability 仍保持拒绝，原生 MCP 路径不变。
任务收敛接线只是必需的一部分，仍需完成：

1. Nomi 的固定 MCP schema/action 投影和 Kernel 精确派发。
2. 新逐工具能力与旧原生 MCP 工具的去重/互斥策略，防止原生目录扩大工具权限。
3. 派发前的持久化副作用记录与 Nomi 重启恢复判定，避免旧恢复日志错误清除未知外部副作用。
4. MiniApp、动态工具和非工具 Lifecycle 的对应 owner 收敛接线。

公共 scope 也不能解决跨重启进程树、远端效果是否发生、需求语义覆盖或多服务器资源绑定。
后续不得仅凭本次任务等待接口就开启上述准入或宣称 Nomi/Coding 全生态对齐。

当前 Nomi build 后缀为 `host4`，相关 invoker、manager、任务模块和 Session 组合源码进入 digest。
Coding build 后缀仍为 `host2-coding-loop15`；未迁移旧精确绑定，未提交或 push。
