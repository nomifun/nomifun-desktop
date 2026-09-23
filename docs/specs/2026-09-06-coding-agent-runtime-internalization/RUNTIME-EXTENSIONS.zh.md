# 开放 Runtime 接入边界

> 归档边界（2026-09-23）：本文涉及 MiniApp、旧 Plugin 发布/Host 的段落仅保留历史背景，
> 不得作为当前 Plugin 合同或实现依据。Plugin 唯一权威为
> `../2026-09-22-unified-plugin-core/README.zh.md`。

依据用户 2026-09-13 的补充要求：NomiFun 是开放平台，执行引擎必须可替换；
不能把接入方式固定为 Nomi/Coding 两个具体实现。所有开发统一在
`rf/agent-capability-platform-v2`，不 push。

按 CAR-D-021，第一阶段仅开放**编译期集成**：添加实现/依赖及组合入口注册后，
重新构建打包应用。发布后的应用不能挂载或安装 Engine；不提供动态库 ABI 或热更新。

## 已实现的进程内扩展接口

2026-09-14 slice90：Coding 自身 codec 增加便携压缩替换上下文，宿主仍负责
精确历史读取和限长持久观察。社区 Engine 不需要实现此 Coding 专属记录，
公共 Session 所有权和编译期注册方式不变。详见
[CODING-COMPACTION-CHECKPOINT-2026-09-14.zh.md](CODING-COMPACTION-CHECKPOINT-2026-09-14.zh.md)，未验证。

2026-09-14 slice89：共享宿主新增 `read_message_history_before_turn`，按同 Session
的精确历史 receipt 读取消息前缀；当前 turn、清理起点和尺寸/条数边界继续有效。
Coding 在完整原生窗口之前加入 Fork/导入前缀，再顺序应用事件/压缩，不在压缩后
追加旧历史。社区 Engine 自行决定组合策略，宿主不接管循环或 codec。见
[CODING-HISTORY-PREFIX-2026-09-14.zh.md](CODING-HISTORY-PREFIX-2026-09-14.zh.md)，未验证。

2026-09-14 slice88：源码策略 `uses_platform_history_context(binding)` 默认 false。
仅每轮从平台历史端口重建对话上下文、没有私有历史副本的实现可开启；Coding 开启。
平台维护会回收 runtime 并持久推进模型历史起点，不删除 UI 消息、效果凭据或恢复
义务；与 Nomi 私有 Session 策略互斥。社区无需新的 Engine 枚举或动态安装入口。
Fork 对此类实现只用复制消息提供历史。详见
[ENGINE-CONTEXT-CLEAR-2026-09-14.zh.md](ENGINE-CONTEXT-CLEAR-2026-09-14.zh.md)，未验证。

2026-09-14 slice87：`RuntimeEngineAdmission::uses_nomi_session(binding)` 是源码
注册策略，默认 false，不能从 family 或用户 JSON 推断。仅完整复用 Nomi 私有
持久化/恢复协议与 Plugin scope 的实现可开启，并须与 runtime 的
`uses_nomi_recovery()` 一致；生产 Registry 拒绝并清理不一致的构建。
独立 Engine 无需开启此标志，继续使用自己的 exact-build 恢复钩子；缺失旧构建
不借用新版 Nomi codec。自定义 Registry 默认拒绝 bound Engine 的 Nomi 私有访问。
见 [ENGINE-EXACT-SESSION-POLICY-2026-09-14.zh.md](ENGINE-EXACT-SESSION-POLICY-2026-09-14.zh.md)，未验证。

2026-09-14 slice86：生产 Registry 持有冷构建任务，调用者丢弃只取消独立的等待
token，不能直接 abort 工厂。工厂必须在正常 Err 前结清部分资源；异常退出导致
Registry 准入关闭和退出证明拒绝，空槽/零 active count 都不能替代证明。
工厂应从 options/平台端口取得依赖，不依赖调用者私有 task-local。
见 [ENGINE-ACQUISITION-2026-09-14.zh.md](ENGINE-ACQUISITION-2026-09-14.zh.md)，未验证。

2026-09-14 slice85：默认公共 Registry 新增 `shutdown_and_wait`，关闭后禁止新建，
等待在途构建与 exact-slot 清理；失败/超时保持隔离与资源，不以 kill 请求替代退出。
源码 Engine 仍实现原 driver 清理方法；只有自行替换 Registry 的宿主需要实现此新
证明接口，默认拒绝。生产组装错误接回桌面/服务端清理，服务端完整资源保留错误可
downcast 为 `bootstrap::NomiCoreCompositionCleanupError` 并重试清理。
见 [ENGINE-SHUTDOWN-2026-09-14.zh.md](ENGINE-SHUTDOWN-2026-09-14.zh.md)，未验证。

2026-09-14 slice84：`RuntimeEngineHost::register_channel(family, channel, build)`
补齐打包时社区渠道声明；先注册构建，再声明渠道，重复/未安装目标/组装后操作拒绝，
官方 stable 不可覆盖。桌面源码宿主可用 `DesktopServer::start_with_runtime_engines`
传入与服务端相同形状的注册回调，保留 DesktopStartError 及失败清理所有权。
Evidence 示例已声明自己的 stable；工作台仍选择精确构建，旧 Session/Fork 不迁移。
见 [ENGINE-COMPOSITION-2026-09-14.zh.md](ENGINE-COMPOSITION-2026-09-14.zh.md)，未验证。

2026-09-14 slice59：Coding 历史 codec 区分完整的继续边界与中断末尾，拒绝重复/
跨步工具结果，并保留真实结果发布顺序；重建全部成功后才替换派生上下文。宿主
零步中断不是无执行证明，清理/解封仍由平台凭据负责，不重放工具。公共 SDK 与
编译期 Engine 接入方式不变。详见
[CODING-HISTORY-BATCHES-2026-09-14.zh.md](CODING-HISTORY-BATCHES-2026-09-14.zh.md)，未验证。

2026-09-14 slice58：Coding 自己记录模型批次是否进入工具端口，据此区分引擎暂缓和
已尝试失败。前者不推进工作区观察 epoch，也不能作为完成证据；后者仍保守处理。
此活跃循环事实不是平台 owner 派发/清理证明，不用于跨重启恢复；公共端口与编译期
Engine 注册不变。详见
[CODING-DISPATCH-ACCOUNTING-2026-09-14.zh.md](CODING-DISPATCH-ACCOUNTING-2026-09-14.zh.md)，未验证。

2026-09-14 slice57：Coding 压缩保留策略可跨过最近工具结果后的普通 assistant 回答，
保留完整批次与后续输入/说明；预算和完整性核对由 Coding 决定，平台不解释任务进度。
消息数也成为强制预检，不能通过二次裁剪必需输入达到预算。详见
[CODING-CONTEXT-FOLLOWUP-2026-09-14.zh.md](CODING-CONTEXT-FOLLOWUP-2026-09-14.zh.md)，未验证。

2026-09-14 slice56：共享平台进程 scope 在 owner 操作前写派发事实，在完整回收观察后
写同序号 barrier；引擎事件/自然语言不能替代该证明。Coding 精确恢复据此关闭已知
回收的中断历史，不执行旧工具；社区 Engine 的恢复 codec 仍需自行注册并核对。
详见 [ENGINE-PROCESS-BARRIERS-2026-09-14.zh.md](ENGINE-PROCESS-BARRIERS-2026-09-14.zh.md)，未验证。

