# 多 MCP 服务器：精确资源绑定与产品接线（未验证）

任务：CAR-03 / CAR-05 / CAR-07。只在 `rf/agent-capability-platform-v2` 本地实施，
没有 commit/push。按用户要求未运行构建、测试、评测、迁移演练或 E2E。
局部 rustfmt 只用于格式化；以下均为源码实现状态，不是运行验收结论。
当前 Nomi build 为 `host7`，Coding build 为 `host2-coding-loop19`。

## 绑定方式

同一个 Agent 可以选择来自多个 HTTP MCP 服务器的冻结工具，最多 16 个服务器，
总资源选择仍受现有 32 项上限约束。服务器集合必须与 Snapshot 的 tool locks 完全一致，
不接受额外服务器、缺失服务器或重复 `(resource_kind, resource_id)`。

没有放开所有资源的多值权限。Kernel 的 `with_target_resource_bindings` 对具有 MCP
tool lock 的 capability，按 lock.server_id 从 mcp_server 资源中选择且仅选择一条；
缺失或重复即拒绝。其他资源种类、没有冻结映射的工具仍维持每种资源一个绑定。
既有 runtime authority 要求请求 binding IDs 与 capability policy 严格相等，
因此模型、Engine 或另一工具无法把服务器 B 的资源传给锁定服务器 A 的工具。

产品解析在每个服务器上只核对属于它的已选工具，而不是要求每个服务器包含整个 Agent
的工具集合。仍核对安装 owner、工具当前可用性、精确配置版本；运行时再次核对 descriptor、
schema、Snapshot lock 和资源 policy。不能将多服务器与依赖单值资源的旧代理 consumer 混用。

## 产品入口与持久状态

- AgentResourcePicker 对冻结逐工具 MCP 启用多选；旧 native MCP 保留单选。
  Wire contract 仍为 `(resource_kind, resource_id)` 数组，不接受客户端传入权限或凭据。
- 工作台测试、启动与 Agent 切换复用同一资源选择解析。
  从既有 Session binding 回显时保留全部服务器，不再按 kind 覆盖成最后一条。
- 启动页对冻结 MCP 只投影资源选择中的服务器，不再混入“所有已启用服务器”的默认选择；
  对应服务器在能力菜单中锁定。该菜单不增加工具授权，更不提供 Engine 切换。
- Session 的 exact MCP projection 逐项读取 owner、状态和配置版本，保留 ID/name 数组。
- 创建、Fork 创建、会话能力修改及 Agent 切换在持久变更前检查产品 MCP overlay 与冻结
  Snapshot 的一致性。运行时加载的检查仍保留，覆盖重启或异常持久状态。
- Engine 仍属于 Agent revision，社区 Engine 仍只能源码接入并重新打包注册；
  此改动没有引入动态 Engine 安装或 post-package mounting。

## 执行、取消与恢复

这是多个服务器的顺序调用支持，不是远端并行事务支持。产品 MCP action 保持
ExternalTransmit 分类；Nomi 工具声明不允许并发，Coding 也按非只读调用串行执行。
没有增加等待队列，避免在停止之后排队派发新的远端事务。

沿用 Session 级单 pending owner receipt：任何服务器出现未知结果或清理失败，
整个 Session 仍隔离。另一个服务器成功不构成解除隔离的证据。
每个 capability 的 canonical key 包含服务器身份，现有 operation/turn/epoch/capability
关联和 Coding 精确恢复不需要扩大 schema 或引入新 Session owner。
Nomi 仍禁止对已有远端事务的原始 source 自动重发/编辑重提，并在模型调用前读取效果观察。

更新两个 Engine 的 build 后缀与摘要输入，包含此次修改的 Kernel compiler 和产品资源/Session
接线。旧 Session 不热迁移，也不能借用新 build 的恢复证明。

## 仍待推进

- stdio、legacy SSE、resources、server-initiated lifecycle 等新 MCP 通道能力；
- MiniApp/Robot/非 function Plugin 的完整 retained effects 和持久效果凭据；
- 跨启动进程树证明、人工隔离解决、安全 checkpoint continuation；
- 动态/递归/符号链接指令作用域、非文本 Skills、Git push 效果证明；
- 独立需求语义覆盖与真实 consumer / Provider / 升级 / 跨平台验收。

本切片未修改或执行测试。特别需要后续验证多服务器 A/B 越权、缺失/重复/过期绑定、
同名工具和同名服务器、按需激活、启动/切换/Fork/UI 回显及远端未知效果隔离。
不能拿此前单服务器或历史测试通过记录证明这些行为。
