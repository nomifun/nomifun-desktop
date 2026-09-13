# 开放 Runtime 接入边界

依据用户 2026-09-13 的补充要求：NomiFun 是开放平台，执行引擎必须可替换；
不能把接入方式固定为 Nomi/Coding 两个具体实现。所有开发统一在
`rf/agent-capability-platform-v2`，不 push。

## 已实现的进程内扩展接口

- `nomifun_ai_agent::AgentRuntimeControl`：消息发送、事件订阅、取消、健康状态。
- `nomifun_ai_agent::RegisteredAgentRuntime`：必需的可确认退出，以及可选控制操作。
- `nomifun_ai_agent::runtime_registry::AgentRuntimeFactory`：异步构造接口。
- `AgentRuntimeHandle::Registered(Arc<dyn RegisteredAgentRuntime>)`：通用生产句柄。
- `RuntimeEngineCatalog`：异构 descriptor/factory 注册、发现、开放 profile/channel、
  exact Build 解析和恢复验证；同一 family/build 不允许覆盖工厂或摘要。
- `CodingAgentRuntime`：Coding 的 Registered 实现；`CodingRuntimeHost` 强制宿主
  提供真实会话准备、语义记录、turn 和 Session 清理证明。

现有 Nomi factory 已改为返回同一 Registered 句柄。新增用户 Runtime 不需要
添加一个枚举分支，也不需要修改 Conversation/Remote/Automation 的调用方。
旧 `Nomi` 构造变体仅保留既有 API 兼容，不作为可扩展引擎选择器。
`AgentType` 是旧产品分类，不能拿它当 Engine family/build 标识。

一个二次开发 Runtime 实现上述两个 trait，在产品组合前注册 descriptor/factory：

```rust,ignore
let application = NomiCoreApplication::compose_with_runtime_engines(
    &environment,
    |host| host.register(custom_descriptor, Arc::new(move |options, exact_binding| {
        let ports = admitted_platform_ports.clone();
        Box::pin(async move {
            let runtime = CustomRuntime::open(options, exact_binding, ports).await?;
            Ok(Arc::new(runtime) as Arc<dyn RegisteredAgentRuntime>)
        })
    })),
).await?;
```

这是接入形状示例，`CustomRuntime` 与 `admitted_platform_ports` 由宿主提供。
它不是一个已经实现的动态安装/API，也不是一套新的 Session 存储。

## 不可省略的生命周期约定

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

剩余：Process、按需激活、Skills/MCP/MiniApp、完整 AGENTS/compaction/checkpoint、
异常进程重启证明、逐工具与多平台验收、Remote/Automation 的完整继承行为验证。
当前非 Nomi 孤儿回合不会套用 Nomi 私有恢复日志，而是保留隔离；不能自动重放。
默认引擎仍为 Nomi；Coding 是在 Agent 工作台显式配置的实现。未知构建不会静默回退。

Catalog 的 channel 改变只影响后续新会话；Fork 继承父会话 exact binding，缺失构建失败，
不重新解析 channel。既有 Session 不热换引擎。是否支持受隔离的
进程外 Runtime 适配器属于后续实现，不把 Rust trait 当作稳定动态库 ABI。