2026-09-14 slice55：共享 `ChatProviderReasoning` / `ProviderReasoningBlock` 为明确
类型化的 Anthropic 完整签名/隐藏块，社区 Engine 可按原序保留并交共享编码器回传。
隐藏内容不是指令、工具结果或完成证据；只允许 assistant 内容，同协议之外明确拒绝。
Coding 仅发布其中可见文本，摘要/输出截断不泄露隐藏载荷；不提供跨重启秘密存储。
详见 [ENGINE-PRIVATE-REASONING-2026-09-14.zh.md](ENGINE-PRIVATE-REASONING-2026-09-14.zh.md)，未验证。

2026-09-14 slice54：MaxOutputTokens 是共享协议事实，不是重试或完成授权。Coding 自己
选择整批不执行、记录作废 ID、清除本步不完整续传并最多两次返回模型边界的策略。
不增加路由/额度或 Kernel 授权；社区 Engine 可采用不同策略，不能执行半截参数。
详见 [CODING-OUTPUT-LIMIT-2026-09-14.zh.md](CODING-OUTPUT-LIMIT-2026-09-14.zh.md)，未验证。

2026-09-14 slice53：官方 Anthropic adapter 的 attempt decoder 把原生内容块/工具参数/签名
归一化为现有共享事件；Engine 不需要解析供应商块索引或自己合并累计用量。
共享 WireBudget 约束 Responses/Anthropic 原生流，工具执行仍经平台权限与效果 owner。
当前只实现标准 text/tool_use/thinking，其他原生块拒绝；详见
[ENGINE-ANTHROPIC-LOOP-2026-09-14.zh.md](ENGINE-ANTHROPIC-LOOP-2026-09-14.zh.md)，未验证。

2026-09-14 slice52：共享 Broker 增加 request-aware `new_frame_decoder_for`（默认委托旧工厂）
及原生 Responses 工具/内容生命周期转换。社区 Engine 可消费统一的 `ReasoningBlock`，
保留完整摘要/opaque 续传和块边界；不能将它与 ReasoningDelta 重复拼接或跨协议错用。
显式 native-preservation consumer 的完成项是额外快照，不应重复执行规范化工具调用。
工具仍由平台授权/执行，不支持运行时挂载。详见
[ENGINE-RESPONSES-LOOP-2026-09-14.zh.md](ENGINE-RESPONSES-LOOP-2026-09-14.zh.md)，未验证。

2026-09-14 slice51：共享 Broker 提供流内明确错误分类及官方协议的单请求 decoder，
Engine 可按自身策略处理类型化错误；传输重试仍属于 Broker，Coding 的压缩恢复
不切换 Engine/路由。旧有状态协议 adapter 需要实现 `new_frame_decoder` 或自行隔离。
详见 [ENGINE-MODEL-STREAM-ERRORS-2026-09-14.zh.md](ENGINE-MODEL-STREAM-ERRORS-2026-09-14.zh.md)，未验证。

2026-09-14 slice50：Coding 自己决定压缩后近期工具交换的原文保留预算及顺序，
平台仍只提供原有工具结果和永久事件；历史引用不构成新的工具派发。
这项策略不强制进入 Nomi 或社区 Engine，详见
[CODING-CONTEXT-TAIL-2026-09-14.zh.md](CODING-CONTEXT-TAIL-2026-09-14.zh.md)，未验证。

2026-09-14 slice49：共享 Broker 补齐完整错误响应的明确上下文超限分类，
`PromptTooLong` 作为平台事实提供给 Engine。Coding 增加一次性、语义输出前的
压缩续接策略，Nomi/社区 Engine 不被强制采用；不新增热加载、路由修改或工具重放。
详见 [CODING-CONTEXT-LIMIT-RECOVERY-2026-09-14.zh.md](CODING-CONTEXT-LIMIT-RECOVERY-2026-09-14.zh.md)，未验证。

- `nomifun_ai_agent::AgentRuntimeControl`：消息发送、事件订阅、取消、健康状态。
- `nomifun_ai_agent::RegisteredAgentRuntime`：必需的可确认退出，以及可选控制操作。
- `nomifun_ai_agent::runtime_registry::AgentRuntimeFactory`：异步构造接口。
- `AgentRuntimeHandle::Registered(Arc<dyn RegisteredAgentRuntime>)`：通用生产句柄。
- `RuntimeEngineCatalog`：异构 descriptor/factory 注册、发现、开放 profile/channel、
  exact Build 解析和恢复验证；同一 family/build 不允许覆盖工厂或摘要。
- `RuntimeEngineAdmission`：注册时必需的 Snapshot 与有效 Session overlay 兼容性校验。
  内置与社区引擎共用；`RuntimeEngineSupport` 提供初始能力集合及 Skills/MCP/按需/
  MiniApp 支持的便捷策略。自定义策略可按 exact binding/profile 执行额外校验。
- `CodingAgentRuntime`：Coding 的 Registered 实现；`CodingRuntimeHost` 强制宿主
  提供真实会话准备、语义记录、turn 和 Session 清理证明。

现有 Nomi factory 已改为返回同一 Registered 句柄。新增用户 Runtime 不需要
添加一个枚举分支，也不需要修改 Conversation/Remote/Automation 的调用方。
旧 `Nomi` 构造变体仅保留既有 API 兼容，不作为可扩展引擎选择器。
`AgentType` 是旧产品分类，不能拿它当 Engine family/build 标识。

一个二次开发 Runtime 实现生命周期 trait，在产品组合前注册 descriptor/factory/策略：

```rust,ignore
let application = NomiCoreApplication::compose_with_runtime_engines(
    &environment,
    |host| host.register(custom_descriptor, Arc::new(move |options, exact_binding| {
        let ports = admitted_platform_ports.clone();
        Box::pin(async move {
            let runtime = CustomRuntime::open(options, exact_binding, ports).await?;
            Ok(Arc::new(runtime) as Arc<dyn RegisteredAgentRuntime>)
        })
    }), Arc::new(CustomAdmission)),
).await?;
```

这是接入形状示例，`CustomRuntime`、`CustomAdmission` 与 `admitted_platform_ports`
由二次开发宿主提供。也可以使用 `RuntimeEngineSupport::initial_only(...)`；
平台完整支持适配器可以显式使用 `RuntimeEngineSupport::platform()`，不能将其理解为
授予全部权限。完整的通用模型/工具/历史宿主 SDK 仍待抽取，不把该示例当作已经可运行
的第三方 Engine。动态安装不属于第一阶段目标，也不另建 Session 存储。

2026-09-14 起也可使用 `RuntimeEngineHost::register_hosted`，传入 `EngineDriverFactory`
与同一份 admission 策略。driver 实现 `nomifun_ai_agent::engine_sdk::EngineSessionDriver`，
公共 `HostedAgentRuntime` 负责任务、取消、清理后终态与 UI 代次隔离。Coding 已使用此公共
生命周期实现。引擎无关模型端口位于 Broker，公共 `EngineTaskGroup` 保留工具退出凭据。
这仍不是完整生产端口装配 SDK；工厂必须自己取得已准入端口，不得从 options 伪造授权。
具体完成范围及缺口见 [ENGINE-SDK-2026-09-14.zh.md](ENGINE-SDK-2026-09-14.zh.md)。

