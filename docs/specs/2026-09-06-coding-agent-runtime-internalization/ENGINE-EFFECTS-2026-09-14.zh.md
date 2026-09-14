# 公共模型事实、工具结算与历史边界（loop10，未验证）

所有修改位于本地 `rf/agent-capability-platform-v2`，没有 commit/push。
按用户要求未运行构建、测试、评测或端到端验证；迁移了已有工具生命周期测试源码，
仅对小范围 Rust 文件做格式化。以下均为源码实现，不是验收结果。

后续 `loop11` 已增加生产 Kernel/资源装配和公共进程 owner，详见
[ENGINE-RESOURCES-2026-09-14.zh.md](ENGINE-RESOURCES-2026-09-14.zh.md)。本文保留本切片记录。

## 模型事实与 engine 策略分离

新增公共 `EngineSessionHost::read_model_facts`，只接受本宿主解析出的 Session。
从冻结 revision 中读取 exact route，并在一个数据库读事务内读取主模型和所有备用模型
的窗口/输出上限。没有上限的数据保留为 `None`；不暴露凭据、连接配置或自选路由入口。

`EngineRouteModelFacts::envelope_with_unknown_policy` 要求调用者明确提供未知值策略，
逐候选应用后求交集，避免已知的大主模型掩盖未知或更小的备用模型。
这个观察不是模型容量预留，也不是 tokenizer；Broker 每次调用仍检查真实路由。

Coding 每回合重新读取事实。32K/4096 未知值假定、最多 4096 输出及 context/8 的
输出预留继续属于 Coding 策略，没有成为所有社区 engine 的默认上下文算法。

## 可复用工具宿主

新增根 facade 导出的 `EngineToolHost`，实现 `nomifun_engine_core::EngineToolInvoker`。
生产 Coding 直接给它装配 `KernelEngineToolInvoker::for_session`，只保留薄适配器与
AGENTS 指令结果摘要策略。它不自行扫描插件，也不编译或授权任意资源。

生命周期顺序为：

1. 校验回合、Session、owner、Snapshot 与 journal 的身份一致，限制调用参数/身份长度。
2. 在与关闭/重新绑定共用的锁中保留调用身份和 owned task；每回合最多 512 次派发，
   同一个 operation 或 call ID 不能重复使用，失败也不释放身份让旧副作用重放。
3. 通过公平的读写门控：声明为可并行的只读工具共享，其他调用独占。
   Kernel 在执行前继续复核真实 mapping/effect 分类，模型不能自声明权限。
4. 在现有 Conversation 日志中写入 `host_tool_dispatch`，SQL 校验 accepted root/epoch。
   它只是派发意图，不是 Kernel 授权、成功副作用或进程退出证明。
5. 执行真实 Kernel 工具并校验返回 call ID/结果结构；在持有同一执行门控期间，
   持久化 `host_tool_settled`，然后才返回结果。
6. 取消调用方等待不会丢弃已经派发的副作用。结算失败、任务 panic 等情况拒绝清理证明。

默认持久观察移除了历史媒体二进制正文，并保留有界文本、截断标记和摘要；
Coding 的指令读取额外仅记录摘要。实时工具返回不被这个历史投影替代。
单条结算限制 64 KiB；日志预算耗尽仍明确失败并保留隔离，尚不提供崩溃后自动续跑。

`mark_observed` 必须发生在 engine 观察事件持久化之后。Coding 能力激活仍检查
`has_unobserved` 和真实进程 owner 的 quiescence，不能用“工具已落盘”代替“已被循环观察”。
清理先关闭派发入口；进程清理报错也继续 join 已持有的工具任务，错误仍向上传递。
只有关闭且已 join 的回合能丢弃未读观察；进程、workspace/Kernel lease 的退出证明
仍分别属于真实资源 owner。

本轮参照本地 Codex `codex-rs/core/src/tools/parallel.rs` 对 step/工具上下文绑定与
共享/独占门控的组织方式。没有照搬其任务 abort 行为：Nomifun 对已准入 Kernel
副作用保留 owned task 和持久结算，取消等待不能成为回滚或清理证明。

## 旧历史与异常恢复的读取边界

公共 `read_message_history` 为旧 Session/Fork 提供数据性质的原始消息窗口。
在同一读事务中先读 ID/字节长度，预算允许后才读取正文，按 owner/Session/root 过滤；
不读取隐藏消息，最多 4096 条、8 MiB。最新一条即超预算时明确失败，旧尾部可整体省略。
角色解释与上下文策略仍由 engine 处理。

Coding 先尝试结构化事件重放，确实需要兼容回退时才读旧消息；不再先加载所有旧正文
再在内存中截断。重启恢复同样在加载 event_json 前检查条数与字节总量。
恢复审计识别新的派发意图，但进程清理未知仍隔离，不自动重放工具。

## 架构与剩余工作

仅预置 Nomi 与 Coding。引擎在 Agent 工作台选择，随 immutable revision 保存，
Session 固定 exact build。社区仍须二次开发后重新打包；没有运行中安装或挂载入口。

`EngineToolHost::new` 是受信任应用装配接口，不等于完整资源宿主 SDK。
下一步仍需从生产 owner 统一装配公共 Kernel/工作区/进程资源端口，以及真正独立、
使用自身规划与上下文算法的社区参考 engine。不能拿 Coding 包装器或空 cleanup 充数。
MCP 当前产品目录接线、MiniApps/非 function Plugin 生命周期、Git push 凭据及外部
副作用回执、跨启动进程证明/人工解隔离、安全 checkpoint 续跑等仍未完成。

新 Coding build 后缀为 `host2-coding-loop10`，摘要包括新增公共源码。
旧 exact build 不自动迁移。整体 CAR 仍为 implementation-in-progress。
