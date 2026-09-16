# Nomi MCP 资源入口迁移（源码实现，未验证）

本切片继续在本地 `rf/agent-capability-platform-v2` 工作，不 commit/push。
当前 Coding `host2-coding-loop45`、Nomi `host28`；没有执行构建、测试、外部服务调用
或数据库迁移。整体 Engine 目标仍在实施中。

## 两种策略、同一个资源 owner

绑定到 canonical Agent 的 Nomi Session 现在通过共享平台 MCP owner 列出和读取资源，
不再由 Nomi 原生 MCP manager 执行这些资源请求。Coding 保持其独立模型循环和控制
入口；Nomi 保持自身 ToolSearch/deferred 发现机制和现有 `mcp_resource_list` /
`mcp_resource_read` 工具名称。没有把两种 Engine 合并，也没有新增可热挂载引擎。

Nomi 模型参数不接受服务器、凭据、资源 binding、capability 或 operation ID。
工具执行上下文提供按源回合/call 派生的操作标识；平台 adapter 固定 principal、Session、
Snapshot、资源 policy 与当前 Registry。这里不伪造 Coding 的 ChatCausality，也不为
ResourceProvider 虚构 canonical Tool action。

共享 owner 仍要求唯一服务器 `connect + read` binding，并在使用前核对启用状态与
配置引用。URI 目录/内容、文本格式、分页与内容摘要遵循前一切片的共享规则。
资源结果及持久化观察始终是数据，不是新的指令或任务完成证明。

## 准入、按需发现和启动行为

应用 Session materialize 后安装资源 adapter，共享同一个 Kernel capability state。
按需工具必须先经过 ToolSearch；工具的激活身份包含 Snapshot 和资源绑定摘要，不把
相同显示名称视作相同权限。首次真实调用按当前 generation 激活已编译的资源 bundle，
拒绝在资源激活过程中悄悄激活其他类型的生命周期 owner。

Agent 已选择但 Session 执行上限不允许该能力时，不安装端口或模型入口。
共享 Session 准入允许资源和冻结 MCP 工具共用同一个服务器；资源消费者仍不能选多个
服务器。`mcp.tool_proxy` 原生全工具代理不能与该资源路径混用，需选择冻结的逐工具
能力。冲突在 Agent/Session 准入时明确拒绝，不降级绕过平台 owner。

canonical 资源 Session 的工厂移除原生 MCP 发现/连接/proxy/resource 路由，清空相应
服务器配置；Nomi bootstrap 不再从用户配置文件重新连接资源服务器。只有独立授权、
由宿主注入的 delegation Gateway 可以保留，它不是资源服务器授权的扩展。
未绑定 canonical Agent 的旧 Nomi 会话仍保留原生兼容路径，此切片没有自动迁移它们。

## 任务、永久凭据与恢复

真正读取通过 Nomi 的共享 `EngineEffectScope` 同步注册任务。丢弃/取消等待者会关闭
该回合的后续效果准入，但不会丢弃平台任务；现有模型边界、回合/Session 清理与永久
MCP source-replay 检查继续生效。工具和宿主都按串行方式处理资源调用。

平台 MCP owner 在 initialize/OAuth/stdio 启动之前保留永久回合凭据，明确返回并完成
清理后才结算。未知结果和清理失败保持隔离，不能因为操作叫 read 就自动重试。
资源操作每回合最多 64 次的限制下沉到永久凭据准入，对两个 Engine 一致生效；总 MCP
事务仍有 512 次上限。没有修改或执行数据库迁移。

如果资源凭据准入返回错误，代码没有进入协议 owner，此时释放内存中的活动占位；
可能已经落盘的 pending 凭据仍保留并阻止后续操作。这样不会因明确的本地预算拒绝
凭空制造远端活动状态，也不会清除不确定的持久化结果。

Nomi 的永久效果观察、源输入 retry/edit 限制和 boot pending 检查复用现有平台机制。
它们不构成自动续跑旧任务、物理停止或远端回滚的证明。没有新增历史资源重读逻辑。

## 未完成范围

此处接通的是同一 MCP 文本资源能力在两个官方 Engine 中的执行路径，不是完整 MCP
协议或整个 CAR 阶段验收。URI templates、binary/订阅、服务器主动 sampling/roots/
elicitation、未知效果人工处置、长期连接以及其他生态生命周期仍待实现。
所有本切片修改尚未编译或运行验证，不能据此宣称端到端可用性已得到证明。