后续 `host2-coding-loop8` 已新增独立 `nomifun-engine-core` 工具契约/Kernel adapter，
以及当前产品的 `EngineSessionHost::resolve/read_turn_receipt`。
可用 `register_session_hosted` 注册接收 options、真实 `AdmittedEngineSession`、Session host
的 driver 工厂。Coding 已消费这些公共组件；该入口仍不自动提供完整历史与副作用端口，
读取 receipt 不等于执行授权。完整第三方执行示例仍待完成，详见
[ENGINE-PORTS-2026-09-14.zh.md](ENGINE-PORTS-2026-09-14.zh.md)。

`loop9` 增加 `EngineSessionHost::open_journal/read_history/open_model_port`，导出引擎无关日志、
model claim、原始历史窗口与真实生产 Broker 模型装配；Coding 已接入，工具结果在宿主任务中持久化后返回。
日志写入分类不等于工具授权或清理证明，仍需实际资源 owner。
详见 [ENGINE-JOURNAL-2026-09-14.zh.md](ENGINE-JOURNAL-2026-09-14.zh.md)，独立 engine 示例仍未完成。

`loop10` 增加 `read_model_facts/read_message_history` 与公共 `EngineToolHost`。
模型事实保留未知值，由 engine 明确选择上下文策略；工具宿主负责持久派发/结算、
重复调用拦截、只读共享/副作用独占和取消后任务持有。Coding 已使用这些接口。
`EngineToolHost` 仍要求应用装配真实 canonical Kernel invoker，不是资源授权工厂；
社区扩展不应直接把任意函数包装进去并声称获得平台权限或进程退出证明。
详见 [ENGINE-EFFECTS-2026-09-14.zh.md](ENGINE-EFFECTS-2026-09-14.zh.md)，本切片未验证。

`loop11` 增加 `EngineSessionHost::open_kernel_session` 和根 facade `EngineKernelSession`，
从 owner 解析的工作区与资源授权完成真实 Snapshot/Kernel 编译、工具宿主装配和
root-bound 进程/工具清理。Coding 已共用此入口；进程 owner 下沉到 engine-core，
平台不再依赖 Coding 的进程类型。通用接口使用现有资源 owner，不代表新增生态已经接通。
详见 [ENGINE-RESOURCES-2026-09-14.zh.md](ENGINE-RESOURCES-2026-09-14.zh.md)，本切片未验证。

## 不可省略的生命周期约定

后续独立源码参考已加入 `nomifun-app/examples/evidence_engine`（Cargo example
`evidence-engine`），使用公共生产端口实现自己的规划/上下文/证据循环，不包装 Coding。
组合回调现在接收 `&Arc<RuntimeEngineHost>`，可直接调用 `register_session_hosted`。
这是可选组合根，不是默认第三个官方引擎；源码已写入，构建与执行尚未验证。
详见 [ENGINE-REFERENCE-2026-09-14.zh.md](ENGINE-REFERENCE-2026-09-14.zh.md)。

1. Session ID、owner、workspace 和消息根身份使用宿主提供的值。
2. 现有 registry 负责 single-flight、turn generation、workspace lease 和 teardown
   quarantine；第三方 Runtime 不另建一个竞争的会话协调器。
3. `kill_and_wait` 必须证明 Runtime task 与子进程都已退出。失败要返回错误，
   让宿主保留 quarantine 并拒绝创建替代实例，不能把“已发送 kill”当作成功。
4. 不支持 clear/steer/rewind/model switch 等变更操作时，默认明确报错。
   不伪造成功、不以另一个 Runtime 自动重试原请求。
5. 工具、模型和资源必须使用已准入的 NomiFun owner ports。此接口是受信任
   二次开发代码的进程内扩展点，不提供任意本机代码的安全沙箱。

## 目录与 Coding 适配器

目录先注册 `RuntimeEngineDescriptor` 和 `RuntimeEngineFactory`，再调用
`resolve(selector, profile)` 得到 `RuntimeEngineBinding`。宿主必须在 Session
创建/Fork 的唯一事务里保存 binding。恢复只调用 `validate_binding` / `open`，
不再次解析 channel。`bound_factory` 用于固定 exact Build 的单引擎宿主；
多引擎宿主应从每个 Session 的持久 binding 调用 `open`。

Coding 使用 `coding_runtime_descriptor` 描述自身，构造 `CodingAgentRuntime`
后返回 Registered，不增加专用 handle 分支。宿主提供已经准入的 model/tool
ports；`CodingRuntimeHost::prepare_turn` 必须校验真实消息根、owner、snapshot、
route、active set，不能从请求 extra 伪造。adapter 还会校验 Session 和 principal。
`record_event` 在 UI 投影前记录语义；`cleanup_turn` 成功前不发布终态。
取消或被丢弃的 teardown waiter 不能让旧任务脱离 registry 的退出隔离。

支持并发追加输入的 Coding 宿主还须覆写 `CodingRuntimeHost::admit_tool`：
收件检查和 `ToolStarted` 持久化必须处于同一同步边界，返回 false 不得写入准入记录或执行工具。
没有并发输入 owner 的宿主才可使用默认实现。生产宿主禁止正数 step 的 ToolStarted 走普通
`record_event` 绕过该边界，准入成功后才广播 UI；锁不覆盖工具实际执行。

默认组合根现已提供 `ConversationCodingHost`，经真实 Broker/Kernel/Wave2 工作。
宿主为每个已开始的工具持有可重复等待的退出证明；取消后的工具结算结果也在
回合终态之前持久记录。未具备真实进程 cleanup 证明的能力仍拒绝准入。

## 当前产品接线与剩余边界

Agent 是用户配置的身份、指令、模型及能力组合；runtime 是执行该 Agent 的可替换实现，
不是另一个与 Agent 平行的产品身份。多个 Agent 可共用一个 runtime 实现，但各 Session
持有各自的绑定和执行状态。产品入口只在 Agent 工作台，日常对话选择 Agent 即可。
配置存入 `AgentPresetRevisionPayload.runtime_engine`，与其他配置一起版本化；Session
创建由服务端将该版本的选择解析为 exact binding。旧 Agent 省略字段时使用默认 Nomi。

以下接线已实现：

- 默认生产组合根中的异构目录以及宿主二次开发注册入口；
- 唯一 Session 创建/Fork 事务冻结 binding，公共 PATCH 与 DB 触发器保护；
- 默认路由中的 Coding 工厂、现有 Conversation 历史、Broker 因果领取及文件/Git 工具；
- Agent 工作台引擎选择、版本化保存及本地模型 HTTP fixture 驱动的真实默认路由测试；
- 创建/Fork DTO 无独立 runtime 覆盖，修改 Agent 不热迁移已有 Session。
- 各引擎注册必需的兼容性策略；保存 Agent、创建/Fork、修改能力、切换 Agent 及
  runtime 构造统一调用策略，不再由业务路由特判 Coding 名称。
- Coding 向能力查询返回 Kernel 工具准入所用的同一 active set，而非另外计算快照。
- Coding 循环接入有界上下文组装并记录 `context_prepared`；历史截断保留完整回合，
  不裁断执行中的工具调用链。每次模型调用前检查字节预算，超限明确失败。

