# 旧 Wrapper 宿主与依赖物理删除

日期：2026-09-14。Slice76，CAR-08 的源码实施步骤，**未经构建或运行验证**。
工作树：rf/agent-capability-platform-v2；未 commit/push，未更改旧阶段文档。

## 本次完成的源码变更

- 删除 nomifun-agent-platform、nomifun-codex-runtime 两个 crate 的源码、清单和
  旧测试/fixture，以及 workspace/app/public 清单和锁文件中的对应直接引用。
  不再保留 legacy-codex-wrapper / legacy-agent-platform 特性。
- 删除 RuntimeStartTurnBrokerBridge 所在的旧执行链、Fresh-v4 组合根、Runtime
  制品加载、旧 AgentPlatform/Remote REST/system 路由和专用集成测试。
- 桌面与 HTTP 入口只保留现有 NomiCoreApplication 产品宿主；取消旧桌面宿主
  选择枚举，不影响 Agent 工作台的 Engine 选择。Remote MCP 保留注入操作、
  安装身份认证和传输生命周期，移除旧平台适配器及 sidecar admission。
- remote_runtime.rs 只保留当前产品的远程任务租约、限流、取消和关闭机制；
  chat_broker_host.rs 保留真实 Provider/模型端口，删除旧平台的 claim-store 适配。

## 明确保留的共享能力与测试源码

知识库/项目记忆/Companion 适配器迁至 router/agent_wave1_host.rs；原产品
nomi_core_builtins 改为引用此处。仍由平台和 Kernel 拥有领域能力，而非 Coding
或任何 Engine 私有实现；没有切换 Conversation 数据所有者。

13 个共享 Wave1 测试迁至 agent_wave1_host_tests.rs，取消旧兼容特性限制：
知识库授权与路径边界、持久 Companion 调用、项目记忆幂等/CAS/隔离/容量/损坏
和版本拒绝，以及错误信息隐藏。目录的 MCP 来源与 MiniApp 发布替换两个测试
迁至 control-plane/kernel_catalog_tests.rs；gate-plugin-n1 的四个目录用例入口
改为调用迁移后的 control-plane 包，保留原测试过滤器。已有模型 route 查找测试不再依赖
旧 app 宿主的开池 helper。以上仅为测试源码迁移，**没有运行或通过声明**。

与被删除 Fresh-v4 AgentPlatform 实例绑定的旧测试一并删除，包括旧 SQLite
platform restart 场景；它没有被冒称为当前 Conversation 恢复覆盖。独立
Kernel 重建/CAS 和真实 Companion owner 测试源码保留。旧测试中的外部 Wrapper
和旧宿主假设不能成为当前 Engine 验收证据。

原 Conversation owner、当前 legacy_conversation_port 能力端口、AgentSession
共享合同/存储库、Fresh-v4 根维护设施均未随旧执行器删除；这些仍有其他调用方。

## 构建身份

Coding：host2-coding-loop76；Nomi：host48。两个摘要都加入迁出的 Wave1、
Companion/receipt、builtin 组合、共享远程任务与公共传输实现。既有 exact build
绑定不自动更新、不通过热换或 fallback 继续旧 Session；需要当前构建的新 Session。
这里只更新源码标识，没有生成或检验二进制。

## CAR-08 仍未完成

1. contracts/runtime 下的 hello/native_action/session_dispose 等旧线协议、
   引用的导出/fixture/生成索引和旧 inventory/deletion 清单仍待按真实依赖拆除。
   state.rs 当前仍读取 CodingRuntimeFeatureInventoryPayload；该类型的校验还
   耦合旧调查 SHA、两种 profile 和八个 RPC 方法。下一步必须先解耦平台的能力
   feature 清单和旧 sidecar 发布合同，再同步生成器/摘要引用，不能只删 JSON。
   这不是当前 Coding 源码参考基线变更，现行 Codex 参考基线保持原值。
   当前 Engine 使用的共享合同不能盲删。
2. gate-agent-v2、macOS native 检查仍有旧 crate/sidecar 假设，
   需更新或退役，不能将未运行的旧门禁当作当前通过证据。
3. 当前 Cargo 图、锁文件一致性、共享测试迁移、默认/可选功能组合、真实 Session
   与平台制品均未验证。锁文件仅按依赖删除做定点编辑，未运行 Cargo 重生成。

只做源码阅读/编辑与限定文件格式化；没有构建、测试、启动服务、调用模型、
执行迁移、发布、读取凭据或 push。其他 CAR 恢复/生态边界详见 readiness 清单。
