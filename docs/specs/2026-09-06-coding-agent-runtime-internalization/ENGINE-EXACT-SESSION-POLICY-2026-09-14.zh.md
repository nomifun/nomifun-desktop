# 精确构建的私有 Session 兼容策略（slice87）

日期：2026-09-14。仅源码实施，未运行测试、构建、服务、模型调用或迁移。
分支 rf/agent-capability-platform-v2；不提交、不 push。

## 修复的缺口

原先 Registry 给 family=nomifun.nomi 的任何构建注入 Nomi Plugin Tool scope，
Conversation 也据此选择 Nomi 私有恢复日志和冷清理路径。但源码目录允许同 family
的其他构建，因此 family 不能证明实现使用同一种 Session codec。

## 实现

- `RuntimeEngineAdmission::uses_nomi_session(binding)` 默认 false。只有实现确实
  复用 Nomi Plugin scope 与私有持久化/恢复协议时，才能在源码注册策略中开启。
  官方 Nomi 开启；Coding、通用 RuntimeEngineSupport 与现有社区 SDK 默认关闭。
- Catalog 核对 family/build/digest/host contract/profile，再读取该构建的策略。
  缺失、漂移、不支持的构建不获得 Nomi 私有访问权；这不替代 `open` 的严格准入。
- 生产 Registry 通过源码组装的 Weak Host resolver 读取冻结目录；没有 resolver 的
  自定义 Registry 默认拒绝 bound Engine 的 Nomi 兼容声明。旧的无绑定 Nomi 会话
  保留明确的兼容路径，不把缺失/错误绑定退回到旧路径。
- Plugin scope 与 Conversation 的重启恢复、reset、冷 clear_context 和清空消息
  的 Nomi 私有分支使用同一策略，不再按 family 名称选恢复协议。
- 已完成构建的 runtime 必须与注册策略声明一致。Registry 在保留 exact slot 后
  检查 `uses_nomi_recovery()`；不一致则执行现有 exact-slot teardown，清理失败
  保留 quarantine/lease。不能在工厂中直接丢弃 runtime 并假装普通无资源构建失败。
- 未安装的旧构建不会误用新版 Nomi codec，仍可走独立注册的 exact-build 重启
  恢复钩子。没有该钩子/证明时仍隔离；不自动升级、切 Engine 或重放工具。

## 边界

这是可信源码集成契约，不是对恶意原生代码的沙箱。社区实现仍拥有自己的循环、
规划和上下文策略；平台 Session owner 不变。没有添加用户 JSON 兼容开关、
打包后安装接口或全局 Engine 切换入口。

本片没有实现社区引擎通用的冷上下文清理协议、旧构建 codec 迁移、Git 网络凭据
owner、MCP 更广泛的服务端主动消息生态或 Vertex 生产路由。既有非 Nomi reset/
消息删除行为也不因此获得其私有存储已清理的证明。不能视为整体能力完成。

官方源码标识更新为 Coding host2-coding-loop87 / Nomi host57；相关策略、Registry、
Conversation 和组装源码均在现有 digest 输入内。未运行验证，不声明行为通过。