后续切片已接入 turn-owned Process、冻结按需激活、制品 Skills/资源分页、路径 AGENTS、
compaction 和有回执的 text steering，均须按 STATUS 中的切片说明区分未验证实现。
剩余：独立 engine 示例的成功执行证据与更多生态资源接线、MCP/MiniApps 与非 function Plugin 生命周期、
更广泛 Skill 资源格式与消费者继承、动态路径指令范围、安全 checkpoint 续跑、异常进程重启证明、
逐工具与多平台验收、Remote/Automation 的完整继承行为验证。
非 Nomi 孤儿回合不会套用 Nomi 私有恢复日志；只能调用 exact-build 注册的恢复证明接口，
没有足够证明时继续隔离，不能自动重放副作用。
默认引擎仍为 Nomi；Coding 是在 Agent 工作台显式配置的实现。未知构建不会静默回退。

2026-09-14 多 MCP 服务器补充（未验证）：官方 Nomi/Coding 已接最多 16 个 HTTP 服务器的
冻结逐工具通道。平台按每个 tool lock 的 server ID 产生唯一资源 policy，Engine 必须发送该
policy 的完整 binding IDs，不能选择另一服务器或发送所有 Session 资源。MCP effects 仍串行，
任一未知效果仍隔离 Session。资源多选不是工具授权，也不是运行时加载 Engine。
详细约束见 `ENGINE-MCP-MULTI-SERVER-2026-09-14.zh.md`。

2026-09-14 HTTP MCP 协议补充（未验证）：服务器主动请求仍属于平台 owner，不由 Engine
任意回调执行。共享 owner 仅直接服务 ping；sampling、elicitation、roots 和其他未声明方法
返回固定 JSON-RPC 错误。全事务服务器请求预算、固定 endpoint/凭据/Session 和原有 deadline
同时约束应答；不接收递归 reply stream，不重连/重放。收到目录变更或当前请求取消时中止；
已经派发的工具保持未知效果隔离。此路径不等于完整的用户介入或 MCP resource 生命周期。
详见 `ENGINE-MCP-PROTOCOL-2026-09-14.zh.md`。

2026-09-14 Coding 图片 Skill 补充（未验证）：已选不可变制品内 PNG/JPEG/WebP 资源可由
宿主限量解码、校验及重新编码后，经 read_context_resource 按精确 ID 返回模型图片结果。
文本仍分页，脚本仍不执行。读取必须具有 active llm.vision 及支持 ImageInput 的 exact
primary route；按需激活仅在平台持久提交后更新可读状态。原有媒体预算、历史省略图片和
Broker 逐请求能力检查保留。不支持任意路径图片读取或任意二进制格式。
源码扩展注意：CodingContextResource.text 改为 content（CodingContextContent 的 Text/Image），
CodingCapabilityView 新增 context_image_input；自行实现 Coding host 必须依据真实平台准入
投影该字段，不能照搬为 true。此为编译期 Rust 源码接口调整，不是稳定动态 ABI。
详见 `CODING-SKILL-IMAGES-2026-09-14.zh.md`。

2026-09-14 Coding 完成策略补充（未验证）：计划步骤可调整，但已登记的来源关联需求只增不减。
update_plan.requirements 关联当前回合真实已接受输入，report_completion 必须覆盖全部 ID，
包括已移除计划步骤背后的义务。scope_changed 必须有后续输入引用且最终显式披露，不是原工作
已完成的证明。这是 Coding 内部策略，不强制其他 Engine 实现相同 planner，也不增加平台权限。
CodingPlan/CompletionReport 新增 requirements，criterion 新增 requirement_ids/scope_change；
旧历史字段缺省只用于解码，不允许绕过当前回合的登记与覆盖检查。编译期宿主需跟随类型更新。
详见 `CODING-REQUIREMENTS-2026-09-14.zh.md`。

2026-09-14 Coding 交互命令补充（未验证）：命令观察保留 launch epoch 与独立交互来源 epoch，
终态记录最多 8 个 stdin/EOF/resize 调用 ID。仅在连续独占且此前未过期时延续观察时效，
失败/并发/其他修改/省略记录仍不可用；真实退出与清理规则不变。此策略不修改公共进程 owner，
不重新派发命令、不借用其他 Engine 的历史或跨启动证明。完成上下文同时限制记录数与字节。
详见 `CODING-INTERACTIVE-EVIDENCE-2026-09-14.zh.md`。

2026-09-14 工作区文本分页补充（未验证）：平台 fs.read 已改为严格参数的有界文本页，
offset > 0 必须带前页 expected_sha256。Coding 的仓库指令读取要求 content、sha256、
total_bytes、offset、next_offset、eof；自行实现该工具的源码宿主需同步，不可省略完整性字段。
引擎保留循环／指令拼接策略，FileService 保留授权与字节读取；摘要不是快照或写权限。
fs.search 同步改为 fresh bounded scan，返回不完整原因、跳过计数、匹配源码偏移和摘要；
宿主不能将过滤后的空结果宣传为全工作区不存在的证明。
详见 `CODING-TEXT-PAGES-2026-09-14.zh.md`。

2026-09-14 工作区图片补充（未验证）：fs.read(format=image) 支持有界 PNG/JPEG/WebP，
不接受 offset/limit。共享 EngineKernelSession 工具端口在 exact active vision／primary route
检查后，将 canonical builtin 的内部结果转为 typed Image 再交给日志；任意 Plugin JSON
不会触发该转换。Coding 限制单调用 batch，压缩保留最新待观察图片的完整工具配对；历史像素
仍可省略，需重读。自建低层 Kernel host 不能假定内部图片 JSON 是通用动态扩展协议。
详见 `CODING-WORKSPACE-IMAGES-2026-09-14.zh.md`。

2026-09-14 Coding 任务延续补充（未验证）：源码宿主可通过
`CodingPriorTask::from_closed_turn` 与 `CodingTurnRequest::with_prior_task` 提供最近闭合任务；
宿主负责 canonical/latest 选择，Coding 额外检查完整 exact binding、身份／terminal／预算。
不能从模型摘要、任意旧任务或 fork 文本回退构造候选。resume_task 只导入有来源的需求并要求
重新规划，不恢复进程或工具证据；社区 Engine 可自行实现不同策略，不强制复用此循环。
模型提交 update_plan 时不能提供 origin；读取 PlanUpdated／CompletionReported 的源码消费者
需认识可选的 CodingRequirementOrigin 历史字段。详见 `CODING-TASK-CONTINUATION-2026-09-14.zh.md`。

2026-09-14 Skill 冻结发布补充（未验证）：Library 详情增加源文件预览／摘要确认，并复用
Plugin Package v1 生成只含冻结 Skill 资源和固定 no-op 入口的候选；仍需显式启用、Agent 选择
及新 revision。Node 依赖是当前 Plugin 格式限制，不表示允许打包后挂载 Engine。
共享 EngineContextResource/EngineContextContent 为纯数据，Coding 的旧类型保留别名；
社区通过 EngineSessionHost.read_selected_skills 获取 SelectedEngineSkills，决定自身上下文策略。
官方 Nomi 已接冻结正文／索引、分页与 vision/model-gated 图片；无可变目录或最新版本回退。
该旧 Skill-to-Plugin 发布切片已退役；其 Coding loop27 / Nomi host12 记录未验证。

