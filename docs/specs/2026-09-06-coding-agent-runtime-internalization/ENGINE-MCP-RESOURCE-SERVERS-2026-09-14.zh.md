# 多服务器 MCP 资源接入（slice60，未验证）

## 目标与实现

原先冻结 MCP 工具支持多服务器，但 `mcp.resource` 仍要求整个 Agent 只有一台
服务器，导致资源读取无法与多服务器工具组合。本切片解除这项组合限制，不增加
Engine 注册/加载机制，也不改变 Agent 选择 Engine、Session 固定 exact build 的关系。

- Agent 可以选择纯资源服务器，或冻结工具服务器与额外资源服务器的组合，总数
  1～16。所有工具映射的服务器必须存在；没有 `mcp.resource` 时不允许额外服务器。
- `EngineResourceRead.server_id` 是冻结产品服务器 ID，不接受 URL、配置或凭据。
  单服务器可省略，多服务器必须显式指定；未知或含糊目标在远端派发前拒绝。
- 共享宿主将省略的唯一目标规范化，再计算请求摘要、记入派发及调用原 MCP owner。
  Nomi 也在能力激活前拒绝未知/含糊目标，不默认取第一项。
- Coding 获得冻结服务器 ID 索引；Nomi 资源工具通过 schema 枚举可选 ID，多服务器
  时将该字段设为必填。社区编译期 Engine 可使用共享索引方法和同一个资源端口。
- 每个工具 policy 仍只指向自己的 exact server。纯资源服务器的 typed binding
  只保留该服务器对应能力需要的操作，不继承其他服务器工具的 `invoke`。
- Kernel 的多资源集合例外限于 bundled/platform-builtin `mcp.resource` 及其
  connect/OAuth 依赖。其他未映射消费者仍要求单服务器；不开放原生全工具代理组合。
- Agent 工作台资源选择器沿用最多 16 台的入口，更新中英文说明；不是新增全局
  Engine 切换页，也不允许现有 Session 热换 Engine。

## 分页与所有权

每页重复输出规范化 `server_id`。摘要基于版本标签、服务器 ID、原始查询及 owner
结果序列化文本；相同资源内容来自不同服务器/查询时不能拼接续页。投影前核对
owner envelope 的服务器和操作确实对应请求，不信任嵌套远端数据中的同名字段。
每次请求仍重新观察服务器；此机制不提供远端快照事务或缓存。

平台仍持有连接、OAuth/stdio、任务、持久化效果凭据和清理生命周期。调用返回或
工具失败不等于无副作用、回滚或可自动重试。未知效果仍隔离整个 Session；恢复
仍要求原有 exact build、boot generation、连续日志及 owner 凭据，不新增自动重放。
本次不增加数据库迁移，也不声称恢复凭据新增了独立服务器列。

## 源码与兼容性

主要修改：`nomifun-engine-core/context_resource.rs`、Coding `remote_resources.rs`、
Nomi `nomi_resources.rs`、共享 `engine_mcp_resources.rs`、Nomi 资源适配器、Kernel
compiler、产品资源 resolver/catalog、官方 descriptor 与 Agent 资源选择器。

JSON 旧请求省略 `server_id` 只在唯一目标时兼容；Rust 结构体字面量需补充字段，
`NomiMcpResources::new` 需传冻结 ID 集合，社区源码须随应用重新编译。这里不是
稳定动态库 ABI。分页摘要升级，旧摘要不能延用；旧 Session 不重解析为新构建。

Coding：`host2-coding-loop60`；Nomi：`host39`。

## 尚未完成与本轮边界

这是资源组合能力的源码实现，不是全部 CAR 完成。复合模板变量、二进制资源、
订阅、授权的服务器主动请求生命周期仍未实现。Git 网络凭据 owner、无清理凭据
的跨重启进程处理、跨 Session/Fork 工作区协调、MiniApp 非工具生命周期和部分
生态消费者仍有缺口；不能用放宽权限或自动重放来填补。

按用户要求，没有构建、测试、E2E、服务启动、模型调用或迁移执行；只做源码检查
与局部格式化。本切片未经运行验证，历史测试结果不覆盖它。无 commit/push，全部
保留在本地 `rf/agent-capability-platform-v2`。
