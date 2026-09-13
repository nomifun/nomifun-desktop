# 主重构分支本地合入审查

日期：2026-09-13。此记录只描述本地工作，不代表远程或发布验收。

## 结论

隔离引擎可以合入，但原交接方案不能原样用于当前桌面生产链。
代码已取入并修复当前合同兼容问题；**尚未完成生产执行引擎嵌入**。
没有新增 UI 引擎选项，没有切换已有 Session，没有删除旧引擎，没有 push。

## 合入范围

- 目标：本地 `rf/agent-capability-platform-v2`。
- 目标起点：`08caa20d7`。
- 源：已有远程跟踪引用 `origin/car/coding-engine`，tip `ef5e53860`。
- 顺序取入：`b652fa29c`、`c8b019389`、`ba1bc4aa6`、`0e33dbed5`、
  `9dc20a346`、`f3ff31b4b`、`449e06110`、`ef5e53860`。
- 未取入源分支旧基线 `6a2a94bd1` 的一期文档修改。三方预检显示该修改与
  目标分支的 05/06 一期文档冲突；取入 CAR 独立提交避免回退这些文档。
- Cargo 按当前 workspace 将新 crate 的 lockfile 版本从 0.7.4 对齐到 0.7.6，
  保留主重构分支已有依赖。
- 后续本地实现提交 `6d87dc232`：开放 Runtime 接口、通用目录、Coding adapter、
  Broker/turn 取消与主分支合同兼容修复。

## 已实现的本地适配

1. Kernel 测试使用当前 `CapabilitySelection` 和 `AgentPresetRevisionPayload`；
   Revision digest 覆盖真实 materialized contribution lock。
2. Workspace 通过 `with_target_resource_bindings` 绑定目标，不重新放回 Preset。
3. `ChatBrokerPort::open_chat_stream_cancellable` 接收进程内取消 token，
   不改变 JSON 模型请求合同。不支持该合同的自定义 Broker 明确拒绝。
4. Broker 自己持有请求生命周期；取消和 stream drop 能释放 route preparation、
   credential acquisition、provider open、stream 消费和发送背压中的 future。
   每个请求使用 child token；释放一个 stream 不会取消父 Session。
5. Coding model adapter 调用原生取消入口，不再依赖自己的转发 wrapper。
6. 生产 HTTP adapter 的凭据 guard 随 provider-opening future 释放，包括取消和
   提前返回错误。此处的“取消”指本机请求/响应资源被释放，不承诺远端 Provider
   已停止计费或服务端计算。
7. `RegisteredAgentRuntime` 提供开放生产句柄和必需的确认退出合同；Nomi factory
   使用同一接口。通用 `RuntimeEngineCatalog` 支持任意 family/profile/channel、
   immutable Build、协议兼容检查和 exact binding；不允许重复注册覆盖实现。
8. `CodingAgentRuntime` 经同一目录/registry 构造，投影文本、推理和工具事件，
   处理 prepare/model/cleanup 阶段取消、panic、single-flight 和失败退出隔离。
   它依赖显式 `CodingRuntimeHost`；目前只有测试宿主，尚无默认产品宿主接线。
9. Coding turn 使用自动释放准入的 guard；future 被 drop/abort 时取消 broker
   child token，不取消父 Session。终态发布等待 host 的工具/进程退出证明。

## CAR-07 必须适配的架构事实

原交接指定：

```text
AgentPlatform → AgentSessionStore/SessionEvent → selected engine
```

当前默认产品实际是：

```text
Desktop/Web/Remote/Automation
→ NomiCoreSessionOwner
→ ConversationService + AgentRuntimeRegistry
→ AgentRuntimeHandle::Registered（当前默认实现仍为 Nomi）
```

证据：

- `crates/backend/nomifun-app/src/router/state.rs` 的 `build_module_states`
  创建 `NomiCoreSessionOwner`，使用现有 Conversation service 和 runtime registry。
- `crates/backend/nomifun-app/src/services.rs` 用 `build_agent_factory` 创建生产
  `InMemoryAgentRuntimeRegistry`，持有现有 runtime lifecycle 和恢复机制。
- 合入前 `runtime_handle.rs` 的生产枚举只有 Nomi；现已增加通用 Registered
  入口。测试 Mock 不承担生产扩展合同。
- `crates/backend/nomifun-app/tests/nomi_core_route_gap.rs` 明确要求默认路由
  不挂载 Fresh-v4/Codex Session/Remote adapter，也不让投影创建第二套存储。
- `crates/backend/nomifun-app/src/router/agent_platform_host.rs` 的
  `initialize_platform` 仍组合旧 Codex supervisor 与独立 SessionStore。

因此，只修改 AgentPlatform 能得到一条非默认主链，不能证明桌面嵌入。
直接切换到它则涉及 Session 持久事实、Remote、Automation 和已有恢复流程的
整体迁移，不能作为一次引擎 adapter 接线暗中实施。

## 当前实施方向（CAR-D-019）

以当前生产 Nomi-core owner 为唯一事实源，通过开放 runtime factory/handle
接入 Coding 和用户二次开发引擎；CAR-D-006 已明确 SessionEvent 的落地约定，
不能额外挂一个 AgentSessionStore 同时写两份事实。保留旧引擎和已有 Session
行为。用户的开放平台要求不是限制为 Nomi/Coding 两种后端。

以下产品接线尚未完成，每片都需要真实默认路由的行为测试：

1. 在当前 Session 创建事务中持久化 exact Engine identity；旧 Session 显式映射
   Legacy，禁止把可变 channel 放进执行时 resolver。明确 fork 与恢复语义。
2. 将已完成的开放目录和 Coding adapter 安装到默认组合根，复用现有 admission、
   generation、cancel/teardown 和幂等消息入口，不创建第二套 coordinator。
3. Broker causality gate、route 和历史由当前 Session owner 提供；不使用
   Fresh-v4 未挂载 Session 的假 authority，也不直接复制旧模型 provider client。
4. Kernel 从当前生产编译结果接收 capability/target resource/owner，File、
   Process、VCS 经已有 Wave2 owner；补 owner 的真实取消和 cleanup 证明。
5. 将 Coding 语义输出投影到当前唯一持久消息/事件链；Context、compaction、
   checkpoint 都从该链重建，不创建引擎私有 rollout 历史。
6. 新建/Fork 的 UI 引擎选择、exact Build 展示、unavailable 错误，以及
   Desktop/Web/Remote/Automation 的公共路由验证。未经灰度验收不删除旧引擎。

本次不迁移整个产品到旧 AgentPlatform/SessionEvent，也不通过恢复旧路由
宣称完成嵌入。当前缺口属于未完成实现，不是等待用户再次确认的阻塞。

## 验证记录

验证结果以 `STATUS.zh.md` 和 `TASK-MANIFEST.json` 中本次 local integration
记录为准。核心、Broker、开放 Runtime、Coding adapter 和应用 Broker 定向测试通过；
默认路由回归为 36 通过、2 失败，失败断言和未运行旧 checkout 基线的限制均已记录。
未执行真实付费模型请求、桌面 UI E2E、macOS/Linux 或发布构建。
由于生产接线尚未完成，本记录不宣称 Stable/Canary 产品验收或整体开发完成。
