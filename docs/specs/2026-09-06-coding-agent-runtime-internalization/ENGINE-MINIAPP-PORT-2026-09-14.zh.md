# MiniApp 共享生产工具端口与 Coding 接入

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`。
源码实现；未构建、测试、运行数据库迁移或请求真实模型／Service。未 commit/push。
当前标识：Coding `host2-coding-loop29`，Nomi `host14`。

## 职责与接线

MiniApp 保持独立 `ResolvedMiniAppCapability`，不伪装为 PluginMount，也不进入 Engine 内部
私有注册表。Agent revision／Snapshot 冻结 Active Release、epoch、Catalog digest、动作及
allowlist；真实 `PluginRuntimeM1ApplicationService` 继续负责派发前的当前授权／release 核对。

`nomifun-engine-core::compile_engine_tool_plan` 现在能映射 MiniApp function action：
校验准确 Snapshot、active set、canonical schema 内容摘要和 authority policy；binding 的
capability digest 覆盖完整冻结 MiniApp 描述。所有 MiniApp action 都按串行外部效果处理，
不依靠 manifest 的只读声明宣称远端效果可逆。低层 Kernel invoker 不直接执行 MiniApp。

`EngineKernelSession::miniapp_tool_plan()` 通过平台 owner 读取准确版本的 schema，缓存冻结
工具计划。该计划含初始及 on-demand 动作，仅是编译预览；不能直接全量交给模型。安装时
既重新编译 binding，又要求 MiniApp 部分与平台实际解析的 schema/name/definition 一致。
合并工具面使用 `EngineToolPlan::merged`，名称冲突报错，不覆盖先前映射。

`EngineKernelSession::install_tools` 在现有唯一 `EngineToolHost` 内装配复合 invoker：
普通工具仍走原 Kernel；MiniApp 走共享 `MiniAppOwner`。调用再次检查 Session、principal、
Snapshot、live active generation 和完整 binding。任务保留、互斥调度、派发及结果日志均复用
现有宿主；社区 Engine 不获得可脱离这些约束直接执行的 Service future。

Nomi 也改为消费 `MiniAppOwner::invoke`，不再复制凭据状态判定／Service 调用代码。
统一 bridge call ID 使用 `platform-miniapp:<operation>`；由新 exact build 承载，未修改旧历史。

## 初始与按需选择

共享 `SessionCapabilityState` 现在将初始 MiniApp 加入初始 active set。此前只纳入普通
capability，而 MiniApp 的 on-demand plan／紧凑索引已经由 compiler 产生，这会导致初始
MiniApp 从共享工具投影中消失。Nomi 原有工具激活机制不因此被替换。

Coding 在常规工具计划上合入 MiniApp，并按同一 active set 过滤。即使只配置 MiniApp
on-demand 动作，也启用原有 search/activate 端口，继续 durable-before-memory 激活和 exact
build 恢复，不用 UI 选择或模型字段直接激活。公共 RuntimeEngineSupport 的 on_demand
开关现在同样约束 MiniApp on-demand，避免社区引擎声明不支持却被放行。

Coding 准入开启 MiniApp，但仍限定 function actions、无需额外 Service resource kinds 的
当前产品能力。声明额外资源的 MiniApp 继续拒绝，不能因为工具可见就假定其 resource port
已经实现。预算包括总选中能力／动作、描述、单个 schema 和冻结描述；不支持 Hidden／CodeMode
动作、重复 action ID 或无允许 function action 的能力。

## 效果、模型与恢复

沿用 migration 098 的 exact-turn pending→returned/rejected 规则，不增加第二套 Session 表。
receipt 先于 Service 调用，结果和 receipt 写入发生在保留任务内。未知保持 pending；Service
reply 不证明物理静止、业务后台活动停止或 Service lease 退出。

- 共享复合 invoker 在所有工具 lane 前检查 hosted pending，不允许改用其他工具绕过未知。
- 公共 journal 的模型 operation claim 在同一 SQL admission 内检查 hosted pending，覆盖
  Coding 和使用该 journal 的社区模型端口，不只在 Session 装配时检查。
- 公共资源清理先收敛任务，再检查 hosted witness；按需激活也检查已知状态。
- `hosted_effect_context()` 提供独立于 transcript 的有限历史。Coding 每个 accepted turn
  注入该上下文；当前回合的新结果仍通过工具消息／原有压缩策略管理。社区 Engine 自己选择
  表达方式，但不能省略有意义的效果历史后以“没有聊天记录”为由重复调用。

Coding 启动恢复新增 MiniApp receipt 对账：同 turn/epoch、domain=miniapp、returned/rejected
必须匹配 ToolStarted 和 host_tool_dispatch 的 operation/call/capability/action 关系；悬空
receipt、重复派发或 pending 拒绝恢复。不调用 Service，不重放效果，只闭合中断历史。
没有足够 owner receipt 来识别／证明的 MiniApp 意图仍保守隔离；未实现通用人工解隔离。

## 社区编译期用法

以下是已经装配公共 Session host 后的接口组合示意，`exposures` 与 `policy` 由社区驱动提供：

```rust,ignore
let resources = host.open_kernel_session(&admitted)?;
let miniapps = resources.miniapp_tool_plan().await?;
let plan = resources.compile_tool_plan(exposures)?.merged(&miniapps)?;
let tools = resources.install_tools(plan.clone(), policy)?;
let active = resources.active_state().snapshot()?;
let visible = plan.for_active_capabilities(&active.active);
let effect_history = resources.hosted_effect_context().await?;
```

仍需 host-issued turn receipt、journal、准确模型／工具 causality、durable activation 和
清理协议；上述代码不是独立可执行样例，也没有运行证据。无需包装 Coding 的规划循环，
更不允许发布后挂载 Engine。

## 剩余边界

本切片完成 MiniApp function 工具路径，不是完整生态生命周期：Robot 的 Coding／社区
生产工具入口、bootstrap／on-demand 生命周期与 context lease 的完整任务归属、MiniApp
额外资源、其他 MCP 协议、远端 VCS 授权和人工恢复仍待实现。真实 Provider、Service 与
跨平台行为未验证。整体 CAR 继续 in_progress。
