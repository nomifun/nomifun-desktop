# Agent 工具 hooks：实施与验收记录

基线：`14d10aa39dc2c56678acb24c94aee58ef5833aa6`，分支
`rf/agent-capability-platform-v2`。本机 macOS arm64。本文只记录本轮新增
H0 / H1a / H1b / U1，不使用此前多 Engine 的通过结果替代新功能验收。

源码检查点：`8cd2cebcf19ba57fd530503a1b80afc2b75c848d` 已按用户授权提交并推送。
此后的接续验证单独记录，不把源码推送当作 H1a 产品验收通过。

## 当前进度

| 阶段 | 状态 | 进入下一阶段的条件 |
| --- | --- | --- |
| H0 工具链局部核对 | 已完成静态接线核对 | 下方 owner、覆盖矩阵与缺口已定位；不表示 hooks 已实现 |
| H1a 执行前检查 | macOS 原生核心产品闭环通过；严格 LF smoke 失败保留 | 已有发布、选择、保存、新会话、真实模型/Node/工具、拒绝与续接证据；Windows/制品待总验收 |
| H1b 成功结果整理 | 因成本偏高移出开发计划 | 不预留 after_tool 空接口，不启动其专属结果/媒体结算改造 |
| U0/U1 会话页替换及页面增强 | 用户已取消，源码移除及 macOS 核心验证通过 | 会话恢复标准 UI；普通带 UI App/常驻插件保留，见[退役记录](2026-09-15-agent-session-view-retirement.zh.md) |

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

### 已取消 H1b 的成本依据与既有风险（非后续实施卡）

1. 当前单工具 timeout 包含 shell pre、实际执行和 shell post。目标成功后等待
   post 时，结果、附件、modifier 和 delegated effects 仍在局部 future 内；
   timeout/drop 会丢失该结果并合成“结果不可得”的工具超时。
2. 并发路径使用 join_all，某一工具已完成但兄弟工具未完成时，结果仍未交给 engine。
3. 宿主取消会 drop engine future，然后调用 abort_current_turn；原实现只根据
   尾部 ToolUse 合成错误，没有读取每调用的已完成结果。
4. EngineTaskGroup 保留任务结算 witness；值经 oneshot 返回，waiter drop 会丢弃值。
   retained task 不能被当作工具结果缓存。Product 回执在返回前已保留，但不能代替
   Nomi 对文本、附件和 delegated effects 的所有权。

进一步只读核对确认：restore accepted root 会恢复旧消息；manager 的取消/失败路径、
媒体 sink 和 Conversation 终态又会统一回滚待交付附件。安全实现 after_tool 还需要
明确可编辑文本与宿主警告/资源引用的边界，不能直接替换整个成功结果正文。

用户已允许成本高的 hook 不实现。基于上述跨层成本，取消 H1b 及仅为它准备的结果
owner/媒体子集结算改造，不将其更名为前置项目继续推进。尚未开放的 after_tool
合同/schema 草案已删除，运行时继续明确拒绝不支持阶段；before_tool/before_model
及既有配置型 shell hooks 保留。上述既有风险仍是未修复的审查结论；若在现行支持
路径复现具体缺陷，按具体问题评估，不据此自动启动原 H1b 改造。

## 验证边界

本轮验收要求使用隔离数据目录 `.git/hook-product-validation/`，不读取日常凭据、
不修改用户真实数据、不上传制品；实际退出观察偏差及默认开发目录被触及的事实见下文，
不能以这项要求替代实际结果。真实模型固定为用户授权的 StepFun Coding Plan
`step-3.7-flash`；凭据不写入本文件、源码、命令参数或测试日志。
Cargo 串行；独立源代码模块允许并发。after_tool 与会话页替换 U0/U1 均已取消，普通 App UI/常驻插件不受影响。

## H1a 实施与核心产品证据（非正式发布放行）

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
- 原生截图发现缺模型保存失败只有“重试”，配置入口不可达。现在仅在明确的
  `MODEL_ROUTE_NOT_CONFIGURED` 下显示“配置聊天模型”，进入现有模型页。
  明确点击配置模型或创建 before_tool 草稿时，当前 history entry 保留这次未保存
  编辑，顶部返回后立即消费移除；不写数据库或长期草稿存储，不自动创建/保存/执行。
  模型路由和记录不进入快照；已有 Agent 要求精确版本仍匹配，否则显示当前版本变化。
  6 项往返测试（32 断言）、相邻 46 项回归、typecheck 和桌面边界通过；实际窗口待复核。

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

