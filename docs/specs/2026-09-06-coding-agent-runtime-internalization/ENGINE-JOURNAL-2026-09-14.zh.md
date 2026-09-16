# 公共回合日志、历史读取与 Coding 持久结算

分支 `rf/agent-capability-platform-v2`，本地实现，未提交、未 push。
按用户要求未运行构建、测试、评测或端到端验证。只做源码阅读和局部格式化。
Coding build 后缀为 `host2-coding-loop9`；整体 CAR/公共 SDK 仍未完成。

后续 `loop10` 将工具结算下沉到公共宿主，并增加模型预算事实与有界旧消息回退；
详见 [ENGINE-EFFECTS-2026-09-14.zh.md](ENGINE-EFFECTS-2026-09-14.zh.md)。本文保留本切片历史。

## 已接入生产的公共日志

`EngineSessionHost::open_journal` 从本宿主读取的 `EngineTurnReceipt` 建立
`EngineTurnJournal`。同一 receipt 的存活 writer 复用同一游标；弱引用缓存不承担 Session
协调或重启恢复职责，失去 writer 后也不会自动续写数据库中的旧前缀。

日志仍存储在 `conversation_runtime_events`，没有新建 Session DB，也没有强迫第三方
engine 使用 Coding 事件 codec。平台负责序号、预算和模型请求领取，engine/宿主适配器
负责语义事件。公共 API 由 `nomifun_app` facade 导出。

- 写入在宿主持有的任务中完成；调用方丢弃等待不丢弃 SQL 写入和游标更新。
- 最多 64 个排队写入，合计不超过 8 MiB 待写载荷；普通阶段 3200 条/4 MiB，
  结算/清理预留区最多 4095 条/8 MiB，保留第 4096 条给异常恢复记录。
- SQL 返回不确定或序号冲突后拒绝后续写入，不能猜测失败后复用同一序号。
- 普通事件在 SQL 中核对当前 running Session、owner、root、epoch、accepted receipt。
- `Settlement` 只用于已准入副作用的结果；取消/收尾后仍可向原 receipt 写结算，不授予新操作。
- `Cleanup` 关闭后续普通写入，`Terminal` 只能随后写入，终态后拒绝追加。
  这些分类本身不是清理证明，只有实际 effect owner 能证明任务/进程已经退出。
- 公共 `ChatCausalityGate` 使用同一回合身份与 route，对已记录 model operation 做一次性
  claim。Broker 内部重试/failover 复用这次 claim，不另造授权。

Coding 的普通记录、steering 原子准入、能力激活和模型领取已消费公共 writer。
steering 仍在原 active-turn 锁内检查输入与写入 ToolStarted；没有削弱追加输入边界。
能力激活在等待持久化前置失败标记；等待被取消后，即使日志已提交，也保持隔离，
由下次精确恢复重建状态，不能假装内存已经激活。

`EngineSessionHost::open_model_port` 已提供真实生产 Broker 的装配，返回同一回合的 journal
与模型端口。调用前必须先持久化 model operation，模型入口由公共 journal gate 领取。
provider、凭据、HTTP transport 和 retry/failover 仍在原 Broker 内。Coding 的模型工厂也
委托这一公共宿主的同一装配方法，并保留自己的能力激活失败隔离，不再另建 Broker 组合。

## 公共原始历史窗口

`EngineSessionHost::read_history` 返回 `EngineHistoryWindow`，包含原消息、receipt 和
事件序列，以及被截去旧回合的标记。读取使用同一个数据库 read snapshot，先查询记录数
与字节大小，再加载载荷，避免先把超大日志全部读入内存后才发现超限。
最多 32 个完整回合，每回合最多 4096 条/8 MiB 事件，总窗口不超过 16 MiB。
超限时只裁掉旧的完整回合；最新回合本身超限则明确拒绝，不截断工具调用链。

返回的是数据，不是“已安全结束”的证明。Coding 自己继续核对 exact build/Snapshot、
解释事件、判断终态、投影工具历史和追加输入；社区 engine 可以使用自己的 codec 和
上下文策略。Coding 的结构化历史读取已经切到这个公共入口，legacy/fork 文本投影仍保留。

## 工具结果先持久化，再返回

Coding 的 JoinedTools 现在在同一个宿主托管任务里完成工具调用和
`host_tool_settled` 写入，随后才把结果交给 engine。取消仅停止等待，不跳过结算。
`join` 包含结算写入，持久化失败保留失败标记，不能发布 host_cleanup_proven。

原有指令结果摘要化与输出大小限制保留；不将完整 AGENTS/Skill 指令原文新增写入日志。
已持久化但尚未被 ToolCompleted 消费的结果另用有界调用集合跟踪，能力激活仍拒绝
跨过未消费结果。旧的取消/工具所有权测试源码调整为读取真实内存 SQLite 日志，但未执行。

## 尚未完成

公共日志和原始历史不是完整 SDK：通用工具原子准入/结算 facade、资源 owner 装配、
统一模型预算事实接口，以及不依赖 Coding 的独立 engine 执行示例仍待完成。
MCP 当前产品目录/凭据/执行 owner、MiniApps、非 function Plugin 生命周期、push
持久回执、进程跨启动证明、安全续跑、动态指令范围等原有缺口仍保留。
完整目标尚无验证证据；用户允许后还需做组件、消费者继承、升级和多平台验收。
