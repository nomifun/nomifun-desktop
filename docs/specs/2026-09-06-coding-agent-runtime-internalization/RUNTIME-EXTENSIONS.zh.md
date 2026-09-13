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

一个二次开发 Runtime 实现上述两个 trait，工厂返回 Registered 句柄：

```rust,ignore
let factory: AgentRuntimeFactory = Arc::new(move |options| {
    let ports = admitted_platform_ports.clone();
    Box::pin(async move {
        let runtime = CustomRuntime::open(options, ports).await?;
        Ok(AgentRuntimeHandle::Registered(Arc::new(runtime)))
    })
});
let runtimes = InMemoryAgentRuntimeRegistry::new(factory);
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

目前该桥接的测试使用 fixture host，不构成默认生产 host 实现。对未具备真实
工具/进程 cleanup 证明的宿主，不能以空实现返回成功。

## 尚未完成的引擎平台接线

开放 trait/factory 不等于完整引擎平台。后续仍须完成：

- 将已实现的异构目录安装到默认生产组合根；
- 在宿主唯一 Session 创建/Fork 事务中冻结 Engine binding，恢复时 exact 验证；
- Coding 实现经通用工厂接入默认产品路由，不能只接未挂载的 Fresh-v4 路由；
- Coding 的事件、Context、File/Process/VCS 和取消映射；
- 用户选择入口与默认路由 E2E。

Catalog 的改变只影响新建/Fork；既有 Session 不热换引擎。是否支持受隔离的
进程外 Runtime 适配器属于后续实现，不把 Rust trait 当作稳定动态库 ABI。
