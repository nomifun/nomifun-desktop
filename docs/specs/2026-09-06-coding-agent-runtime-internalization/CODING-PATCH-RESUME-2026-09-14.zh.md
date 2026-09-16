# Patch 恢复状态跨回合接线（2026-09-14）

状态：源码实现，未验证；分支 `rf/agent-capability-platform-v2`，未 commit/push。
Coding `host2-coding-loop34`；Nomi 保持 `host18`。任务 CAR-05 / CAR-06。

## 本轮完成的接线

上一切片只在内存中保存失败 Patch 的待观察目标。普通 Coding 历史仅有最多 32 回合的窗口，
还需要聊天消息和模型摘要，因此不能用作恢复约束的唯一来源。

新增 `CodingPatchRecoveryState` v1 与 `PatchRecoveryUpdated` 执行事件。它属于 Coding 策略
状态，不是第二个 Session owner、文件事务、执行 checkpoint 或新权限。使用既有永久
`conversation_runtime_events` 和 delivery receipts，不添加或运行数据库迁移。

1. 新回合在真实工具执行前加载前一永久状态，校验当前已接纳回合、来源 user/Session、
   来源 completed receipt、TurnStarted 的精确 family/build/digest/Snapshot。
2. Engine 把加载的状态重新写入当前执行日志，独立于聊天消息、hidden 标志、历史窗口和摘要。
   初始化失败不会写入一个空状态覆盖旧约束。
3. Patch 经工具准入后、交给 invoker 前，先持久化目标。目标预算无法承载则在调用前失败，
   不先执行再发现无法记录。写前状态失败也不执行工具。
4. 正常成功返回后清除本次写前状态并持久化；失败、等待取消、崩溃或清除记录失败，留下
   保守的待观察状态。已准入但实际未派发的取消窗口也可能要求重读，不据此宣称发生修改。
5. 失败后的读取全部完成才持久化空状态。部分读取只改善当前回合的待办，若回合中断，
   下一回合重新读取整组目标，不把旧的部分读取当当前证据。
6. permanent state 后出现的 fs.patch host dispatch 若没有相应待观察状态，拒绝继续；
   有 dispatch 却完全没有状态时同样拒绝，不从成功 receipt 或摘要推断恢复已完成。

加载器在同一只读事务内先检查 blob 长度，再读取最多一个状态和一个绑定根；状态支持
最多 64 个规范路径、16 KiB 原始路径总量。后续派发检查从状态 event id 起查询，避免每轮
重新读取完整执行历史。没有把用户路径或文件正文当作系统指令。

启动恢复审计识别并校验新事件。它仍只决定是否可以关闭已中断回合，绝不把进程清理或
宿主结束当文件回滚。新回合仍经过既有 owner 准入及未知效果隔离；读取不能清除 Wave2
未结算 journal，不能自动重试旧 Patch 或恢复旧命令。

## 边界与后续

- 这是同一 exact Session 的恢复义务传递，非全局工作区锁。独立 Session / Fork 没有通过
  复制聊天自动继承另一 Session 的执行义务；跨 Session 工作区协调需单独设计。
- 自定义 Coding 宿主需持久化新事件并通过 `with_patch_recovery` 提供经自身 owner 审核的
  状态；默认空状态用于无历史的嵌入，不是“历史一定安全”的证明。
- 源码社区 Engine 不必采用 Coding 状态结构或策略；仍可以用公共事件/工具宿主编写自己的
  循环、规划与上下文。编译期随应用打包注册政策及 Agent 工作台 immutable 绑定不变。
- 手工解除隔离、未知效果仲裁、跨启动进程树证明、完整 checkpoint 续跑、多文件事务及
  外部原生修改/重命名隔离仍未完成。没有运行构建、测试、故障注入、迁移、服务或模型调用。

整体 CAR / Engine 完成状态仍为进行中，不以此文替代真实验证证据。
