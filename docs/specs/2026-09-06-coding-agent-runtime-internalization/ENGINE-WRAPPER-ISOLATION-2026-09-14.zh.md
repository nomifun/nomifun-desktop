# Slice74：默认产品构建隔离旧 Wrapper（未验证）

继续 CAR-08 的第一步，不是 CAR-08 删除验收通过。分支为
rf/agent-capability-platform-v2；没有构建、测试、服务启动、模型调用、迁移、commit 或 push。

## 本次实现

- 将 KernelCatalogProvider 和三项 Catalog materialization 函数从旧
  nomifun-agent-platform 移至 nomifun-agent-control-plane/src/kernel_catalog.rs。
  新 Nomi 目录和 Plugin publication 直接引用 control-plane，不再为读取能力目录
  依赖具体 Session executor。保持来源、可用性、MiniApp 冲突与摘要校验；错误类型
  改为 ControlPlaneError。旧平台重导出名称，不保留第二份目录实现。
- nomifun-public 默认只编译注入 CanonicalRemoteOperations 的传输路径；
  AgentPlatform / AgentSession 旧实现依赖改为 optional，并由非默认
  legacy-agent-platform 特性统一包含旧构造器、适配器及其专属测试。
- nomifun-app 的 nomifun-agent-platform 与 nomifun-codex-runtime 改为 optional。
  非默认 legacy-codex-wrapper 特性集中开启这两个依赖和 public 旧适配器。
  旧 Fresh-v4 bootstrap、HTTP/Remote REST 路由、sidecar artifact resolver、
  RemoteRuntimeCoordinator、旧模型 claim bridge 及桌面启动/清理入口均加同一边界。
- Nomi 的 Wave1 owner、Remote detached task/permit、真实 Provider broker、
  Conversation owner 与 Engine 注册入口没有改成旧平台，也没有新建 Session 库。
  apps/desktop 仅请求 computer-use/browser-use，apps/web 不请求旧特性。
- 原 agent_platform_e2e / remote_rest_e2e 保留为显式旧特性的 fixtures；
  startup_smoke 仅旧宿主测试加特性并改正名称，当前桌面测试继续保留。
  混合在 agent_platform_host 的历史单元测试暂由旧特性保护，后续应把共享 Wave1
  owner 测试迁至中立模块，不能把这些未执行或未迁移的测试计为新引擎覆盖。

## 边界与余项

这是源码依赖隔离，不是 cargo tree、编译或制品扫描结果。默认产品 package 的
依赖声明不再主动选择 Wrapper；显式旧特性、workspace 全成员构建或其他依赖的
特性合并仍可能包含它。Cargo.lock 保留 optional 包是正常的，不能据此断言产物包含
或不包含旧引擎。历史源码仍在 workspace，CAR-08 必须继续物理清理，不能用一个
兼容开关替代删除目标。

尚未删除 nomifun-codex-runtime、旧 runtime_chat_bridge/Fresh-v4 源码和协议产物，
macOS 打包脚本仍有 --with-codex-runtime 支路，历史 gate/release fixtures 仍有旧假设。
未运行这些工具，未制造三平台交付证据。共享 Wave1/Remote 代码拆分、旧 fixtures
迁移和删除应在保留当前资源 owner 的前提下继续。

Engine 仍为源码集成并重新编译打包注册；此临时历史特性不是社区 Engine 安装接口，
没有增加 UI 引擎切换入口或打包后挂载能力。

## Exact build

当前 Coding 标识为 host2-coding-loop74，Nomi 为 host47；两个摘要都加入 app/public
Cargo 声明和迁出的 kernel_catalog.rs。这里编号变化代表共享宿主/目录实现变化，
没有声称新增 Coding 推理算法。已有 exact Session 不自动升级、不回退至其他 build。

仅对新目录文件和 public Rust 文件做 scoped rustfmt；未执行类型检查或行为验证。
整体多 Engine 目标保持未完成。
