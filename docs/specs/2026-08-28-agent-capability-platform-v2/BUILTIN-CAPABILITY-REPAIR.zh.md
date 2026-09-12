# 官方内置 Agent 能力修复记录

日期：2026-09-12。分支：`rf/agent-capability-platform-v2`。

## 结论

本轮没有删除、隐藏或撤下任何官方 Agent，也没有通过修改标签、放宽
Catalog 判定或接受占位 handler 来制造“可用”。当前 7 个官方模板仍完整
发布，共包含 97 条直接能力引用、78 个唯一能力 ID；真实 Nomi-core 启动后的
精确目录连接结果为 `missing=[]`、`unavailable=[]`。

官方能力按唯一执行 owner 分为：

| 执行 owner | 数量 | 说明 |
|---|---:|---|
| Nomi 原生投影 | 37 | 复用已有、受 Preset 能力上限约束的原生工具与运行时能力。 |
| host-backed PlatformBuiltin Tool | 30 | 必须存在真实 typed host port、精确 Schema 和执行适配。 |
| ContextContributor | 4 | 进入真实 Session/回合上下文，失败时按角色约束关闭。 |
| Lifecycle / TurnMiddleware / EventSource | 7 | 建立并释放真实 Session 生命周期句柄。 |

合计 78 项且 owner 集合互不重叠；`UNCLASSIFIED=[]`。Catalog 初始化、
Session 物化和 Plugin Registry refresh 使用同一组 source-aware admission。
TestFixture、metadata-only 占位注册、`domain_support accepted:true` 以及和原生
能力重复的 owner 都会失败关闭。

## 真实执行链

### 通用助理与搜索

- `web.search` 使用获得授权的 OpenAI Responses `web_search` 路径，严格选择
  支持该 feature 的模型与协议；综合回答和来源条目分离，不伪造来源摘要。
- `citation.render` 只能渲染当前 Session 搜索实际产生的 citation ID/URL。
- 搜索、Session 控制、Plugin 和动态工具统一返回结构化、脱敏错误；私有
  endpoint、凭据、数据库和底层 `AppError` 不进入模型上下文。

### Coding Agent

- Coding 的 22 项 runtime feature 逐项对应真实实现，再由验证后的
  `CompilerEnvironment` inventory 同时供 materialization policy 和 compiler
  使用；没有把它们伪造成 capability manifest 依赖。
- `agent.execution.observe/steer/fork`、`subagent.send/wait`、
  `fs.delete/watch/snapshot`、`vcs.push`、MCP、搜索、引用、视觉和 review 均有
  当前 Session 的权限与生命周期边界。
- MCP 在 `mcp.connect` 被按需激活前不读取仓库、不解析 OAuth、不建立连接；
  Session 只保存服务端 MCP ID/配置引用，env、headers 和 token 仅在宿主运行时
  注入。
- `fs.watch` 只有在真实 watcher 创建成功后才提交 active generation；失败不
  会显示为已激活，Session Drop 会释放 watcher。
- Chat/Creation Provider 的不可变修订在网络提交前校验；配置漂移时旧
  Snapshot 失败关闭，不静默采用新 endpoint 或 credential。
- Fork 不接收客户端自造的 operations/typed parameters；父 Session 的资源会
  重新经过完整服务端产品资源解析器。

### 伙伴、记忆、渠道与客服

- 伙伴 persona/roster/learn/evolve 以及 companion memory recall/write/merge/evolve
  使用真实 `CompanionService` 和持久层。
- 伙伴记忆写入、合并和演进使用耐久 exactly-once 回执；重启重放不会重复
  reinforce、重复归档或再次演进，取消/崩溃后的不确定结果禁止自动重试。
- Channel send/reply/pairing/group policy 使用真实渠道、配对与策略 owner；
  companion 和 customer-service 两种 owner domain 分开校验。
- `channel.receive` 注册真实 AgentSession ingress，每次入站都重新校验当前
  Channel owner 和持久化 typed binding，解绑不会误删更新后的绑定。
- 客服 dialogue middleware 接收服务端提供的当前消息、媒体类型和
  `cs_dialogue_id`；middleware 失败或超时会阻止模型调用，不能静默退化为
  普通 Agent。
