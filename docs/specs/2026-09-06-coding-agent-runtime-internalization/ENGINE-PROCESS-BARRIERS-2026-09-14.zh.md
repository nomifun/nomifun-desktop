# 进程 owner 中途回收凭据（slice56，未验证）

工作分支 `rf/agent-capability-platform-v2`。没有构建、测试、服务、模型请求、迁移、
commit 或 push；仅阅读、编辑源码/文档及定向格式化。Coding `host2-coding-loop56`，
Nomi `host38`。本轮未增加数据库表或迁移。

## 解决的问题

此前重启恢复只要看到 process.exec ToolStarted/host_tool_dispatch 就要求整轮
host_cleanup_proven。即使命令已经结束、进程树回收完成，只要在整轮清理凭据写入前
崩溃，也无法根据已有事实关闭中断历史。反过来，不能把工具的普通结果或空进程表
直接升级为进程清理证明。

本地 Codex `core/src/unified_exec/process_manager.rs` 的 refresh_process_state
从持有的 process entry 判断退出，prepare_process_handles 核对对象身份后提供句柄。
本轮沿用“以 owner/持有句柄的观察为准”的原则；下面的永久凭据和跨重启核对是
NomiFun 平台自己的实现，不声称 Codex 具有同样的持久恢复协议。

## 实现

- EngineKernelSession 将同一 accepted receipt 的 EngineTurnJournal 交给进程 scope。
  Kernel/平台仍掌握 scope 和进程，Engine 不生成清理凭据，也不直接获得进程句柄。
- 每次真正进入进程 owner，在任何启动/交互前记录 host_process_dispatch，携带
  确切 Kernel operation ID 与 scope 内连续 ordinal；同一 scope 的调用和观察
  在一个锁内串行，最多 512 个 operation，最多保留 64 个启动句柄。
- 只有全部持有句柄的终态均有 cleanup.reaped 且不存在未登记启动时，平台写入
  host_process_quiescent。凭据记录该 operation/ordinal 及保留句柄数；不保存命令、
  环境变量、stdin、输出或凭据。每次后续 owner 派发都使旧 barrier 失效。
- 写入失败关闭进程 admission，不得忽略后继续发起进程操作。取消后只允许既有
  操作按原 journal Settlement 规则落下事实，不重新开放进度。
- ManagedEngineProcessOwner 新增 start_with_evidence，保留底层结构化失败事实：
  预启动校验/取消、明确 SpawnFailed/容量拒绝可以证明没有遗留存活进程；
  StartLost 仅以 cleanup.reaped 为依据，其他未知错误不从字符串猜测。
  原 start API 保留兼容，委托新方法后返回原错误类型。
- scope 在启动前标记未登记窗口，只有取得并登记句柄或底层明确无存活进程证明
  才清除。未知启动结果会阻止后续准入与清理成功声明，不能因为 map 为空而解封。
  明确失败依然是失败，清理凭据不将其改为成功或撤销已发生的文件/外部效果。

## 重启核对与历史

CodingRestartRecovery 仍要求启动时冻结的旧进程代次、相同 Engine build/digest、
accepted receipt、Snapshot/turn 与连续有界日志，并独立核对原有远端效果凭据。
新增 ProcessRecoveryAudit 核对 ToolStarted → host_tool_dispatch → owner dispatch
的调用/operation 关联，要求 ordinal 连续、operation 唯一；barrier 必须对应最新
owner operation，不能重复、倒退或把保留句柄数减少。

没有 owner dispatch 的引擎/host 意图，在此精确编译实现中没有进入进程 owner。
所有 owner 派发被最新有效 barrier 覆盖，或已存在原有整轮 joined-cleanup 凭据时，
可以关闭被中断的历史。若最后派发之后没有有效回收证明，仍隔离。恢复只追加中断
终态，不重新执行命令、恢复旧句柄或将任务标为完成。

普通 Coding 历史读取跳过平台进程凭据，不把它们作为新模型指令或工具结果。
同时修复 slice54 的实际缓冲缺口：CodingEventBuffer 原先删除全部 ToolCallDelta，
会丢失截断调用的提议身份；现在每模型步保留首次 ID/name（arguments_delta 为空），
最多 64 个。真正参数片段仍不持久化，replay 能核对截断作废列表，不伪造执行结果。

## 尚未完成

这不是“所有崩溃都可自动恢复”：应用死于启动/运行中且没有后续回收凭据，仍需要
独立的 OS 身份/进程树证明或人工处理。没有新增跨重启 PID 猜测、kill、自动重试、
副作用回滚或未知远端效果解封；整个平台多 Engine 注册方式与权限边界不变。
社区 Engine 可消费平台日志，但要提供自己的精确恢复适配，不能直接沿用 Coding
的工具名/事件 codec 假设。Nomi 的独立恢复策略没有被替换。

未运行验证，新增代码及原有进程 runtime 的跨平台清理语义仍需后续执行证据；
不得据此宣称 CAR 或所有恢复场景已完成。
