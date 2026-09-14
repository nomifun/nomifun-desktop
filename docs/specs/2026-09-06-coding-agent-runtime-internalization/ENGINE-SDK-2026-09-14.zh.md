# 公共 Engine SDK：生命周期、模型端口与任务托管

分支：`rf/agent-capability-platform-v2`。日期：2026-09-14。
本切片是源码实现，未运行构建、测试、评测或真实 provider；未提交、未 push。
Coding build 后缀更新为 `host2-coding-loop7`。不将整体 CAR 或完整 SDK 标记完成。

后续 `loop8` 已完成工具契约/Kernel adapter 的中立化和当前产品 Session/receipt 读取，
下文缺口是本切片时点；最新状态见 [ENGINE-PORTS-2026-09-14.zh.md](ENGINE-PORTS-2026-09-14.zh.md)。

## 已抽取且由生产 Coding 使用的部分

`nomifun-ai-agent::engine_sdk` 提供：

- `EngineSessionDriver`：每个已绑定 Session 的策略与宿主端口组合。执行循环、规划、上下文
  仍由 engine 实现；消息根/权限/工具/历史事实由原平台宿主确认。
- `HostedAgentRuntime`：统一实现 Registered runtime 的 single-flight、取消、退出等待、
  panic 隔离和清理后终态发布。没有新增 Session DB 或替代现有 registry。
- `EngineTurnOutput` / `EngineProgress`：只提供非终态投影。固定 Session/turn generation，
  执行退出后关闭输出门；旧输出不能落到新 turn，清理阶段也不继续接受迟到文本。
- `EngineTurnOutcome`：宿主清理成功后持久化的同一份终态，随后才发布 UI finish/error。
  engine 不能通过该输出接口直接广播 Finish 绕过清理。
- `EngineTaskGroup`：托管已准入的异步副作用，保留退出凭据，限制在途任务数量；它不是
  权限/幂等/日志系统，也不把任务返回当成进程树已清理。
- `hosted_engine_factory`：将 driver factory 包装为现有 catalog factory。

模型接口已移至 `nomifun-chat-model-broker::engine_port`，SDK 同时重导出
`EngineModelPort`、`EngineModelStream`、`BrokerEngineModelPort`。该模块不依赖 Coding。
模型路由、凭据、重试/failover 和 transport 仍由 Broker 决定，取消只调用原生可取消入口。
Coding 原有 `CodingModelPort` / `BrokerCodingModelPort` 名称成为兼容重导出，不维护第二套实现。

## Coding 的实际消费链

`CodingAgentRuntime` → `HostedAgentRuntime` → `CodingSessionDriver` → 原 Coding loop。
Coding 特有的计划、指令、压缩、能力激活、steering 和事件 codec 没有移入公共生命周期层。

`CodingSessionDriver` 使用现有 `CodingRuntimeHost` 准备真实回合、校验 principal、执行循环，
并核对成功返回值与缓冲终态一致。公共层完成 cleanup 后，driver 把同一 outcome 转换为原
`CodingEngineEvent` 落入现有日志。失败、取消和 UI 结果不再由两套分支各自推导。

生产 `JoinedTools` 已改用公共 `EngineTaskGroup`，保留原有工具结果摘要与未消费结算记录。
调用取消仅停止等待；实际工具仍在宿主托管任务中完成，宿主随后 join 并记录结算结果。
任务 panic 会保留失败隔离，join 仍等待其他任务，不会因第一个错误跳过剩余副作用。
最终 Session 清理关闭任务准入；普通 turn/能力激活边界使用可复用 join，并由宿主先隔离生产者。

## 退出约束

Session teardown 由公共层保留一个独立 cleanup attempt。调用者丢弃等待 future 不会取消该任务；
后续等待复用同一结果。已知失败可以开启后续重试，未完成的 attempt 不会被误当作空资源集合。
失败重试仍要求宿主幂等、保留真实进程/资源凭据，不能伪造成功。

active turn 的完成 future 不强引用其所属 runtime，避免 runtime → active → future → runtime
引用环。清理任务注册在异步宿主 executor 上；缺少 executor 时明确报错，不返回虚假退出证明。

## 编译期接入

`RuntimeEngineHost::register_hosted(descriptor, driver_factory, admission)` 复用既有目录准入与
组装冻结规则。`driver_factory` 类型为 `EngineDriverFactory`，收到服务端 build options 和
exact binding 后返回 `Arc<dyn EngineSessionDriver>`。工厂必须取得已经验证的宿主端口。

这一入口没有安装包上传、动态库加载、运行中挂载或热替换功能。默认产品目录仍只有 Nomi
和 Coding；社区 engine 必须修改应用源码/依赖后重新打包，且只在 Agent 工作台选择。
需要超出本 SDK 的特殊运行时仍可实现完整 `RegisteredAgentRuntime`，不得绕过平台 owner。

## 尚未完成，不能由本切片代替

1. 通用的精确 Session/turn 端口装配工厂，供社区 engine 从当前 Conversation owner 获得
   已准入的 history、model、canonical tool mapping 与 state；目前 Coding 的具体宿主仍在 app 内。
2. 引擎无关的工具映射/调用契约和历史 codec 接线。`EngineTaskGroup` 仅完成任务托管，
   不能因此宣称社区 engine 已取得任意工具；原 Kernel 仍是唯一权限与执行准入来源。
3. 不依赖 Coding 循环、通过默认产品 owner 成功执行的第三方参考 engine。不能把
   driver trait 或 factory helper 当成已运行示例。
4. MCP 当前产品目录/精确映射/凭据和执行 owner；MiniApps、非 function Plugin 生命周期、
   push 持久回执、进程跨启动清理证明、安全续跑、动态指令范围等既有缺口。
5. 用户允许后再做 SDK/Coding 生命周期、取消与清理回归，以及消费者、升级和多平台验收。

原有 Coding 投影和工具托管测试的源码已随内部结构调整，但没有执行；旧通过结果不能证明
这次抽取正确，也没有因此宣称 Coding 比 Nomi 或 Codex 更强。