- 客服备注、人工交接和 Channel 动作使用 SQLite 耐久回执/CAS 状态机；发送
  后网络结果不确定时进入 `OUTCOME_UNKNOWN` 栅栏，禁止盲目重发。

### 创作、Workshop 与 MiniApp

- `creation.text/image/image_edit/video/audio` 调用真实 `CreationService`，只在
  任务到达 `succeeded` 时返回成功；失败、取消、超时取消和结果不确定均是
  结构化非成功结果。
- Canvas/Asset 使用真实读取与 CAS；Asset 受选定素材库约束，Template run
  由 owner、Session、幂等键、绑定 Canvas 和模板派生，不能跨 Canvas 运行。
- 需要人工审阅的模板在产生副作用前返回 `HUMAN_REVIEW_REQUIRED`；无需审阅的
  受支持模板由后端推进至 terminal。
- MiniApp 已覆盖真实 source read/edit CAS、build、publish、enable 与 serve
  闭环，serving projection 不暴露 bearer capability。

### Robot

- Robot link/audio 建立真实 Session lease；动态 display/motion/device tools
  来自所绑定设备实际发布的精确名称与 Schema，不再让模型填写任意
  `tool_name`。
- 每次调用重新校验 Robot、Companion pairing、device identity 和 capability；
  Session Drop 释放 lease。
- 物理动作使用耐久 effect ledger。重启时未结算动作转为 sticky
  `OutcomeUnknown`，`retry_safe=false`；只清理超过安全窗口的明确终态回执。

## 资源配置语义

官方模板预置“角色完整”的能力，但具体 Companion、Channel、Customer、Robot、
Knowledge、MCP、Canvas、Provider 等用户资源在创建 Session 时选择。未配置资源
显示“使用时配置”，属于可操作的配置状态，不再被错误标成“待接入”。真正的
官方内置完整性错误显示为“系统能力异常”并保留 canonical error code。

客户端只提交 `resource_kind + resource_id`。operations、typed parameters、
owner 关系、连接修订和 secret 引用全部由服务端解析、校验并冻结；Create、
Switch、Fork 和远程入口使用同一解析边界。

## 发布门禁与验证

- 官方完整门禁：启动真实 Router，读取官方模板、Capability Catalog 和 Skill
  Catalog，创建全部 7 个模板并要求 Preview `Ready`；随后真实导入并应用一个
  本地 Plugin，触发 Registry reconcile/availability refresh，再次全量验证。
  两个阶段均为零 missing、零 unavailable，测试 `1/1 PASS`。
- `cargo check --locked -p nomifun-app --lib`：通过，零 warning。
- `plugin_tool_consumer`：`12/12 PASS`，覆盖 Tool、Context、Lifecycle、按需
  ToolSearch、失败不晋代、动态 Schema no-dispatch 与错误脱敏。
- Wave1/2/3/4、Channel、Customer Service、Robot、DB ID/schema、Provider
  revision、MCP、Subagent 与前端资源选择均有定向回归；合同生成器 `check`、
  TypeScript typecheck、i18n parity 和 diff whitespace 检查通过。
- Windows 开发桌面使用独立 `NOMIFUN_DATA_DIR` 完成真实 Tauri/WebView 编译和
  启动，Nomi-core Router、WebSocket 与 Agent 模板 API 正常。当前自动化环境未
  暴露原生应用窗口，因此没有把浏览器页面冒充成 Tauri 视觉点击验收。

## 尚需真实外部环境验证的边界

本轮没有用户的真实 OpenAI Responses/MCP 凭据、付费媒体 Provider，也没有
真实 ESP32/OLED/舵机/扬声器/摄像头。相关代码通过受控 HTTP、真实服务与
SQLite、MCP manager/fake transport、WebSocket fake device 及产品门禁验证；
外部供应商可用性、模型输出质量、计费和物理硬件效果不能由这些测试替代。

## 关键源码

- `crates/backend/nomifun-app/src/router/nomi_core_builtins.rs`
- `crates/backend/nomifun-app/src/router/nomi_core_session.rs`
- `crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs`
- `crates/backend/nomifun-ai-agent/src/plugin_tools.rs`
- `crates/backend/nomifun-app/tests/official_preset_catalog_integrity.rs`
- `ui/src/renderer/components/agent/AgentResourcePicker.tsx`