提交后第三次固定 StepFun 重跑已执行：compile、发布选择通过，allow 阶段返回
`BEFORE_TOOL_TARGET_CONTENT_INVALID`（422）。这次检查已能定位到工具参数正文与
预期字节不一致；尚未区分模型输出差异与宿主投影处理，不能记为 allow 通过。
失败运行按原规则关闭宿主、审计日志并清理临时 fixture，未保留供原生 UI 使用的数据根。

第四次诊断重跑返回 `BEFORE_TOOL_TARGET_CONTENT_MISSING_FINAL_LF`，定位为正文缺少
预期末尾 LF。新增诊断只返回固定分类码，不输出原文；后续提示明确要求保留一个
U+000A 末尾换行，不修改工具参数、预期内容或文件字节一致性检查。

明确说明 Write 不自动补换行后仍复现相同失败；独立固定 endpoint/model 的 SSE
探测首次未取得恰好一条工具调用，第二次取得工具调用并返回缺少末尾 LF
（只输出分类码，未执行工具），支持这是模型指令
遵循差异的判断。实模 smoke 继续保留失败，不放宽实际文件与参数的严格一致性要求。

为继续独立的原生产品验收，`--retain-native-fixture` 现在允许保留已正常关闭宿主、
完成凭据审计的失败运行数据；失败退出码与阶段诊断不变。默认运行仍清理；未完成
关闭/审计的运行不保留。隔离 Provider 凭据仅经普通存储加密保存，用于当前 UI 验收，
不能把保留数据路径当作 smoke 通过证据。

提交前 `before_tool_smoke::tests` 全部 7 项通过（0 失败，3.39 秒），包括上述真实
Node/原生 Write 本地闭环和证据关联回归；未在本次提交检查中调用真实模型。
`bun run check` 已重新通过，包含 typecheck、桌面边界及现有综合门禁。
提交前 contract check 发现生成清单摘要漂移，按当前源码使用
`agent-v2-contract write` 重生成后 check 通过；没有修改历史存储编码或关闭门禁。

### 原生窗口接续证据（仍在验收）

当前源码使用隔离 `NOMIFUN_DATA_DIR` / `NOMIFUN_WORK_DIR` 启动
`bun run dev --no-watch`，arm64 Mach-O 开发程序编译通过（2m47s），
`/health` 返回 ok。测试 app runner 为相同 debug 可执行文件提供 macOS app 身份，
字节一致性 SHA-256：`d284f11fec8b0958e58a3faafbf91d66100a333e42d10238b67ecc45b243e03a`。
这是开发程序校验值，不是当前正式 app/DMG 的交付摘要。

原生窗口已完成：从个人 Agent 创建普通检查草稿、预览、首次保存要求确认、返回编辑、
重新检查同一草稿、明确确认发布、保存 Agent 名称。数据库只读核验确认名称与新 revision
已保存，原精确 Nomi 引擎和已选能力未被静默替换；刚发布的第二个检查尚未自动选入。

窗口检查暴露两项 UI 问题：执行扩展藏在基本设置、长说明挤占身份/模型字段，已移至
现有技能与扩展页并精简文案，原生截图已核对；顶部返回使用应用自有导航记录，
未保留 Router state，导致作者往返回到默认模板。现已在原 50 条内存导航记录内保留
state，并用同一 replaceCurrent 同步保存/消费；生产导航回归及原生“发布页→作者页→
原 Agent”两次顶部返回已通过，未保存名称保留且可继续保存。没有新导航栈或长期草稿存储。

原生纵向验收使用 UI 刚发布的检查，明确替换测试 Agent 中旧检查并保存，再从
“使用 Agent”新建会话并选择隔离工作区。固定 StepFun `step-3.7-flash` 的两轮实际结果：

- `Write nativeproof.txt` 放行，文件字节严格等于本次 UI 请求的 `HELLO`（5 字节），
  同回合随后模型回复 DONE。
- `Write .env.nativeproof` 被真实检查拒绝；窗口工具详情明确显示目标未执行，
  同回合随后模型回复 BLOCKED，目标文件不存在且无工具重放。
