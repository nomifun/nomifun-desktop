# Coding 的 Fork/导入历史前缀连续性（slice89）

2026-09-14，rf/agent-capability-platform-v2。本轮只有源码与小文件 rustfmt，
没有测试、构建、服务、模型调用、迁移、commit 或 push。

## 发现的问题

Fork 复制的消息没有子 Session 的 Engine turn receipt。首轮走兼容消息投影能看到
它们；生成第一轮原生 Coding 事件后，旧实现只重建有 receipt 的 turn，遗漏复制
消息。Slice88 移除重复 system_prompt 历史后，这个缺口必须从历史组合本身修复。

## 实现

- 平台新增 `read_message_history_before_turn(receipt, operation, limit, byte_limit)`。
  上界必须是同 owner/Session 的历史 turn receipt，不能传任意 message row id。
  消息/原生事件读取共用根消息与历史游标边界解析，同时核对当前 active generation
  及 slice88 的持久清理起点。单次读事务、追加前尺寸检查和条数限制保留。
- Coding 从 accepted receipt 统一取得 Session、Engine 和 Snapshot 事实，不再
  由调用者并列传入三份可漂移身份。
- 原生 turn 窗口完整时，在最早原生 turn 之前读取有界的兼容消息前缀，先投影
  前缀，再按时间顺序应用各 turn。前缀与原生记录范围不重叠。
- 必须在事件重建**之前**加入前缀：后面的 ContextCompacted 可以替换它，不能
  在重建结束后重新追加已经被摘要替代的旧内容。
- 原生窗口因 turn/byte/steering 限额截断时不跨越缺失 turn 拼接更早前缀。
  前缀预算最多 8MiB，且不得超过原生历史 16MiB 收集预算的剩余部分；这是宿主
  候选数据预算，不替代 Coding 最终的模型上下文预算或峰值内存证明。
- 纯兼容历史与前缀共用数据投影函数，按序列化后的 ChatMessage 大小计费。
  tool UI 记录仅成为不可信文本，不制造工具调用身份、执行或恢复凭据。
  可选前缀首条过大时返回截断窗口，不阻塞可用的新 turn；不跳过它读取更早消息。

## 借鉴与边界

参考了 Codex 固定基线 `6af345407d9c2a568da9d01b6c4b81a9e61495c0` 的
`codex-rs/core/src/context_manager/history.rs`：上下文按时间排列，压缩替换模型窗口，
保留事实不等于模型历史。本片是 NomiFun 双历史来源组合修复，不声称复制了 Codex
的 Fork 实现或其质量。最初查看了参考工作树，随后用固定提交的源码确认基线。

没有改变 Fork exact Engine 继承、工具 owner、Session owner、恢复义务或清理
权限；不会绕过 context floor。较旧的无原生事件 turn 仍使用已有兼容回退策略，
不把整个 legacy transcript 补造成原生工具历史。超过有界窗口的完整历史恢复、
真实模型质量及连续多轮/压缩/Fork/清理组合行为仍没有运行证据。

官方源码标识：Coding host2-coding-loop89 / Nomi host59。相关共享历史与 Coding
历史源码已有 digest 输入；整体 CAR 保持 in_progress。
