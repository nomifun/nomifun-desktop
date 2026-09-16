# Engine 组装失败与进程级关闭（slice85，未验证）

分支 `rf/agent-capability-platform-v2`；无 commit/push。未运行测试、构建、服务、
模型、迁移；两个新模块仅做局部 rustfmt。源码实施不构成关闭行为的证明。

## 本轮发现的实际缺口

1. `try_create_router` 虽返回 Result，内部 `build_module_states` 却将 Engine/Plugin
   组装错误转为 panic。桌面和 NomiCore 生产入口又调用 panic 包装的 create_router，
   因此源码扩展与官方目录冲突等组装错误不能进入入口已有的失败清理分支。
2. `shutdown_nomi_core_host` 会停止 Agent Execution 等生产者，但没有等待所有
   Conversation Engine 实例退出。Registry 的 `terminate_all` 只有 kill 请求，
   active count 又排除了隔离/空构建槽，均不能作为 SQLite 可关闭的依据。
3. 独立服务端原先用于早期失败的清理只覆盖部分资源。路由已部分组装时需要保留
   完整 AppServices，而不是只保留 Browser/Gateway 与数据库。

## 实施内容

### 组装错误沿生产路径返回

新增内部 `try_build_module_states`，该函数内直接处理的 JS foundation、Engine/Plugin、
JS manager、Service 注册/存储/运行时、source reconciliation 和 Channel owner/ingress
错误现在返回 Result；`try_create_router` 传播错误。原 build_module_states 和 create_router
保留兼容包装，生产桌面/NomiCore 不再调用它们。未全面捕获更深层第三方代码的 panic。
已有结构性测试源码中的 builder 名称同步，测试未运行。

桌面路由失败先释放尚未服务请求的 loopback socket，再用原 DesktopKeepAlive /
DesktopStartError 流程清理，失败继续保留环境与资源所有权。未发布部分 Router。

### Registry 的永久关闭屏障

`AgentRuntimeRegistry::shutdown_and_wait` 默认明确不支持，不能把自定义 Registry 的
terminate_all 或 active count 伪装成证明。生产 InMemory 实现：

- 同步关闭新建准入并请求当前实例取消；旧 terminate_all 语义不变。
- get/create 全路径持有共享准入读锁，覆盖模型配置读取、每 Session gate 等待、
  冷构建及构建后检查；关闭任务先等待独占锁，再枚举当前/隔离实例，避免只看到
  暂时不存在或空的槽就认定已退出。关闭后的 get_runtime 不再提供新句柄。
- 使用原 exact-slot teardown 和每 Session gate，最多并发 16 个清理，不因一个
  返回错误就跳过其他实例；失败保持 quarantine 与 workspace lease。
- 检查 runtimes、quarantine、turn admissions、workspace/model bindings 全部清空，
  不使用 active_runtime_count。不能证明时返回错误，不清除未知状态。
- 清理在调用时启动并拥有独立任务；Shared flight 让取消/超时的等待者不会中止
  清理。失败结果被观察后可以重试同一永久关闭的 Registry，不允许重新开放准入。
  构建或退出若一直不返回，清理仍 pending，不能据超时推断无副作用。

该屏障处理当前进程已登记实例，不是跨重启进程树证明，也不是允许调用者任意
abort 冷工厂的授权。Engine/资源 owner 仍必须履行其退出与持久回执合同；进程强制
退出、panic=abort、任意第三方任务脱离平台所有权不由此恢复为安全。

### 平台关闭与服务端资源保留

`shutdown_nomi_core_host` 一开始关闭 Engine 准入，随后在 SQLite 关闭前等待该屏障。
本地等待上限 15 秒；超时记为清理失败，数据库保留打开，已拥有的清理任务继续。
其余原有平台 owner 仍分别清理，Engine 成功不替代它们的证明。

NomiCore 在注册回调/finalize/router 失败后现在使用完整 host 清理。新增可 downcast 的
`bootstrap::NomiCoreCompositionCleanupError`，清理未完成时保留完整服务图及其中的
boot server-lock authority，后台仅重试清理，不重新构造 Engine 或重放 turn。
`retry_cleanup` 与后台重试串行，成功状态缓存，避免再次访问已关闭的数据库。
嵌入宿主仍须保留 executor 与启动 environment；后台任务不可能超越 executor 生存。
早于完整 AppServices 的构建失败继续使用既有较小的 startup authority，未声称重写
全部进程启动/退出机制或所有环境锁的所有权。

## 身份、兼容及剩余证据

Coding 前进到 `host2-coding-loop85`，Nomi 为 `host55`。摘要加入 Registry/关闭模块、
AppServices、路由与组合失败清理源码。Engine 的规划/上下文算法、Agent 工作台选择、
编译期注册限制、Session exact binding 和 Fork 继承不变。

自定义 Registry 需提供真实 shutdown_and_wait，否则产品关闭明确失败；这不是要求
每个社区 Engine 重写 Registry。使用公共 HostedAgentRuntime 的 Engine 继续提供
既有 cleanup_turn/cleanup_session，由平台原 Registry 统一调用。

仍缺直接证据：冷构建并发关闭、关闭与缓存命中/取消竞争、单例失败不漏清理、丢弃
等待者、超时后续等、重试与锁/数据库顺序、Nomi/Coding/社区 Engine 实际退出，以及
服务端/桌面组装失败与跨平台构建。未运行以上验证，整体 CAR 保持未完成。网络 Git
凭据、MCP 其他生命周期、Vertex 路由合同及未知跨重启效果的原边界没有解除。