2026-09-14 宿主效果补充（未验证）：该切片 Coding loop28 / Nomi host13。
Nomi 的 with_effect_scope 同时覆盖 Kernel、MiniApp、动态工具 invoker，预留一个 mandatory
context guard；工具等待被取消后关闭 turn，未知结果关闭 Session。平台真实 MiniApp/Robot
适配器提供 exact-turn 持久凭据和 source replay witness，scope 自身不提供远端完成证明。
NomiPluginToolError 新增 OutcomeUnknown，源码 exhaustive match 需适配，模型仅接收固定
HOSTED_EFFECT_UNPROVEN。保留任务不等于 Service lease 退出或 Robot 物理停止；初始化、
按需激活等非工具生命周期及 Coding 的对应工具面仍未接齐，不得宣称自动受此包装保护。
详见 `ENGINE-HOSTED-EFFECTS-2026-09-14.zh.md`。Engine 编译期加载政策不变。

2026-09-14 MiniApp 工具端口补充（未验证）：该切片 Coding loop29 / Nomi host14。
EngineKernelSession.miniapp_tool_plan 读取 exact schema，EngineToolPlan.merged 合并且拒绝
重名，install_tools 安装真实平台复合 invoker；MiniApp 保持独立冻结类型，不伪装 Plugin。
初始 MiniApp 纳入共享 active set，on-demand 仍要求 durable activation；返回的完整工具
计划是预览，模型可见面必须按 live active set 过滤。Coding 已接初始及按需路径；Nomi 复用
同一平台调用／receipt 实现。公共模型 claim 和清理均拒绝 hosted pending。
社区可调用 hosted_effect_context 保留效果历史，不能据 transcript 缺失重复执行。
仅支持当前无额外资源依赖的 function actions；Robot／非工具生命周期不由此自动接通。
旧 MiniApp 端口切片已退役；编译期 Engine 注册和 immutable Session binding 的历史结论不变。

2026-09-14 Robot 端口补充（未验证）：当前 Coding loop30 / Nomi host15。
公共 `EngineKernelSession.robot_tool_plan()` 冻结真实设备 schema/名称，install_tools 串行
执行并保留 owner 回执；必须显式选择/激活 link，音频调用另外要求 audio。link/audio 是
平台共享 lease，不是模型工具。`robot_vision_context(generation)` 读取已有近期文字观察，
没有拍照或新模型调用。Coding 的可选 CodingLiveContextPort 每轮替换动态观察槽，保留
预算/取消/超时并使变化前的完成 review 失效。社区源码集成 engine 可直接消费公共端口，
不需继承 Coding 循环；仍需自己的 durable activation、上下文策略和资源清理。
匹配 Robot 回执只证明宿主请求已经结算，不证明物理停止；其他非工具生命周期仍未完成。
详见 `ENGINE-ROBOT-PORT-2026-09-14.zh.md`；不允许打包后挂载或热换 engine。

2026-09-14 路径指令端口补充（未验证）：当前 Coding loop31 / Nomi host16。
共享 fs.read 新增 `format=instruction_scope` 和该模式专用的 `recursive`；返回规范路径、
目标类型、指令目录及完整性。文本模式的 `missing_ok=true` 对真正缺失返回 typed
`workspace_file_absent`，其他错误保持错误；Coding 源码宿主需实现该契约，不能再用任意
RESOURCE_NOT_FOUND 字符串代替不存在。Coding 负责加载/重扫、批次暂停及重新规划，社区
Engine 不被强制采用同一策略。元数据不是快照或 shell 访问拦截；扫描预算和不完整结果
必须保留。详见 `CODING-INSTRUCTION-SCOPES-2026-09-14.zh.md`。

2026-09-14 File/VCS 契约补充（未验证）：当前 Coding loop32 / Nomi host17。
Wave2 的 write/patch/status/diff/stage/commit 不再暴露 open object；源码 Engine 需从 exact
schema ref 获取真实参数，不能依赖未知字段被忽略。Coding 组装拒绝开放/不透明的标准工具
schema；process 的严格 oneOf 变体保留。Patch 行参数不包含 CR/LF 或文件首 BOM，读取/搜索
行号与 patch 一致；旧 before-next-line 坐标仅在无歧义时兼容。Patch 回执增加可缺省的
source_sha256/written_sha256，表示历史输入/发布版本而非当前文件锁。新建意图保持 no-clobber，
多文件提交仍非事务。详见 `CODING-PATCH-CONTRACTS-2026-09-14.zh.md`。

2026-09-14 Patch 失败回执补充（未验证）：当前 Coding loop33 / Nomi host18。
共享 fs.patch 错误通道携带 `workspace_patch_failed` v1 的零基 request.files 发布/恢复索引；
新建目标保留不自动回滚删除。journal 结算未确认明确返回，既有 started 记录阻止同范围
新键重试。社区 Engine 必须将错误视为可能有部分效果；Coding 自己实现 turn-local
重新观察/重新规划约束，其他 Engine 可实现自己的恢复策略。观察不是事务、当前状态锁或
完整阅读/验证证明；跨回合结构化恢复未完成。详见 `CODING-PATCH-RECOVERY-2026-09-14.zh.md`。

2026-09-14 Coding 恢复状态补充（未验证）：当前 Coding loop34，Nomi 仍为 host18。
CodingEventSink 新增 `PatchRecoveryUpdated`，自定义 Coding 宿主需要永久记录事件，并向
下一回合 `with_patch_recovery` 提供经 owner 核对的状态。生产宿主独立于聊天窗口读取既有
永久执行日志；来源需匹配 exact Session/构建/Snapshot 和已结束 receipt。写前记录失败
不派发 Patch；中断后不能把旧的部分读取当作当前观察。此接口不授权解除未知效果隔离，
不恢复旧工具或命令；其他源码 Engine 可以采用自己的策略。详见
`CODING-PATCH-RESUME-2026-09-14.zh.md`。跨 Session/Fork 协调与人工解除仍需单独处理。

2026-09-14 共享 MCP SSE 补充（未验证）：当前 Coding loop35 / Nomi host19。
官方和源码集成 Engine 通过既有 Kernel 工具端口消费明确配置的 `http` / `sse`，
不新增 Engine 安装或热挂载入口。SSE 的目录发现与执行共用有界协议核心；同源路由、
无重定向/重连/重放、完整目录核对与未知效果隔离不能由 Engine 覆盖。释放本地流不等于
远端退出或回滚，stdio 仍未接入 canonical execution owner。详见
`ENGINE-MCP-LEGACY-SSE-2026-09-14.zh.md`。

2026-09-14 共享 MCP stdio 补充（未验证）：当前 Coding loop36 / Nomi host20。
`stdio` 与 HTTP/SSE 经同一冻结工具/资源准入，由平台 owner 启动受管 MCP 进程；
模型只提供工具参数，不提供 launch argv/env。返回成功要求工具响应及全树清理均成立。
最小环境不是 OS 沙箱，启动/初始化也可能有外部效果，取消/未知清理不授权自动重放。
每调用临时进程不是持久 Session，也不是打包后 Engine 挂载。详见
`ENGINE-MCP-STDIO-2026-09-14.zh.md`。

2026-09-14 Coding 搜索指令补充（未验证）：当前 Coding loop37 / Nomi host20。
Coding 要求 canonical fs.search 的单文本 JSON 契约，命中片段进入模型前通过现有
fs.read instruction_scope 与指令读取加载精确文件层级。缺少读取权限、不完整发现或
路径变化会明确暂扣片段，不影响平台原始执行记录，也不授权自动授予能力。串行后续
调用需按新规则重新规划；社区 Engine 可通过同一平台端口实现自己的上下文策略。
路径观察不是原子快照。详见 `CODING-SEARCH-INSTRUCTIONS-2026-09-14.zh.md`。

