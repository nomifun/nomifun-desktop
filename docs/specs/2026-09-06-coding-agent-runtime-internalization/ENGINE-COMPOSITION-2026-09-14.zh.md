# 多 Engine 组装入口补齐（slice84，未验证）

分支为 `rf/agent-capability-platform-v2`，无 commit/push；未运行构建、测试、服务、
模型调用或迁移。本次补齐源码组合入口，不改变 Agent/Engine/平台职责。

## 缺口与实现

底层 RuntimeEngineCatalog 原本支持渠道，RuntimeEngineHost 却仅允许社区声明构建。
新增 `RuntimeEngineHost::register_channel(family, channel, build)`，复用底层标识符、
同 family 已注册目标检查；只能指向已在本轮组装注册的扩展构建，不接受路径或代码。

注册锁内同时检查组装状态；构建与渠道使用同一个暂存 Catalog。安装时克隆暂存目录，
添加官方构建和 stable 渠道，全部成功后才发布 OnceLock。重复渠道（含相同目标）
拒绝；官方 nomifun.nomi/nomifun.coding 的 stable 保留。自定义 family 可声明自己的
stable/canary，官方 family 的源码扩展构建可声明其他渠道，不能覆盖默认稳定构建。
底层 `set_channel` 仍供持有独占可变目录的组合代码使用；产品 Host 不暴露热修改入口。
Catalog 克隆只复制描述和 Arc 工厂/准入，不创建 Engine、Session 或工具权限。

独立 Evidence 示例注册构建后声明 stable；不加入默认产品的两个官方预置。
工作台仍列出精确构建/profile，预先编写的 Agent 配置可使用渠道。新 Session 按原链
解析并持久化 exact binding；已有 Session 与 Fork 不重新解析渠道，不静默迁移。

桌面此前没有独立服务端已有的源码注册回调。新增
`DesktopServer::start_with_runtime_engines(..., register)`，保留原桌面启动路径，
在服务与 boot authority 就绪后、finalize/router/监听服务前调用一次。默认两个入口
传入空回调。回调出错进入原 `cleanup_start_failure`，保留类型化清理结果及必要的
keep-alive，而不是丢弃资源所有权。注册回调不应启动任务或获得平台外部资源。
没有新增 IPC、配置文件装载、动态库、打包后挂载或第二套 Session owner。

## 源码身份与边界

官方源码标识前进到 Coding `host2-coding-loop84` / Nomi `host54`，摘要纳入共享
Catalog 和桌面组合代码，Coding 也显式纳入 RuntimeEngineHost。并未改 Coding 循环
算法或增加一次模型/工具请求；版本前进反映宿主组装行为变更，不代表质量提升证据。

尚缺渠道重复/目标缺失/关闭注册/并发组装、桌面回调失败清理、Agent 保存与 Session
恢复/Fork、社区运行和跨平台构建证据。按用户要求没有执行这些验证，不能据此宣告
整体 Engine 能力完成。网络 Git 凭据 owner、MCP 其他生命周期、Vertex 配置合同及
无清理证明的异常恢复等原有边界也未因本次组装接线而解除。
