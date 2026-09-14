# 公共工具端口与生产 Session 事实接入

分支：`rf/agent-capability-platform-v2`。本地实现，未提交、未 push。
按用户要求未运行构建、测试、评测或桌面验证；仅阅读源码和对小文件格式化。
Coding build 后缀为 `host2-coding-loop8`。不将公共 SDK 或 CAR 整体标记完成。

后续公共日志/原始历史与 Coding 持久结算进度见
[ENGINE-JOURNAL-2026-09-14.zh.md](ENGINE-JOURNAL-2026-09-14.zh.md)。下文缺口保留本切片时点。

## 独立的 engine-core

新增 `crates/backend/nomifun-engine-core`，不依赖 Coding 或 AI-agent：

- 工具映射、计划、调用、结果、副作用分类和结构化端口错误；
- `compile_engine_tool_plan`：从固定 Snapshot、明确 exposures 和活跃代际编译；
- `KernelEngineToolInvoker`：复用唯一 Kernel 权限/执行入口，不维护第二套能力目录；
- 重导出 Broker 模型端口，执行循环、规划和上下文策略不进入公共工具层。

Coding 的原类型名成为公共类型的兼容导出，Kernel adapter 委托公共实现并转换错误。
原工具契约测试移至公共 crate，Coding adapter 的兼容测试保留；均未执行。
Cargo workspace、相关依赖和 lockfile 同步新增本地 crate；未引入外部包或 Codex path 依赖。

公共 Kernel 端口新增 `for_session`，固定 Session ID、owner、Snapshot 和完整已选工具映射。
生产 Coding 使用它而非自由 scope 的低层构造器。每次调用须与已准入的工具映射一致，
并由 Kernel 复核 live active generation、资源绑定与 canonical action。
完整已选映射可以包含 on-demand 工具，但不因此激活工具。

调用入口补齐 model call 名称/参数边界检查，以及 action presentation、effect class、
parallel-safe 与 canonical action 一致性检查。不能把写操作重标成可并行只读。
工具已经派发后不再因 cancellation token 直接丢弃 Kernel future；应用仍必须通过托管任务
保留调用并持久化结算。该端口本身不等于副作用持久回执或进程树退出证明。

## 当前产品的公共 Session 入口

`nomifun_app` 公共 facade 导出：

- `EngineSessionHost::resolve`：通过现有 Conversation owner 与 control plane，核对
  canonical 用户/Session、exact engine binding、Agent revision、Snapshot、actor 和注册准入策略。
- `AdmittedEngineSession`：构造器不公开，只提供上述已解析事实的只读访问。
- `read_turn_receipt` / `EngineTurnReceipt`：读取现有 accepted turn receipt，不新增 receipt；
  校验 running Session、消息根、owner、operation、admission epoch、当前绑定和固定 Snapshot，
  核对消息正文。附件/Skills 仍由对应平台适配器处理。

数据库 receipt 与 Session 配置在同一查询中读取；查询后的身份比较使用真实绑定，不将
Conversation 的托管工作区路径重定位投影误判为持久配置变化。读取结果不是长期有效的 lease，
实际模型/工具/事件准入仍须使用现有 owner 的 live fence。

Coding 工厂和每回合准备已改用公共入口，不仅是供未来使用的新接口。原进程宿主打开前就
核对消息正文，减少无效消息准备期间分配资源。保留原 Coding 事件、steering、技能、历史与
清理路径；没有把社区 engine 绑定到 `CodingRuntimeProfile::Coding`。
Coding 不再为了读取 Session 强持有 Conversation owner，公共入口使用 Weak 引用。

`RuntimeEngineHost::register_session_hosted` 在编译期注册 driver 工厂：
工厂收到 build options、`AdmittedEngineSession` 与 `Arc<EngineSessionHost>`。
平台先做真实 Session 解析，再调用该工厂；共享 HostedAgentRuntime 仍负责生命周期。
注册冻结后不可更改，没有打包后挂载、安装或热替换接口。默认目录仍只有 Nomi/Coding。

## 仍未完成

1. 通用 history/state journal 的生产装配、工具原子准入/结算及资源 owner 的完整公共 facade。
   当前公共工具端口与 Session/receipt 读取已落地，但不能仅凭这些事实自行授权工具。
2. 不依赖 Coding 循环及其事件 codec、通过真实生产端口完整执行的第三方 engine 示例。
   本切片没有用 mock driver、空清理或 Coding 包装器充当该示例。
3. MCP 当前产品目录/凭据/执行 owner、MiniApps 和非 function Plugin 生命周期；push
   持久回执、跨启动进程证明、人工隔离解决、安全续跑、动态指令范围等原有缺口。
4. 用户允许后再执行组件回归、消费者继承、真实 provider、升级与跨平台验收。

依赖公共组件的改动已纳入 Coding build digest；旧 Session 不重算 channel，也不自动迁移到新构建。
