# Agent 工具 hooks：实施与验收记录

基线：`14d10aa39dc2c56678acb24c94aee58ef5833aa6`，分支
`rf/agent-capability-platform-v2`。本机 macOS arm64。本文只记录本轮新增
H0 / H1a / H1b / U1，不使用此前多 Engine 的通过结果替代新功能验收。

## 当前进度

| 阶段 | 状态 | 进入下一阶段的条件 |
| --- | --- | --- |
| H0 工具链局部核对 | 已完成静态接线核对 | 下方 owner、覆盖矩阵与缺口已定位；不表示 hooks 已实现 |
| H1a 执行前检查 | 实施中 | 发布、选择、冻结、真实 Product/Node 与实际工具调用及拒绝验证 |
| H1b 成功结果整理 | 待 H1a 闭环 | 已执行事实先由现有 owner 保留，后处理失败不丢结果、不重放 |
| U1 业务输入与结果展示 | 待 H1b 闭环 | 保留现有页面平台，仅补表单、明确提交和有依据的展示 |

## H0：实际调用链

Nomi 的 `execute_tool_calls_scoped` 与 `execute_tool_calls_with_protocol`
共同执行参数规范化、请求中已公布工具集合/延迟状态/外层 JSON Schema 校验，
再进入 `execute_single_with_authority`。工具实际派发只有外层
`Tool::execute_with_context`，关联使用宿主派生的 `ToolExecutionContext`。
配置型 shell pre 在派发前，shell post 在派发后；保留这两条原功能。

外层 Schema 合法不等于边界工具的实时权限已经通过。新增 hook 公开参数前，
需经原 owner 的只读准入预检；实际派发仍保留最终授权检查。预检不能获取资源、
启动 Service 或执行目标操作。不能用 hook 的 allow 替代授权。

| 外层路径 | 实际执行 owner | H0 发现 |
| --- | --- | --- |
| 原生 builtin | ToolRegistry / Tool | 内部路径/动作校验需先明确可公开参数的边界 |
| 平台 builtin、PluginMount、冻结 MCP function | NomiPluginTool → retained invoker → Kernel | ThinAuthority、active set、exact target 在 Kernel 内；复用其只读预检 |
| PluginProduct function | NomiPluginProductTool → PluginProductOwner → Product Service | owner/release/epoch/catalog/action 需在 hook 前重新检查，Service 不提前启动 |
| 精确 MCP proxy | McpToolProxy / McpManager | 只覆盖外层一次调用，不覆盖远端内部动作 |
| lazy GenericMcpToolProxy | lazy runtime → exact MCP proxy | 外层 arguments:object 不等于实际目标 Schema；必须验证 exact target |
| MCP resource / Skill / fork | 各现有 adapter 与 Session owner | 内部资源/技能资格不得被外层 gate 冒充；不递归挂钩内部动作 |
| Mount dependency、Service 内部调用、hook 自身调用 | 原 Kernel / Product owner | 不经过 Nomi 外层工具 hook，不构成递归 hook 调度 |

### H1b 必须先修的结果所有权窗口

1. 当前单工具 timeout 包含 shell pre、实际执行和 shell post。目标成功后等待
   post 时，结果、附件、modifier 和 delegated effects 仍在局部 future 内；
   timeout/drop 会丢失该结果并合成“结果不可得”的工具超时。
2. 并发路径使用 join_all，某一工具已完成但兄弟工具未完成时，结果仍未交给 engine。
3. 宿主取消会 drop engine future，然后调用 abort_current_turn；原实现只根据
   尾部 ToolUse 合成错误，没有读取每调用的已完成结果。
4. EngineTaskGroup 保留任务结算 witness；值经 oneshot 返回，waiter drop 会丢弃值。
   retained task 不能被当作工具结果缓存。Product 回执在返回前已保留，但不能代替
   Nomi 对文本、附件和 delegated effects 的所有权。

H1b 的必要局部调整：每调用完成后、任何后处理 await 前同步交给现有 turn owner；
取消结算先使用已完成结果，只对未完成调用生成取消/未知状态。原结果用于媒体、
效果、回执和历史，插件只生成标明来源的模型消费文本。不得另建历史库或执行系统。

## 验证边界

本轮只使用隔离数据目录 `.git/hook-product-validation/`，不读取日常凭据，
不修改用户真实数据，不上传制品。真实模型固定为用户授权的 StepFun Coding Plan
`step-3.7-flash`；凭据不写入本文件、源码、命令参数或测试日志。
Cargo 串行；独立源代码模块允许并发，U1 在工具 hooks 闭环后实施。

## H1a 当前实施证据（尚未收口）

已接通精确 before_tool 发布合同、隐藏消费者、Nomi 引擎准入、冻结顺序、原 owner
只读预检和实际派发。Coding 及非 Product hook 来源明确拒绝；未选择 hook 时不调用
预检。拒绝保持目标工具未执行，技术失败停止后续模型与未派发工具。参数经过有界
脱敏，插件不能修改原参数。新增执行扩展入口复用工作台和普通 Plugin 草稿。