2026-09-14 MCP 网络发现补充（未验证）：当前 Coding loop38 / Nomi host21。
设置页 HTTP/SSE 与工具执行共享有界初始化/目录分页/协议处理及会话清理；不再保留
无界 HTTP 正文读取或单页目录旁路。此操作不调用 tools/call、不授予 Agent 权限，
不新增 Engine 挂载点。清理失败不显示目录成功；直接丢弃 future 仍不证明远端释放。
详见 `ENGINE-MCP-HTTP-DISCOVERY-2026-09-14.zh.md`。

2026-09-14 消费者 Binding 补充（未验证）：当前 Coding loop39 / Nomi host22。
AgentResolvedSnapshot 的可选 canonical_binding 是保存 artifacts 的引用，不是 Engine
选择或权限。源码消费者必须经应用 Session host 重新准入；底层拒绝缺少 metadata/exact
Engine 的 canonical 快照，不能借此回退 Nomi。Cron 保存引用，创建重试沿用已经创建的
exact Engine。旧无引用快照不自动迁移；协作 Attempt 的旧工具名限制/brief overlay 尚未
完成 Engine-neutral 适配。详见 `ENGINE-CONSUMER-BINDING-2026-09-14.zh.md`。

2026-09-14 协作约束补充（未验证）：当前 Coding loop40 / Nomi host23。
canonical Attempt 将 brief/step_spec 作为 user 输入，保留 Agent 固定指令；Session 的
execution_constraints 仅收窄既有能力。社区 Engine 应通过 compile_tool_plan 获取过滤后
工具表再 install_tools，使用 allows_activation 检查按需依赖；实际端口也核对完整工具绑定。
不要根据别名或自报 read-only 标签猜测授权。约束属于平台 Session，不是可热换 Engine
参数。Nomi 的原生/动态注册表执行相同上限。ReadShell 不是只读 OS 沙箱。
详见 `ENGINE-ATTEMPT-CONSTRAINTS-2026-09-14.zh.md`，旧消费者迁移和生态生命周期仍待完成。

2026-09-14 追加上下文补充（未验证）：当前 Coding loop41 / Nomi host24。
RegisteredAgentRuntime 和 EngineSessionDriver 的 supports_steering_context 默认 false；
支持方必须实现 receipt-backed 输入核对，声明不授予文件/Skill/视觉权限。
RuntimeSteerDelivery 携带文字、files 和 inject_skills；默认文字适配器拒绝非文字字段，
不能静默忽略后返回成功。Coding 仅接受原 Snapshot 已锁定 Skill 的提示，以及宿主按
既有路径、模型视觉能力和预算准备的图片。永久历史不保存图片载荷，也不重读历史路径。
前端不把交付不确定的追加输入自动转成下一轮，保存为阻止自动执行的待确认草稿。
此操作不改变 Agent 的固定指令/能力集合，也不热换或挂载 Engine。详见
`CODING-STEERING-MEDIA-2026-09-14.zh.md`。

2026-09-14 Git 生命周期历史补充（未验证）：当时 Coding loop42 / Nomi host25。
平台 local/file push owner 的工作任务与效果确认分离：worker 退出不是远端效果证明，
也不是永久回执已经提交的证明。共享宿主保留清理凭据、工作区准入路径身份及未知状态
检查；Nomi 已选 push 的 Session 同样检查清理与模型边界。执行固定 commit OID 与
已准入的本地目标，检查 libgit2 实际 push URL，不能通过配置改写绕过目标限制。
社区 Engine 不获得新的权限。Coding push 尚未开放，缺少 source-attributed Git
回执及 HTTPS/SSH 凭据 owner；Nomi push Session 的自动源输入重放也暂不支持。
详见 `ENGINE-GIT-LIFECYCLE-2026-09-14.zh.md`。

2026-09-14 Git 归属接入补充（未验证）：当前 Coding loop43 / Nomi host26。
平台共享 hosted receipts 增加 Git，新增 099 迁移源码保留旧记录；准入规范工作区
同一时刻只允许一个 pending push，跨 Session 和重启仍检查。Coding 仅对 Agent
显式选择的 `vcs.push` 开放配置好的 local/file 目标；依然无 SSH/HTTPS 凭据、force
或 ref 删除能力。Nomi source replay 改为读取永久源回合凭据；未知/取消/回执失败
不解除隔离，NotApplied 也不等于无传输。恢复审计只关闭历史，不重放 push。
此接口不改变 Agent 与 Engine 的职责，不允许打包后动态挂载 Engine。
详见 `ENGINE-GIT-ATTRIBUTION-2026-09-14.zh.md`。未执行迁移或任何实际 push。

2026-09-14 MCP 资源补充（未验证）：当前 Coding loop44 / Nomi host27。
SDK 导出 `EngineResourcePort`、`EngineResourceRead`、`EngineResourceQuery`。共享
`EngineKernelSession::start_mcp_resource` 同步注册平台任务并返回只含结果的等待者；
调用者先持有有效 turn journal，按其 Engine 的边界策略串行化激活/新输入/资源派发。
生产端口要求已选且 active 的 bundled `mcp.resource` ResourceProvider、唯一服务器
connect/read binding 和 claimed 模型操作。编译后 policy 是授权，不以模型工具名授权。
资源任务由共享 Session 清理持有；丢弃等待者不是事务取消或远端回滚。Coding 已接入，
Nomi 的既有资源入口暂未迁移。仅精确目录 URI 文本读取，有界字节分页且续页校验摘要；
不支持模板、blob、订阅或服务器主动请求授权。详见
`ENGINE-MCP-RESOURCES-2026-09-14.zh.md`。所有修改未验证，不允许打包后挂载 Engine。

2026-09-14 Nomi 资源接入补充（未验证）：当前 Coding loop45 / Nomi host28。
canonical Nomi 的 `NomiMcpResources` 适配器沿用 ToolSearch，但真实读取交给共享
平台 owner；不伪造 Coding ChatCausality 或 canonical Tool action。资源调用注册在
Nomi EngineEffectScope，按同一永久凭据约束处理取消、清理和源输入重放。该路径
禁止 bootstrap/配置文件自动连接资源服务器；按需资源身份绑定 Snapshot/资源摘要。
共享接口仍只允许单服务器文本读取，64 次/回合限制由永久凭据准入统一执行。
允许与同服务器冻结工具组合，不允许混入原生全工具代理。非 canonical Nomi 旧路径
仍独立保留。详见 `ENGINE-NOMI-RESOURCES-2026-09-14.zh.md`；实现未验证。

2026-09-14 MCP 模板补充（未验证）：当前 Coding loop46 / Nomi host29。
`EngineResourceQuery` 增加模板目录和精确模板/标量参数读取；平台在同一服务器会话
内核对模板目录，受约束展开后读取，仍由原 ResourceProvider/connect/read 授权。
Coding 与 canonical Nomi 各自保留控制循环/ToolSearch，社区编译期 Engine 复用同一
端口。复合参数、二进制和订阅不在实现范围，详见
`ENGINE-MCP-TEMPLATES-2026-09-14.zh.md`。未构建或运行验证。

