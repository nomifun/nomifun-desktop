# 源码集成参考：Evidence Engine

这是独立 Engine 的完整接入源码，不是 Coding 的 profile、prompt 包装或新增官方预置。
默认桌面/Web/nomicore 仍仅注册 Nomi 和 Coding。只有本 example 的组合根显式注册
`community.evidence-reference`，不增加打包后安装、挂载或热替换入口。

本切片未运行构建、测试、真实模型或端到端验证。下面是留给后续验证的命令，不是通过记录：

```powershell
cargo run -p nomifun-app --example evidence-engine -- --data-dir C:\nomifun-evidence-sandbox --port 18082 --local
```

选择一个独立的测试数据目录，不要与运行中的桌面应用共享目录。这个 example 启动现有
NomiCore HTTP/WS 服务，没有另造 Session 数据库，也不包含新的桌面外壳。行政操作使用
`nomicore`；自执行 MCP/terminal 辅助入口仍委托平台现有实现。

通过 Agent 工作台/现有 Agent API 选择目录里的 Evidence reference、`read-only` profile，
保存一个包含 `fs.read` 和/或 `fs.search`、精确模型路由及工作区授权的 Agent 版本，再创建
Session。不要在聊天页添加 Engine 切换。新引擎也服从 Agent revision → exact Session binding。
更改已有 Agent 不迁移旧 Session。

源码组合时还注册了 `community.evidence-reference` 的 `stable` 渠道，指向已注册的
`source-v1` 构建。宿主编写的 Agent 配置可使用该渠道和 `read-only` profile；工作台
下拉仍发现并选择精确构建，不新增聊天页切换或在线渠道管理。重复渠道、未注册目标
及组装后的注册均拒绝。调整渠道必须修改源码并重新打包；已有 Session/Fork 不重解析。

桌面二次开发可在原生外壳调用 `DesktopServer::start_with_runtime_engines`，将最后的
注册参数设为同样的 `driver::register`（从二次开发 crate 导出）；其余启动参数与
`start_with_outcome` 相同。返回的 `DesktopStartError` 清理状态及 retained keep-alive
仍必须按原生外壳现有流程处理，不能转换成普通错误后丢弃。注册回调只声明构建、
渠道和恢复 hook，不启动任务或持有平台外部资源。本 example 本身仍是服务端组合根。

## 独立执行策略

1. 模型输出 1～3 个结构化研究问题；严格解析，不把失败 JSON 自动当成有效计划。
2. 每个问题最多两次研究采样，一次最多四个工具调用，整轮最多十二次只读调用。
3. 每次使用“当前任务 + 完整历史回答对 + 证据板”重建上下文，不调用 Coding 的
   transcript/compaction/规划器。只有真实工具结果进入证据板，模型研究性文字不是证据。
4. 单条观察保留最多 768 字节并标注截断；最终综合只能引用板上成功观察的 ID，
   没有成功观察时必须显式给出不确定项。引用结构校验不等于事实正确性验证。

`driver.rs` 实现生命周期与循环，`model.rs` 实现采样/流边界，`history.rs` 实现自己的
`evidence-v1` 回放格式，`main.rs` 是独立组合入口。源代码不使用 `nomifun_coding_engine`、
Coding tool surface、Coding 事件或 app 私有 router API。借鉴 Codex 的固定 StepContext
原则：整个模型批次先映射到当前准确工具表/active-set generation，再派发；工具实际执行
与取消后的所有权交给公共平台宿主。

## 使用的真实端口

- `register_session_hosted`：平台先解析持久 Agent/Session/Snapshot；回调收到受信任事实。
- `read_turn_receipt`、`open_model_port`、`read_model_facts`：真实 accepted root、Broker、
  一次性模型操作领取和主/备用模型限制；Engine 不读取凭证或自行选择 provider。
- `open_kernel_session`、`compile_tool_plan`、`install_tools`：从平台 canonical schema 和
  已选授权装配 `inspect_text` / `find_evidence`，不是本机文件读写的直接封装。
- `EngineToolHost`：持久派发、持久结算、固定 Session 调用身份和取消后的任务持有。
- `cleanup_turn(root)` / `cleanup_session`：真实工具/进程/资源 owner 清理；成功后才写入
  本 Engine 的 cleanup/terminal，再由 HostedAgentRuntime 发布终态。
- `read_history` / `EngineTurnJournal`：复用 Conversation 的日志表，无第二个 SessionStore。

注册闭包现保留 `&Arc<RuntimeEngineHost>`，便于工厂使用 weak host 捕获。社区 crate 可以
依赖 app 公共 facade，由最外层可执行程序同时依赖并注册它；不要让 app 反向依赖该
community crate 形成依赖环。此处使用 app example，Cargo example 是独立编译目标，
不是 app library 内嵌的第三个引擎。

进程关闭现在由公共 Registry 的 `shutdown_and_wait` 关闭新建并等待实例清理；示例
仍实现 driver 的原有清理方法，不另建 Registry。`NomiCoreApplication` 组装失败时
可能返回可 downcast 的 `bootstrap::NomiCoreCompositionCleanupError`：嵌入宿主须
保留启动 environment 和异步 executor，调用 `retry_cleanup` 等待完整资源退出。
保留的后台工作只重试清理，不重新执行任务；kill 请求或等待超时都不是成功证明。

## 明确边界

- 文本输入最多 4096 UTF-8 字节；附件、Skills、MCP、MiniApps、Plugin、按需激活、
  文件写入、命令执行和 steering 均不支持，显式拒绝，不静默忽略。
- 最多八次模型请求；每次 120 秒截止、8192 个事件、256 KiB 流序列化预算、16 KiB 文本。
  工具调用增量与完成内容不一致、重复 ID、不完整 EOF、拒绝或输出耗尽均失败关闭。
- 未知模型上限显式采用 32768 context / 2048 output 的示例策略，再与全部主/备用候选
  取交集；输入按保守序列化字节预算检查，不宣称是精确 tokenizer。
- 历史仅接受最多六轮同构建/同 Snapshot、已 cleanup + terminal 的完整记录；上下文
  历史约 6 KiB，超出时省略整个回答对并标注。无本 Engine 日志的导入/Fork 历史和
  中断日志不猜测、不重放。准备阶段连 receipt 都无法确认时也不伪造清理证明。
- 不注册异常重启恢复扩展。旧进程中断且缺少平台证明时仍隔离；只读并不允许绕过
  boot-generation、资源与日志证明。
- Build digest 是启动时流式计算的当前可执行文件 SHA-256，覆盖最终链接的 SDK、
  依赖、feature 与编译结果。不是从外部文件装载 Engine。升级二进制会改变绑定；
  旧 Session 不自动迁移，示例不是成熟发布/升级方案。
- 本次仅完成源码参考闭环；仍需后续构建与实际执行证据，不能据此宣称社区 SDK 已验收。
