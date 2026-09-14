# Nomi MiniApp / Robot 宿主调用归属与持久观察

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`。
状态：源码实现，未运行构建、测试、数据库迁移、模型请求或端到端验证。未 commit/push。
当前构建标识：Coding `host2-coding-loop28`，Nomi `host13`。

## 本次完成范围

Engine 仍只拥有执行循环和上下文策略。MiniApp Service owner 与 Robot 的设备执行／持久
effect ledger 保留实际权限、调用及生命周期责任，不迁入 Engine，不另建 Session authority。

Nomi 的 `EngineEffectScope` 原先只保留 Kernel 工具任务，现在同时保留 MiniApp 与动态工具
invoker；适配器与 scope 两种安装顺序都受覆盖，重复安装被拒绝。真实 Robot 动态适配器另加
平台凭据包装；这不是给任意动态工具自动授予设备权限或持久效果证明。
MiniApp 与动态工具均按串行 hosted 调用调度，设备或 manifest 声明只读不绕过单 pending 约束。

调用方取消／超时后，已开始的任务继续由 scope 持有。等待者退出关闭当前 turn 的派发入口；
任务内发现结果未知则关闭 Session 入口，即使等待者已离开也生效。每次模型请求前的
mandatory context fence 检查 turn 仍开放，防止迟到结果落盘后同一已取消回合继续推理。
下一回合只能在保留任务和 owner witnesses 结算之后重新开放。任务 panic 无法被当成成功。

## Conversation 归属凭据

新增 migration `098_conversation_hosted_effects.sql`，只编写，未执行。
它归属到既有 accepted turn receipt，不替代 MiniApp Service 或 Robot ledger：

| 状态 | 含义 | 后续处理 |
|---|---|---|
| `pending` | 已在准确的运行中 turn/epoch 下保留调用，结果尚未可靠记录 | 禁止自动重试、恢复和新调用 |
| `returned` | 真实 owner 已返回可确认的结果，包括已确认的设备错误 | 不代表回滚、物理静止、Service 退出或用户任务成功 |
| `rejected` | 已知在远端派发之前被拒绝 | 不宣称发生过远端调用 |

派发前写入 owner/session/operation/turn/epoch、domain、capability、具体 action 和输入 JSON
序列化摘要；不保存原始参数。action 与摘要只用于溯源，不是跨不同 JSON 表达形式的语义
去重键，也不赋予幂等重试权限。每回合最多 512 条；同一 Session 至多一条 pending hosted 调用。
数据库触发器拒绝非准确运行回合、身份修改、终态再改与删除。记录不会随 transcript 清空而消失。

只有真实 app adapter 解释结果：MiniApp 的 `Invalid/NotFound` 当前均发生在 Service 调用前；
Runtime／数据库错误不能据此认定未派发。Robot 的设备拒绝／失败码发生在设备 ledger 已确认
失败之后，记为 `returned` 的错误观察，绝不归类为无派发。离线等仅限源码确定的前置拒绝。
其他错误或凭据写入失败保留 pending；不能凭错误文本、模型判断或 retry_safe 字段消除未知。

结果观察最多保存 8 KiB envelope；小结果保留原值，大结果只保存摘要、长度与有界预览。
摘要采用流式 JSON writer，预览缓冲最多 4096 字节，不额外构造完整结果 JSON。
此约束只限制记录过程的额外内存，不是对实际 owner 原始输出分配／计算成本的新承诺。

## 历史、重试和恢复

Nomi 每轮模型调用补入独立于 transcript 的 bounded hosted history：最近最多 16 条、
记录预算 32 KiB、显式 omitted 数量；观察是非可信数据，不是指令。
查询失败／超时／pending 均阻断模型请求，不能静默省略必要状态。
原输入已派发非 rejected 调用时，自动重试与编辑重发被拒绝；应观察真实状态后发送新指令。

Session 装配、共享 EngineSessionHost、启动 terminal proof 及 Coding 恢复事务均检查 hosted
pending，不会因进程已经退出就把未知外部效果当成完成。Coding 的检查属于共享恢复防线，
不表示 Coding 已经可以调用 MiniApp/Robot。

同时修复既有 schema trigger 校验的顺序依赖：SQLite 查询按名字排序，声明契约原先没有
同序；现在先排序声明再逐项比较，保留精确白名单和 invariant 检查，不放宽数据库约束。

## 接入契约与仍未完成项

`NomiPluginToolError` 新增 `OutcomeUnknown`；模型只收到固定的 `HOSTED_EFFECT_UNPROVEN`
投影，不泄露 DB、HTTP 或 Service 内部诊断。第三方源码中的 exhaustive match 需适配。
`with_effect_scope` 会预留一个 mandatory context slot，并覆盖已安装／后来安装的 hosted
工具 invoker。平台提供凭据 witness；scope 自身不能证明远端状态或跨启动结果。

以下不能算本切片完成：

- 初始化、按需激活、TurnMiddleware／context lease 等非工具生命周期的完整任务与效果归属。
- Coding/社区 Engine 的 MiniApp/Robot 生产工具 surface 与生命周期端口。
- 任意其他 native tool 的自动覆盖，或任意动态适配器的持久效果证明。
- Robot 物理运动停止、MiniApp Service lease 退出；returned 不是这些证明。
- 不确定外部效果的人工引导解隔离，以及跨启动进程树证明、安全 checkpoint 续跑。
- 其他 MCP transport／资源／授权主动交互、VCS push 的远端授权与效果语义。
- 任务需求与证据相关性的独立语义判定，跨平台／真实 provider 执行证据。

因此整体 CAR 仍为 in_progress。Engine 只能通过源码／依赖和组合根注册后重新打包，不允许
发布后挂载或热替换；Agent 工作台选择 Engine，既有 Session 保留 exact build binding。