2026-09-14 资源结果补充（未验证）：当前 Coding loop47 / Nomi host30。
共享资源页携带宿主生成的 `is_error` 和 failure，明确拒绝只有在有效响应/目录观察
及协议清理后才能结算。Engine 应将它作为失败，不把 Ok(page) 等同于获得资源数据；
缺失标记不可默认成功。永久上下文保留 rejected/available，旧缺失元数据不推断成功。
未知/坏协议/初始化和清理失败仍隔离，无新增自动重试或回滚授权，详见
`ENGINE-RESOURCE-OUTCOMES-2026-09-14.zh.md`。

2026-09-14 MCP 工具结果补充（未验证）：当前 Coding loop48 / Nomi host31。
MCP owner 的有效工具失败在清理后作为观察返回；Conversation 宿主必须先结算，
再投影为 `MCP_TOOL_RETURNED_FAILURE`。两个官方 Engine 和社区 Kernel 端口沿用
普通能力错误通道，不把工具失败误报为成功，也不因已返回失败而误留未知占位。
永久恢复上下文保留 tool_reported_is_error，false 不证明任务成功。初始化/目录、
坏协议和清理失败仍隔离，详见 `ENGINE-MCP-TOOL-OUTCOMES-2026-09-14.zh.md`。

2026-09-14 多服务器资源补充（未验证）：当前 Coding loop60 / Nomi host39。
此补充取代前述资源端口仅支持单服务器的限制。Agent 可绑定 1～16 台纯资源或混合
工具/资源服务器；每个工具仍只获得自身映射服务器，额外服务器须选择 mcp.resource，
纯资源绑定不继承其他服务器工具的 invoke。EngineResourceRead 新增可选 server_id：
唯一目标可省略，多目标必须显式指定冻结 ID。社区 Engine 可通过共享 Session 的
mcp_resource_server_ids 获取索引，不会因此连接服务器或获得新权限。Rust 源码
字面量和 NomiMcpResources 构造器需更新并重新编译，不提供动态 ABI 兼容。
分页摘要绑定规范化服务器、查询及结果，旧摘要不可延用；宿主核对 owner envelope
目标。任务、效果凭据、未知隔离及恢复边界不变，详见
`ENGINE-MCP-RESOURCE-SERVERS-2026-09-14.zh.md`。未运行构建或测试。

2026-09-14 Coding 工具历史回读补充（未验证）：当前 Coding loop61 / Nomi host39。
这是 Coding 内部上下文策略，不新增社区 SDK 或平台权限。search_tool_history /
read_tool_history 从有界回合内档案搜索并读取已获得的工具文本，独立于压缩窗口；
不读取任意文件或数据库、不恢复媒体/私有推理、不把历史回读计为新执行证据。
ID 只在当前回合有效，截断/淘汰不会触发原工具重放。持久化全量历史检索仍未实现。
详见 `CODING-TOOL-ARCHIVE-2026-09-14.zh.md`；未运行构建或测试。

2026-09-14 持久化历史分页补充（未验证）：Coding loop62 / Nomi host40。
公共 `EngineSessionHost::read_history_before(receipt, limit, before_operation)` 在原有
原始窗口上增加向前分页。平台核对 receipt 来源、同用户/Session 和当前 root 截止，
保留事务内先算大小、连续序号与读取预算；社区 Engine 自行解释事件及选择上下文。
这不是效果执行授权、动态挂载接口或跨会话读取入口。

Coding 的可选 `CodingHistoryPort` / `CodingTurnRequest::with_history_port` 已接生产
适配器，`load_tool_history` 单次导入一个旧回合供 search/read 检索，独立于初始模型
窗口。exact binding/来源/结构核对后原子发布，不导入合成结果，不给当前完成或
Patch 恢复增加证明。最近 64 个已加载回合去重，档案仍有界且可能淘汰；搜索按入档
倒序而非跨回合执行时间排序。无全文索引、损坏/超大日志跳过或旧截断内容恢复。
社区源码适配并重新编译打包；Nomi 只共享平台分页能力，不采用 Coding 控制循环。
详见 `CODING-PERSISTED-TOOL-HISTORY-2026-09-14.zh.md`，未运行构建或测试。

2026-09-14 MCP 复合模板补充（未验证）：Coding loop63 / Nomi host41。
此补充取代模板仅允许 scalar 的限制。EngineResourceQuery / McpResourceOperation
的模板 variables 变为 `BTreeMap<String, serde_json::Value>`，允许字符串、字符串
列表和字符串值字典；现有 JSON 字符串格式兼容，Rust 构造方须改为 JSON 值后重新
编译。两个官方 Engine 使用共享 `mcp_template_variables_schema()`；社区可复用。
平台在连接/效果前校验类型、总字节/叶字符串上限及展开后的绝对 URI，schema 不
代替授权。列表保持顺序，字典按键排序；空集合省略、空字符串保留，复合前缀拒绝。
原有 exact-server 模板目录核对、请求/分页摘要和未知效果隔离仍保留。没有增加
动态安装、挂载或自动重放，详见 `ENGINE-MCP-COMPOSITE-TEMPLATES-2026-09-14.zh.md`。

2026-09-14 结果记录补充（未验证）：Coding loop64 / Nomi host42。
Coding 串行效果结果逐项核对并经现有 sink 持久记录后才继续，不再只在批次末尾
记录；Patch 恢复清除在合法成功结果记录之后。社区 Coding host 必须维持现有
record_event “先持久化、后 UI”合同；没有增加新事件格式或要求其他 Engine 使用
相同循环。中断的部分批次不是完成证明，未知 owner 效果仍不可自动重试。MCP
模板目录新增 composite_expansion_supported/value types，标明平台复合展开支持。
详见 `CODING-INCREMENTAL-RESULTS-2026-09-14.zh.md`，未运行构建或测试。

2026-09-14 并行结果补充（未验证）：Coding loop65 / Nomi host42。
Coding 内部 codec 新增 ToolResultsOrdered，用于把并行结果的异步记录顺序与模型
proposal 排列分开。逐项结果仍经既有 sink 先持久化后发布，整批完整后才记录排列，
历史重建核对完整身份集合/原顺序/步号/唯一性；没有送达、效果顺序或清理含义。
穷举 CodingEngineEvent 的源码扩展需适配后重新编译。社区通用 Engine SDK 和
注册方式不变，Nomi 不被要求使用 Coding 循环。详见
`CODING-PARALLEL-RESULTS-2026-09-14.zh.md`，未运行构建或测试。

2026-09-14 清理异常补充（未验证）：Coding loop66 / Nomi host43。
共享 EngineEffectScope 逐 witness 捕获可展开 panic 并继续尝试后续清理；新增
guard_effect_settlement 可保护 callback 创建和轮询，但不持有任务、不证明效果
结束。观察到的 scope 失败在下一次 await 前持久保留于内存状态，重复清理不能
清除失败；进程 owner 同样保留 cancel panic 不确定状态。生产 Kernel 各 owner
和 Coding inbox 收尾分别隔离，仍由原 retained completion 保存结果并阻止无证明
终态。abort、挂起、跨启动恢复不在此保护范围。社区清理合同仍需真实 owner
witness，Engine 仍只能编译打包进入注册，详见
`ENGINE-CLEANUP-ISOLATION-2026-09-14.zh.md`。未运行构建或测试。

