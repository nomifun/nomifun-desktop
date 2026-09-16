# Coding 工具派发与完成证据分离（slice58，未验证）

## 目标及来源

CAR-03 / CAR-06：区分模型提议、引擎门禁暂缓、实际调用端口和最终观察。
参考本地 Codex `codex-rs/core/src/tools/executed_tool_calls.rs` 在执行边界记录
attempted calls，以及 `tool_dispatch_trace.rs` 将派发与返回结果分离的设计。
不移植其 Session、权限系统或遥测存储，也不把日志成功当作任务成功。

## 本次实现

- `ToolDispatchBatch` 只记录当前模型批次的 call IDs，在进入工具端口前记录尝试；
  仅保留有界 ID 集，不存参数、结果或凭据，锁不跨 await。同一批次重复派发拒绝。
- 内部 `coding-instructions:` 调用不属于模型提议；既有输入身份检查禁止模型使用
  该保留前缀，因此读取指令不会把被暂缓的模型调用标记为已尝试。
- 规划/资源混合批次、指令刷新、用户 steering、前序失败等引擎暂缓路径未进入端口，
  不推进工作区 epoch，不计命令/修改，不清除 Patch 的重新观察义务。
  仍计入 failed_tools 并要求重新规划和重新提交完成报告。
- 进入端口后失败仍保守视为可能有部分副作用。存活进程仍令指令缓存失效；
  不通过“被暂缓”推断已有进程结束或工作区静止。
- `CodingCompletionObservation.invocation_attempted` 明确进入端口与否；未尝试
  永远不能成为成功证据，不能附带命令退出观察。旧字段缺失默认 true，不能据旧日志
  推断没有执行。无派发却得到平台工具成功结果属于契约错误。

## 边界与后续

此记录只用于 Coding 的活跃循环规划/完成核算，不是 Kernel 准入、owner 派发或
回收证明，不能用于跨重启解封、不触发任何重试或效果重放。没有改变公共工具端口、
Nomi 的循环、Agent 选择 Engine 的关系或编译期注册规则。

Coding build 更新为 `host2-coding-loop58`，摘要包含新模块；Nomi 保持 `host38`。
仅本地源码及文档修改，未运行构建、测试、服务、模型请求或迁移，未 commit/push。
仍待验证的重点：规划门禁/混合控制批次、前序失败后多调用暂缓、并行读取/内部指令
调用隔离、派发后失败/取消，以及旧观察缺省字段读取。整体 CAR 仍未完成。
