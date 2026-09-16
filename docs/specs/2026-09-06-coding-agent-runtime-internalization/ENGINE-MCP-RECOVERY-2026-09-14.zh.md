# MCP owner 持久凭据与 Coding 中断恢复（未验证）

> 历史切片：下面的 Nomi 禁止准入及不保存观察描述对应 migration 096 / host5。
> 当前状态由 [ENGINE-NOMI-MCP-2026-09-14.zh.md](ENGINE-NOMI-MCP-2026-09-14.zh.md)
> 更新：migration 097 增加 bounded observation，Nomi 效果感知保护后开放单服务器新路径。

本切片仅在 `rf/agent-capability-platform-v2` 本地实施，没有 commit/push，
没有执行构建、测试、评测、迁移演练或 E2E。局部 rustfmt 不代表编译通过。
Coding build 更新为 `host2-coding-loop16`，Nomi build 为 `host5`。

## 已写入的执行链

- migration 096 新增 `conversation_mcp_effects`。它从属于现有 Conversation turn receipt，
  不是另一套 Session owner。每条记录冻结 user、Session、工具 operation、turn operation、
  admission epoch 和 capability；不存参数、凭据、连接地址或远端响应。
- 当前产品 MCP owner 在 OAuth/initialize/远端调用之前，必须等待 `pending` 写入成功。
  INSERT 在一条语句内捕获并核对现有 running/accepted turn；唯一约束阻断同 Session
  的未收敛事务和相同 operation 的重复执行，每个 turn 最多 512 条。
- 只有真实 owner 返回成功结果且协议会话清理成功，才将该条记录转成 `settled`。
  超时、取消、owner 错误、写入不明或 settlement 写入失败均不伪造成功。
  内存 guard 与持久记录同时保留未知状态；异步 SQL waiter 丢弃不能授权提前远端调用。
- 凭据身份不可改写，仅允许 pending → settled，不能删除。数据库表、ID/逻辑关联、
  索引和触发器同步加入现有 schema contract 注册表。
- EngineKernelSession 和 Nomi 的 MCP settlement witness 现在同时查询内存与持久记录。
  启动 terminal-proof provider 在本地进程证明和注册 Engine recovery 之前，
  检查该 Session 所有 generation 的 pending；读取失败同样不能证明恢复安全。

## Coding 恢复语义

当前精确 build 的恢复事务核对：ToolStarted 的 call/capability/action、host_tool_dispatch
的 call/operation/capability/action，以及 owner receipt 的 user/Session/turn/epoch/capability。
不接受孤立 owner receipt、重复派发或 pending。

本 build 的 owner 保证先持久化再执行，因此只有工具意图、没有 owner receipt 的调用
可以判定尚未进入远端事务；并不因此重放它。已 settled 的事务也可能产生了外部变更，
只能保留历史并将中断回合闭合为失败，不把它当作任务成功或撤销效果。
旧 build 仍按 exact build/digest 的恢复规则处理，不能借用本 build 的无凭据推论。

## Nomi 逐工具通道的准备与准入

- Nomi Kernel materializer 增加独立的 frozen MCP schema 分支，使用现有精确 action
  identity、Session active-set/on-demand 激活、Kernel 权限及 retained tool task。
  应用 provider 从当前 Registry 的冻结 descriptor 生成 schema，不在模型循环发现工具。
- 采用每 Session 的独立 MCP approval map，不扩展 PlatformBuiltin ID 集，也不覆写
  Robot 使用的 host_dynamic invoker。MiniApp/非工具 Lifecycle 的归属范围没有被扩大。
- 工厂识别 frozen MCP actions；与 native MCP connect/proxy/resource/oauth 或 deferred
  MCP 工具混用会报错，逐工具选择不能触发服务器全工具发现。
- **产品准入仍关闭**，并在 Session provider 加入同样闸门，覆盖历史 Session 的构建路径。
  剩余阻碍不再是工具投影缺失，而是 Nomi 私有 transcript rollback、自动 failover/retry、
  session restore 的副作用语义：远端清理成功不等于远端写入可以撤销或重新执行。

## 不包含 / 后续

1. Nomi effect-aware 回退与重试、恢复后提示/历史观察；完成之前不能打开上述闸门。
2. Native Nomi MCP 旧通道未改为此 owner，不能用本报告宣称旧通道拥有同样的持久效果证明。
3. 多 MCP 服务器、stdio、旧 SSE、resource/server-initiated 生命周期仍未完成。
4. 凭据不提供远端业务事务回滚、人工解除隔离、成功任务验收或自动 checkpoint 续跑。
5. MiniApp/Robot/非工具 Lifecycle、未知进程树恢复、消费者继承及升级兼容仍需后续工作。
6. 本切片全部为未执行验证的源码实现；整体 CAR 仍为 implementation-in-progress。