2026-09-14 长工具输出补充（未验证）：Coding loop67 / Nomi host43。
Coding 私有模型窗口策略按 canonical capability 摘录进程日志和 Git diff 的已知
JSON 正文字段，保留操作元数据和原始错误标志。证据和有界档案先接收原结果；
search_tool_history 增加可选精确 call_id 筛选以便回读，导入历史的 source 本身
可能已有截断，不能以档案 eof/truncated=false 证明原文完整。历史重建也应用
该派生策略，不增加事件格式、执行、存储 owner 或社区 SDK 要求，不强制 Nomi
采用 Coding 上下文算法。详见 `CODING-TOOL-CONTEXT-2026-09-14.zh.md`。
未运行构建或测试，公共授权/注册和现有日志预算不变。

2026-09-14 模型预算补充（未验证）：Coding loop68 / Nomi host43。
Coding 官方 from_limits 政策按候选模型输出交集、context/8 与 16384 限制输出，
未知能力维持 32768/4096 默认值。回合入口合并调用方更小显式额度并冻结，发送和
压缩预留保持一致；显式零值失败，不改变 reasoning 设置。源码接入者仍可构造
合法 CodingModelBudget，真实 Provider 限制由 Broker 校验；这不是动态配置或
平台授权接口。Nomi 和社区其他 Engine 的策略不受影响。详见
`CODING-MODEL-BUDGET-2026-09-14.zh.md`；未运行构建、测试或模型调用。

2026-09-14 MCP 媒体补充（未验证）：Coding loop69 / Nomi host44。
共享 EngineResourcePort 新增带默认拒绝实现的 read_image，可继续只实现 read。
输入 EngineResourceImageRead 包含 server/query/content_index 和必填原始 blob
expected_source_sha256；生产宿主额外校验 active llm.vision、primary ImageInput
和同代身份，持有远端任务并在 cleanup/receipt 后准备 typed Image。普通 read
与新持久回执只暴露安全的二进制描述，页摘要升级为 v3、不能沿用旧页摘要。
Coding read_mcp_resource 新增 format=image，单独调用且不能混入分页字段；
Nomi 本轮只共享描述，不增加像素 Tool。社区 Engine 自定何时显式请求媒体，
不可绕过平台授权或从拒绝回退成 base64 文本。源码集成后重新编译打包注册，
无动态挂载、私有二进制存储、客户端 URL 抓取或效果重放。详见
`ENGINE-MCP-MEDIA-2026-09-14.zh.md`；未运行构建或测试。

2026-09-14 Nomi MCP 媒体补充（未验证）：Coding loop70 / Nomi host45。
NomiMcpResourceInvoker 增加默认拒绝的 read_image；text-only 接入无需伪造图片结果。
生产 adapter 与 NomiMcpResources 共享 initially-unbound NomiResourceImageAuthority，
with_image_authority 仅供源码宿主在构建前安装；运行时用真实 model support/native
vision activation 一次绑定，工具无法绑定/替换。平台仍独立检查冻结视觉和服务端
授权，EngineEffectScope 持有包括 decoder 在内的工作。Nomi image 输出使用已有
ToolImage 链路而非 Coding 循环；资源结果内部 call_id 为原 Nomi scoped operation。
本补充替代上一条“Nomi 尚无像素入口”的限制，仍不增加任意媒体下载、效果重放、
运行时挂载或新 Session owner。详见 `ENGINE-NOMI-MCP-MEDIA-2026-09-14.zh.md`。
未运行构建或测试，旧 Session exact binding 不变。

2026-09-14 Coding 资源观察补充（未验证）：Coding loop71，Nomi host45 不变。
资源本地准备与端口派发分开；准备通过后先持久化工作区观察失效和失败 Patch
重读义务，再调用宿主。资源会话可能启动 stdio，不能把资源读取当作工作区未变
的证明；失效不证明真正发生 IO 或修改。重读完成的 Patch 目标遇到后续副作用也
重新失效，串行调用逐项复查；既有清理例外不变。无新 SDK/事件格式/权限，也不
自动重读、重试或验证。详见 `CODING-RESOURCE-OBSERVATIONS-2026-09-14.zh.md`。

2026-09-14 Coding 长任务上下文补充（未验证）：Coding loop72，Nomi host45 不变。
压缩的可选原文尾部最多保留连续三批/64 个调用，仍共享 32 KiB 原文预算及总输入
预算；不拆调用/结果、不跳过中间批次、保留期间用户输入和响应顺序。新增的
历史解析仍只引用已存在的观察；独立归档可缺更早前缀但不导入它。事件字段不变，
旧单批记录仍受支持，旧 Session 不热换。此策略只属于 Coding，不要求社区或
Nomi 采用同样循环/上下文算法。详见 `CODING-MULTI-BATCH-CONTEXT-2026-09-14.zh.md`。

2026-09-14 Patch 源前置条件补充（未验证）：Coding loop73 / Nomi host46。
平台 fs.patch 文件项增加可选 expected_source，取 kind=existing + sha256、
kind=absent 或兼容默认 kind=any。源码宿主构造 AgentSessionFilePatch 时新增
expected_source 字段，枚举 AgentSessionPatchSource 从 nomifun-file 导出。
所有目标准备阶段检查完整源摘要/存在性，既有发布前比较和失败回执继续生效。
共享 schema 变化进入 exact build digest，不热换旧 Session；社区 Engine 可在
自己的修改策略中使用同一 owner 前置条件，不需要采用 Coding 循环。详见
`CODING-PATCH-SOURCE-2026-09-14.zh.md`；未运行构建或测试。

2026-09-14 旧 Wrapper 隔离补充（未验证）：Coding loop74 / Nomi host47。
KernelCatalogProvider 从旧 agent-platform 迁至 agent-control-plane，相关函数返回
ControlPlaneError。public 默认走注入的 Remote operations；app 的旧外部 Wrapper
宿主仅在 legacy-codex-wrapper 特性下编译，默认桌面/服务端不请求此特性。
它是清理历史源码前的构建隔离，不是社区 Engine 注册接口，不替代 CAR-08 物理删除。
共享目录和构建声明加入两个官方 exact build 摘要。详见
ENGINE-WRAPPER-ISOLATION-2026-09-14.zh.md。

Catalog 的 channel 改变只影响后续新会话；Fork 继承父会话 exact binding，缺失构建失败，
不重新解析 channel。既有 Session 不热换引擎。是否支持受隔离的
进程外 Runtime 适配器属于后续实现，不把 Rust trait 当作稳定动态库 ABI。
# 编译期恢复扩展补充（2026-09-13，未验证）

除 factory/admission 外，宿主现在可以在组装前通过
`RuntimeEngineHost::register_restart_recovery` 为 exact family/build/digest 注册
`RegisteredEngineRestartRecovery`。这是可选扩展：未注册或证明失败时继续隔离，
不能复用 Nomi 的进程日志或 rewind 逻辑。扩展必须在 boot-frozen generation 核对后
检查自身持久化证据，允许关闭中断历史，不允许重放副作用。

官方 Coding 当前只恢复“没有命令启动”或“已有同代持久化清理证明”的中断回合。
执行中命令的清理结果未知时仍隔离。接口与实现尚未经过本轮构建/测试。
详见 [CODING-CONTROL-2026-09-13.zh.md](CODING-CONTROL-2026-09-13.zh.md)。
