# Coding：有回执的执行中追加输入（2026-09-14）

状态：本地源码已接线，未运行验证；整体 engine 目标仍进行中。
分支：`rf/agent-capability-platform-v2`。没有 commit 或 push。
构建后缀：`host2-coding-loop5`。旧 Session 不自动迁移。

## 已实现的接线

1. 通用 runtime 接口增加 `RuntimeSteerDelivery` 和 `steer_with_receipt`，携带永久回执
   operation、目标 wire turn、平台轮次 generation 与文本。默认实现保留已有 Nomi/
   社区 engine 的 steer 行为，Coding 的裸 `steer(text)` 仍不准入。
2. 普通会话与 Agent Execution 的既有幂等追加入口传递已保存的回执。
   IDMM 的非幂等追加入口也先创建回执，再投递；不是另建会话或新执行轮次。
3. Coding host 核对 owner、Session、当前 admission epoch/active operation、回执状态、
   回执中的目标 wire turn/generation 及原文；不把平台 generation 当作 DB epoch。
   附件和 `inject_skills` 元数据不准入 steer，须使用新的正常轮次。
4. `turn_input_scope` 必须紧随 durable TurnStarted 写入，随后才开启收件。
   收件箱与结束检查共用 active-turn 锁，按 receipt ID 去重；每轮最多 16 条，
   每条最多 16 KiB，operation ID 最多 1024 字节。回执写入在平台投递前已完成；
   host 的收件校验只读数据库，内存入队到返回成功之间没有 await。
5. engine 在模型调用前、模型流结束后、串行调用边界、正常结束前检查追加输入。
   模型流期间收到追加输入时，旧模型批次补齐“未执行”的工具结果后重新推理；
   串行调用边界存在新输入时，其余调用延后。已开始的工具/进程不会被强行中断。
6. 追加内容以真实 User 消息进入派生上下文，不进入 system instructions，不能扩大权限。
   当前 root 和后续追加输入都保留在压缩保留区，清除旧 provider continuation parent，
   并要求重新考虑计划后才能继续副作用。模型步数预算不因 steer 自动扩张。
7. 结束前的 `take(close_if_empty=true)` 原子关闭空收件箱；若还有输入则继续循环。
   取消/失败时关闭收件入口，未到模型边界的输入写为 `steering_deferred`。
   即使输入日志失败，也仍尝试原有进程与工具任务清理。
8. 串行 `ToolStarted` 改为在实际调用准入前写入，而不是预先覆盖整个待执行批次。
   因错误、路径指令变化或 steer 延后的调用不会伪装为已经准入副作用。

## 投递与恢复语义

- 成功回执表示已排队，**不表示模型已看到、遵守或完成了追加指令**。
- `steering_inputs` 是交付到 engine 边界的记录；紧接着取消、压缩失败或预算耗尽，
  仍可能没有后续模型调用。没有将其升级为“模型执行证明”。
- 正常历史重放保留输入及未投递说明，并继续维持工具调用/结果配对。
- 异常退出可能丢失内存队列。历史读取会结合已记录的 wire scope 与永久 steer 回执，
  将缺少边界/延后记录的请求投影为“投递状态未知”的历史数据；不会在新轮次自动入队。
  该补充观察每旧轮次最多 64 条，超限明确提示；仍受整体 16 MiB 历史预算约束。
- 重启审计拒绝重复输入 scope、重复/超量输入身份、清理证明后的追加输入记录。
  进程树或 Plugin owner 清理未知仍保持隔离；本切片没有放宽副作用重放规则。
- 输入在安全边界处理，不是即时中断：用户要求立即停止仍应使用平台取消入口。
  工具准入已经开始的边界可能继续完成其已拥有的调用。

## Codex 借鉴及架构边界

只读参考 `multi/codex/codex-rs/core/src/session/input_queue.rs` 的 turn-local inbox、
持锁入队/取出和关闭投递边界。未移植其 Session owner、mailbox 或 provider 私有协议。
NomiFun 仍由平台持有回执、权限、会话生命周期与副作用清理；engine 决定何时重新推理、
如何处理旧工具批次及如何保留追加需求。没有新增聊天页 engine 切换或打包后挂载入口。

## 仍未完成

MCP 的生产 exact-lock/schema/owner 接线、MiniApps 和 Plugin 生命周期、Git push 的
凭据与持久外部回执、跨启动进程树证明、安全 checkpoint 续跑、通用 host SDK 和成功运行的
社区 engine 参考实现，以及分页 Skill 资源和动态/递归路径指令范围等仍在剩余清单中。

按用户要求未运行构建、测试、模型评测、桌面 E2E 或全量检查。只做源码实现、阅读与
小模块定向格式化；当前没有本切片的运行正确性或性能证据，不能标记整体目标完成。