### 真实产品流程发现及修复

- 严格输出用例发现 serde 单元枚举会忽略 allow 中的额外字段；改为严格对象变体，
  保留负向回归，不能夹带参数 patch。
- retained waiter 原来只关闭 turn，取消未传入 Node。现在同一调用取消标志贯穿原
  invoker、Product owner 和 Service actor；保留任务与 Unknown/pending 结算语义。
  真实 Node 的 AbortSignal 标记、零目标执行、停止后结算均有定向证据。
- 普通保存把 `NeedsTestInput` 误报成代码失败，导致有业务能力的 Service 无法通过
  用户入口发布。现在区分“启动通过、业务未验证”，展示发布前确认；确认绑定精确
  draft/source/release/receipt/config/credential 版本，复用原发布校验。没有自动确认、
  把业务未验证改成 Passed 或允许启动失败的代码通过普通确认发布。
- 返回编辑后重新检查会尝试重复 build 相同 Ready；改为只在源码变化或无现成制品时
  build，重新检查同一精确 Ready 产生新的回执。过时确认仍拒绝。
- 双 before_model 冷启动在未优化开发构建中耗时约 6.6 秒，超过原共享 5 秒。
  sha2 的 dev/test 编译优化后，原始双冷启动/排序/冻结用例通过（总计 3.64 秒）；
  不缓存或跳过完整 Node 摘要，不延长门禁。临时预启动步骤已撤销，原失败冷链已重验。

### 已运行的定向证据

以下是不同范围的结果，重叠用例不相加：

| 范围 | 当前证据 |
| --- | --- |
| nomi-tools 全库 | 337 通过，包括 9 个新增只读预检 |
| Nomi before_tool | 8 个执行用例通过，另 2 个严格输出/脱敏用例通过 |
| Kernel 只读预检 | 4 通过，包括零资源获取、零派发、后续撤权 |
| Product owner 预检 | 真实发布后预检通过，拒绝请求不启动 Service |
| Product waiter 取消 | 2 通过；原任务继续保留，正常结果不发取消 |
| 原有工具执行 / shell hooks | 10 通过 |
| 真实 Node before_tool | 发布选择、allow/deny、旧会话无 hook、冻结排序、非法输出、超时、取消、停用通过 |
| 普通模板发布 | 首次不发布、伪造/过期确认拒绝、重新检查及精确确认后启用通过 |
| Plugin 草稿/保存库用例 | 15 通过 |
| UI | 执行扩展与发布确认定向用例、typecheck、桌面边界通过；原生新流程待验 |

首次安全实模尝试在普通模板发布阶段返回 400，尚未进入模型推理；该次宿主关闭与
凭据审计已完成。修复后第二次实模尝试通过发布选择，但 allow 阶段因
`BEFORE_TOOL_TARGET_PROJECTION_INVALID` 返回 422，不能记为实模通过。

后续本地闭环修正两处测试证据读取假设：等待会话 ready 后重新读取最终消息，避免
旧消息与新状态混读；API 展示操作 ID 与 Conversation 持久操作 ID 属于不同层，
按真实 owner/session/idempotency key 查找唯一成功 delivery，核验请求正文与持久用户
消息后再关联 HostedEffect，不修改生产存储编码或删除关联检查。
真实 Node 普通模板发布、Nomi 原生 Write、allow/deny 文件效果、回执和后续模型消费的
loopback 用例通过（1 passed，0 failed，3.30 秒）。模型为本地 stub，此结果不替代
真实 StepFun；修正后的 `--before-tool-smoke` 仍待重跑。

提交前 `before_tool_smoke::tests` 全部 7 项通过（0 失败，3.39 秒），包括上述真实
Node/原生 Write 本地闭环和证据关联回归；未在本次提交检查中调用真实模型。
`bun run check` 已重新通过，包含 typecheck、桌面边界及现有综合门禁。
提交前 contract check 发现生成清单摘要漂移，按当前源码使用
`agent-v2-contract write` 重生成后 check 通过；没有修改历史存储编码或关闭门禁。

### 当前目标工具范围限制

常用文件、计划、发现、LocalOwner 命令、现有精确 Mount/Product 函数以及已连接的
精确 MCP 工具已接入只读预检。Python 解释器探测、特殊 sandbox、无可信 URI 目录的
MCP resource read，以及没有只读 owner 准入口的动态工具当前明确拒绝，不先执行操作
取得资格，也不向 hook 外送未准入参数。复合 Skill 只覆盖外层一次调用，不递归拦截内部动作。

Windows x64 需要重跑当前相同源码的 Node 冷启动、取消/进程回收、空格/Unicode 路径、
文件缓存预检、发布确认和 native UI；macOS 结果不代替 Windows。H1b 与 U1 尚未实施，
签名、公证、当前 hooks 的正式安装包和最终真实模型验收仍未完成。
