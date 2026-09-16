# 公共生产资源装配与进程所有权（loop11，未验证）

仅修改本地 `rf/agent-capability-platform-v2`；没有提交或 push。
遵照用户要求，未运行构建、测试、评测或端到端验证。迁移了既有进程测试源码，
做了局部格式化；这些不是验收结果。整体 CAR 仍在实施中。

## 生产装配入口

新增根 facade 导出的 `EngineKernelSession`，通过
`EngineSessionHost::open_kernel_session(&AdmittedEngineSession)` 获取。
只接受同一宿主从真实 Conversation owner 解析的事实，不接受 engine 自造的资源授权。

宿主使用已有 Kernel、编译环境和 Wave2 owner：

- 工作区来自 owner 已重定位的 Conversation response，而不是 engine 传入的路径；
- 文件工具要求唯一的 workspace 类型授权，进程工具要求唯一的 process_session execute 授权；
- 使用现有 Session 资源绑定规则和 Snapshot 编译器，保留 principal/revision/Snapshot 一致性；
- 同一存活 Session 复用资源上下文和 active set，身份或工作区不同则拒绝；
- 弱引用句柄表只防止重复装配，不建立第二个 Session 数据库或替代运行时 registry。

Coding 工厂已改用此入口，不再持有自己的一套 Kernel/进程 owner 装配逻辑。
能力激活仍引用公共资源上下文中的同一个 `SessionCapabilityState`，仍由 Coding
负责自己的激活事件、恢复和安全边界。

## 工具端口

`compile_tool_plan` 从显式 exposures 生成模型工具映射，最多 256 项；可预编译已选的
on-demand 能力，但不会因此激活它们。`install_tools` 再对整份映射做 canonical 复核，
通过实际 `KernelEngineToolInvoker::for_session` 装配公共 `EngineToolHost`。

工具面只能在开回合前安装一次。宿主保留工具 host 强引用，driver 丢弃句柄不会让资源
清理忘记已托管的任务。此生产入口不返回任意 Kernel invocation 绕过通道。
每次实际执行仍由 Kernel 独立检查 Snapshot、活跃代次、capability/action/resource。

社区 driver 的接线顺序是：

1. 在编译期注册的 factory 中取得已准入 Session，打开 Kernel 资源上下文；
2. 明确编译工具面并安装，使用公共模型事实、历史和 Broker 端口；
3. 每回合读取真实 accepted receipt 和 journal，调用 `open_turn`；
4. 自行实现规划、上下文、模型/工具循环及事件 codec；
5. 调用 `cleanup_turn(root_message_id)`，成功后才持久化清理/终态并发布完成；
6. 最终调用 `cleanup_session` 释放实际资源。

这些是生产接口约定，不是“独立 engine 已成功执行”的证明；完整参考实现仍待完成。

## 清理不能被取消等待或跨回合调用破坏

`open_turn` 校验 receipt 来源、Session、owner、exact engine binding、Snapshot 与 journal。
回合在尝试创建进程 scope 前即被保留，部分准备失败也能进入真实清理路径。
新回合必须等待旧回合清理完成；不能通过重新绑定覆盖失败或未观察的旧状态。

`cleanup_turn` 固定到 accepted root，不会把上一轮迟到的调用重定向到新回合。
完整清理任务由宿主持有，调用方停止等待不取消它；重复等待复用同一个结果。
清理先关闭工具派发和所有相关进程 scope 的入口，再尝试进程清理和工具 join。
一个进程 scope 失败仍尝试清理其他 scope，失败 scope 不被从 owner 中删除。

最终 Session 清理也保留唯一完成结果：先等待已有回合清理并完成工具/进程清理，
再释放 Session 快照与 Kernel scope 资源。前置清理失败时不释放 workspace/Kernel
权威句柄，不把“再次释放时 map 已空”误认成之前已经成功。

日志 Cleanup/Terminal 分类仍不是自动清理证明；driver 仍须先等待上述实际 owner。
读写互斥、工具结果落盘和观察屏障继续沿用 loop10 的公共工具宿主。

## 移除平台对 Coding 进程类型的反向依赖

`ManagedEngineProcessOwner`、请求/会话/输出/cleanup 契约和 `EngineProcessError`
现位于 `nomifun-engine-core::process`（通过 crate 根导出），仍使用原来的
`nomi-process-runtime`，没有新增进程 supervisor 或外部执行通道。

应用进程适配器改为 `engine_process_host.rs` 并直接使用公共类型，不再依赖 Coding crate。
原 Coding API 保留 `CodingProcess*`/`ManagedCodingProcessOwner` 名称作为 re-export；
这些方法的错误类型现为 `EngineProcessError`，提供到 `CodingEngineError` 的转换。
这是源码重新打包接口，不宣称保持二进制动态库 ABI。

旧的内部 `coding-session-process:*` 资源标识暂时保留以避免无关的标识迁移；
它不是 engine 选择入口，也不意味着该资源只允许 Coding 使用。

## 剩余边界

此次覆盖现有标准工作区、进程和 Kernel 资源路径，不宣称 MCP/MiniApps/所有 Plugin
生命周期已经接通。还需要真正独立的社区 engine 参考循环及真实成功执行证据、
当前产品 MCP 目录与凭据 owner 接线、非 function Plugin/MiniApps、Git push 外部
副作用回执、跨启动进程证明与人工解隔离、安全 checkpoint 续跑等。

仍只默认预置 Nomi 与 Coding，Agent 工作台选择后随 revision 固定 Session exact build。
社区必须源码/依赖集成后重新打包，不提供打包后挂载。
本切片 Coding build 后缀为 `host2-coding-loop11`，摘要包含新增资源/公共进程源码。