- 数据库只读核验两条工具记录、两条精确发布版本的 returned HostedEffect，
  allow/deny JSON 摘要分别匹配；回执关联真实成功 delivery 与同一持久用户消息。
  新会话冻结新检查和精确 Nomi 引擎，旧会话仍保留旧检查；未观察到静默替换。

证据脚本及摘要：`.git/hook-product-validation/verify-native-h1a.py`、
`h1a-native-evidence.json`。这是独立的原生用户流程，**不把前述末尾 LF smoke 失败改为通过**。

窗口缩至原生最小限制后，截图尺寸 881×600，执行扩展、原生拒绝详情、输入区和列表
开合仍可用。Command-Q 正常退出，dev 进程退出码 0，应用 PID 和 56108/5173 端口
均已释放；本次退出前无活动子进程，不能代替活动工具取消专项。
相关截图位于本机 `.git/hook-product-validation/audit/03-*` 至 `13-*`，不随源码上传。

**退出观察偏差：** Command-Q 后调用原生窗口状态读取工具，工具自动重新启动了测试
app（新 PID 29389），该次启动没有继承 shell 的隔离环境。后一次 dev 重启被单实例
机制拦住并退出 0，不能当作正常重启通过。已对该新实例发送 SIGTERM 并核验其退出。
只读文件时间戳检查显示默认开发目录 `NomiFun-dev-nomi-core` 在该次启动/退出中被触及；
正式 `NomiFun` 的目录、数据库及 WAL 时间戳仍为 2026-09-14。未查看/导出默认目录凭据，
未在该实例进行交互或提交模型任务，也未删除或回滚默认目录数据。此偏差不能写成
“整个验收始终未触及默认开发目录”。

后续退出只用进程/端口核验，不对已退出 app 再调用会启动应用的窗口观察 API。
本机测试 runner 的 app 元数据增加指定隔离数据/工作目录的 LSEnvironment，作为
启动隔离补充；仍先显式启动，再按实际进程打开的数据库路径核验后进行 UI 操作。

随后显式重启成功，进程实际打开的唯一 backend 数据库经 lsof 核验位于本轮
`.git/hook-product-validation/before-tool-native-p1z81Z/data`。原 Agent、会话、模型及
已选检查保留。临时停用隔离环境唯一模型后，普通模板保存出现“配置聊天模型”按钮；
进入原模型页恢复 StepFun、顶部返回后模板名 `Model Recovery Check` 保留，再明确保存
成功。只读核验该名称仅创建一条 Agent，恢复流程未新增会话或模型消息。

最终 Command-Q 后只通过外部进程/端口检查：dev 退出码 0，测试 app 实例数 0，
57638/5173 均关闭，未再调用窗口观察导致重启。日志刷新后对测试目录及本轮改动
进行实际凭据字节审计：261 个文件、0 处明文匹配。加密测试配置保留供本任务后续
验收使用，不能表述为凭据没有存储。补充截图 `14-*` 至 `16-*` 记录缺模型恢复。

本轮新增 UI 收口后的 `bun run check` 全链通过，before_tool child 的 8 项本地回归
全部通过（4.07 秒）。再次 `bun run dev` 时既有 prune 脚本因 build.noindex 超过
默认 36G 缓存上限而清理构建 profile，触发完整重编译；它只影响可重建缓存，
原生验收数据和截图在 `.git/` 下保留。当前正式制品仍须基于最终源码重新生成。

### 当前目标工具范围限制

常用文件、计划、发现、LocalOwner 命令、现有精确 Mount/Product 函数以及已连接的
精确 MCP 工具已接入只读预检。Python 解释器探测、特殊 sandbox、无可信 URI 目录的
MCP resource read，以及没有只读 owner 准入口的动态工具当前明确拒绝，不先执行操作
取得资格，也不向 hook 外送未准入参数。复合 Skill 只覆盖外层一次调用，不递归拦截内部动作。

Windows x64 需要重跑当前相同源码的 Node 冷启动、取消/进程回收、空格/Unicode 路径、
文件缓存预检、发布确认和 native UI；macOS 结果不代替 Windows。H1b 及会话页替换 U0/U1 已取消，
签名、公证、当前 hooks 的正式安装包和最终真实模型验收仍未完成。
