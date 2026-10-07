# 插件与独立小程序实施记录

更新：2026-10-07。状态：首期独立小程序修复及桌面验收完成，可交付用户体验。真实模型、正式实例与重启数据已核对；不以历史成功记录替代当前生成验收。

## 首期范围

首要目标是以 NomiFun JS 运行环境承载即时开发、可运行、独立可托管的小程序。托管指 NomiFun 内安装、生命周期管理与持久存储，不要求公网发布。UI 使用真实 WebView Surface 与 SDK；后台逻辑可按需使用 Service，Binding 可以为空。

创建会话结束或开发模块关闭后，已安装小程序仍能从插件库打开。Agent 工具、原会话消费、Hook、Desktop/Automation 扩展属于后续能力；保留实现和回归覆盖，移出首期阻塞验收。

## 当前链路与边界

- 普通会话使用 plugin.development，Agent 和 GUI 共用统一 Plugin Core。链路为 list → open → plan → apply → check → preview → 计划测试 → install → inspect。
- plugin_delivery 是明确请求合同；仅拥有旧草稿的会话获得中立上下文，本次实际 plan/apply 后才检查本请求交付。预激活仅改变展示，不增加冻结权限。
- plan 固化输出、功能和精确用例，修复不能弱化。plan.cases 是唯一必需集合，所有计划用例都必须通过，包括未被 feature 引用的用例；持久化必须取得 reopen/restart 后的读取证据。
- 计划外探针保留诊断，不扩大验收或进入正式实例探测。删除 acceptance 镜像和由最后一次用例覆盖的顶层 ui_ready；正式 installed_observation.ui_ready 保留。
- UI 用例各自从新预览存储开始，reopen 保留该用例数据。UI + Service 共用实例存储；Service 写入和 restart 读取相邻，再执行 UI 用例。
- 安装后拒绝预览测试。安装验证准确 Artifact、正式入口、SDK/Host 往返和持久交付回执；不在生产环境重放写入或外部效果。
- SDK 存储使用真实 DataRoot。异步加载与写入期间禁用相关控件或设置 aria-busy；不用 iframe 禁止的本地浏览器存储、表单提交或弹窗。
- 初始化读取失败时保持未知状态与变更禁用，显示真实错误并提供重试或重开；读取成功后才允许写入。生成指导不要求产品提供模拟故障开关或测试专用分支。

## 阻塞修复

| 问题 | 当前实现 |
| --- | --- |
| B1 无限修复 | 从已结算 typed canonical 工具事件回放派生：计划用例连续失败三次且无新增计划用例通过，或创作输入段达到 64 模型步，复用原生 execution pressure/checkpoint 暂停。revision、诊断和返回 passed:false 不算进展。 |
| 暂停与恢复 | 沿用 PLUGIN_VERIFICATION_REQUIRED 和原会话继续入口。已接受用户输入先于停滞检查生效；新输入重开修复段，恢复本身不重置，累计预算不增加。Delivered/Dormant 不被停滞检查误暂停。有效结构化 blocked report 交还宿主，不猜测自然语言关键词。 |
| B2 探针扩大义务 | 安装、完成闸门和正式探测统一只检查 plan.cases，失败探针不能阻塞交付。 |
| B3 无诊断 | test_ui 直接返回最多四条限长诊断，包含步骤、操作、selector、期望、实际观察及错误；插件文本保持工具数据，不进入宿主用户角色反馈。 |
| B4 修订冲突 | 宿主在现有草稿 verification 中记录 edit_revision。模型可复用该水位至当前版本间的 revision，底层仍按当前精确 CAS 提交。plan/apply、GUI 编辑、安装基线、回滚及删除关联抬水位；缺失/非法水位严格检查当前版本，不凭相同摘要盲接旧编辑。 |
| GUI 并发覆盖 | 原子文件替换持锁直到数据库 CAS，失败回滚实际替换前整棵源码树。GUI 保留精确版本锁，UI/Action 晚到结果仍校验精确执行版本和 Surface。 |
| GUI 旧证据残留 | 成功源码编辑只保留请求关联、计划和新编辑水位，清除旧结构、运行、用例、授权与交付证据，不能用安装的旧 Artifact 结算修改后的草稿。 |
| B5 报告计数差一 | 保留全部错误事实，拒绝反馈给出该批次记录后下一次应提交的计数，与下一次 schema const 一致。连续控制失败之前宿主 Delivered 可结算纯插件任务；workspace/ssh/requirements、运行进程与 patch 恢复义务仍遵守原账本和 terminal input fence。 |
| UI 异步就绪 | click/fill 在四秒内重查真实 DOM、等待可交互，支持 disabled/readonly/aria-disabled/inert/aria-busy/隐藏状态，超时给具体原因。reopen 等待新 SDK 连接及 ready 往返，替代固定 300ms；text/count 保留四秒断言轮询。 |

宿主完成反馈最多三次，按 accepted input 分段，后续反馈之间要求已授权工具工作。该反馈预算与实际修复停滞检查不同。HostPause/Delivered 的适用路径在没有账本工作时跳过完成复核、开放计划和完成报告检查；最终闸门仍以持久事实决定。

观察器不持久化另一份 transcript、进度账本或插件 checkpoint。恢复保持同一 canonical Session、Turn、Snapshot 与原生预算合同。

真实验收另发现并修复模型切换后的 prior_task 边界：已验证的旧 Snapshot 保留为普通历史，但旧任务账本不能传给新 Snapshot。候选只取最新关闭 Turn，并要求相同 Session/runtime/Snapshot；不会回溯挑选更老的账本，也不放宽 Runtime 或 checkpoint 校验。

## 本轮验证

| 检查 | 当前证据 |
| --- | --- |
| agent-runtime | 364 通过，1 项原有忽略 |
| plugin-development | 7 项单测和 3 项合同测试通过 |
| plugin-platform lib / lifecycle / draft_replacement | 3 / 8 / 5 项通过，含真实 SQLite 与源码树的冲突回滚 |
| agent-contracts / agent-session | 72 / 95 项通过 |
| SDK / 插件页面 | 14 / 29 项通过，覆盖初始化、保存、控件替换和重开 |
| typecheck / 前端生产构建 | 通过 |
| i18n / Desktop UI / Agent Session / Unified Plugin / vocabulary / UARC | 通过 |
| app 定向测试 / plugin_e2e | 69 / 18 项通过；另 6 项 history 测试通过，包含模型与 Agent 切换。 |
| clippy | 所列包与 app all-targets 执行通过；当前工具链报告既有 lint 警告，未以“零警告”表述结果。 |
| 当前代码真实模型 | step-5-preview 新建 30 步完成，7 个计划用例通过；更新 38 步完成，6 个计划用例通过。两次均取得 canonical 完成与准确安装回执。 |
| 生成 HTML 异步专项 | 最终更新源码 12/12 通过，覆盖初始化延迟/失败、并发保护、写入失败回滚与重试。隔离 DOM/模拟 SDK，只证明源码在这些条件下的行为，不代替真实 Bridge/DataRoot 验收。 |
| 同会话模型切换 | 原会话切换到 step-5-preview 后，只读回合 1 步完成，历史读取与 prior_task 边界修复已真实验证。 |
| 独立打开 / 880×600 / 完整退出重启 | 通过。创建会话结束后从插件库独立打开，实际添加三项、勾选第二项、删除第三项并刷新；真实桌面内尺寸 880×600 下再添加、删除。完整退出验收进程并重启，正式版本、数据代际、两条数据和勾选状态全部一致。 |

失败基线：此前待办需求执行 170 步，54 次 test_ui、29 次 apply，没有安装或暂停；25 次修订冲突，计划外探针成为必需集合。此前 headless 原会话调用成功，但发生五次报告拒绝。这些用于定位问题，不证明本轮代码通过。

本轮早期 Step-3.7-flash 曾完成 31 步创建安装，零修订冲突；人工走查发现计划点击整行遗漏实际 checkbox。后续隔离专项又发现 add 与行点击未共用保护，以及初始化读取失败后可能覆盖未读到的既有数据。这些早期成功不能作为最终体验验收；后续真实模型已修复实际 checkbox、共享变更保护与读取失败恢复，最终源码的专项结果为 12/12。

### 当前真实模型证据

证据目录：`C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance`。完整事件来自当前隔离数据集的 canonical 日志，不以模型最终回答代替完成或安装事实。

| 场景 | Session 与结果 | 证据 |
| --- | --- | --- |
| step-5-preview 新建待办 | `01a11643-57f8-7c70-a203-39c0da475302`：30 步完成，7 个计划用例最终通过，安装 revision 1。创建 Turn 的 canonical 完成在 seq 563；该文件也保留之后另一个已取消跟进回合。 | [完整事件](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/full-step5-fresh.jsonl) |
| step-5-preview 更新与交付整理 | `01a11661-2afa-7433-9e9c-d0a9faddf6b7`：38 步完成，6 个计划用例通过，安装 revision 2，移除开发故障开关并保留真实读取失败的重试入口。 | [完整事件](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/full-step5-cleanup.jsonl)、[12/12 异步专项](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/async-acceptance-step5-cleanup-preview-retry.json) |
| 原会话显式模型切换 | `01a115dd-d063-78a0-aa6a-2fda625da90a`：step-3.7-flash 切换到 step-5-preview，修复后的只读回合 1 步完成；保留历史与旧失败记录。 | [完整事件](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/full-step5-model-transition.jsonl) |

新建与更新都实际触发过计划用例三次失败后的 `PLUGIN_VERIFICATION_REQUIRED` 暂停，接受用户续作输入后继续原 Turn 并最终交付。这证明有限修复与继续入口参与了真实模型流程；上述步数包含这些修复，不代表一次无干预完成。

最终体验样本：Plugin `01a11651-d0f5-7fb0-b856-70d6fc98beb0`，revision `2`，版本 `0.1.1`，Artifact `b0748298f1aa6228df4e93b3ce54725af849a935ebaf593e7796c0cd8899fe92`。桌面独立运行与重启使用这一实例和准确 Artifact；正式数据代际 `01a11651-d10e-79d1-a0cf-dbe9389a9206` 未改变。

人工桌面验收保留两条数据：“体验验收：重启后保留”（未完成）和“体验验收：第二项勾选”（已完成）。[重启前事实](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/manual-before-restart.json)与[重启后事实](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/manual-after-restart.json)断言源码 SHA、安装记录、数据与存储 revision 相等；[最低桌面尺寸截图](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/gold-880x600-add.png)与[重启后截图](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/gold-after-full-restart.png)保留真实 UI 证据。

体验程序为本机 debug 验收构建，使用独立标识与 `NomiFun-repro-plugin-r3-20261007` 数据目录。[启动入口](C:/Users/rika0/.codex/visualizations/2026/10/07/01a11586-ae1b-7251-abcc-44d95ab4bcdb/nomifun-acceptance/启动小程序验收.bat)无需开发服务器；打开“插件与小程序”中最新的“待办清单” v0.1.1。上述留存验收 exe 的 SHA256 为 `2c8d4c32b8119db4e078b71623edcd9c65ebfc77ba0fc4bd6f2d676d23de8762`，对应本次合入 main 前的创建验收构建；没有制作生产安装发行包。

### main 集成

用户要求合入 main 后，先获取远端 `12c415003`，保留其全部更新。插件修复提交为 `845bb0243`，UI 创作与显示入口提交为 `0e2c8e619`。删除 `PLUGIN_FEATURE_VISIBLE` 及退役 helper，插件库、独立运行路由、侧栏、会话交付卡片和 Agent 编辑入口直接显示；桌面授权、WebUI 只读和用户显式关闭开发能力仍按原合同生效。

共享 Runtime 按 main 的 completion review、voice immediate correction 与 report-only phase correction 整合。明确宿主 Reject 反馈允许继续修复已授权的插件缺口，不续购 review 或执行预算，不释放普通报告纠正锁。纯插件的宿主交付结算使用原生 TurnCompleted，保留已记录的输出与工具事实；不伪造已接受的完成报告或模型输出。回归验证包含完整历史回放和实际启动下一 Turn。main 原有完成阶段规则保持完整。

合并回归：Runtime 378 通过、1 项原有忽略；app 定向及历史 82 通过；plugin_e2e 18 通过；Agent contracts 80、Agent Session 116 通过；Plugin development 7 项单测和 3 项合同测试、Plugin platform 全部单测与集成套件通过。UI 集成 95 项测试、SDK 14 项、typecheck、i18n、Desktop UI、Agent Session、Unified Plugin、vocabulary、UARC、烟测 runner 自检及 PowerShell 语法检查均通过。`nomi_core_live_provider_smoke` 使用 `browser-use,computer-use` 特性编译通过，确实覆盖插件烟测模块；只编译，没有调用真实 Provider。测试日志保存在上述证据目录的 `main-merge-*.log`。上述真实模型与重启证据仍属于留存创建验收构建，不宣称在 main 集成版上重跑了真实模型。

本次仅在本地提交并合入 main；不推送远端，不改写既有提交历史。

## 体验验收条件

1. 从插件库进入普通会话，真实模型完成待办生成、检查、交互和持久化验证、安装、交付。
2. 添加、勾选、删除、关闭重开后数据保留；核对 canonical 终态与准确安装事实。
3. 完整退出并重启 NomiFun，从插件库独立打开同一小程序，代码与数据保留。
4. 覆盖桌面最低 880×600，不增加移动端验收。
5. 无进展时有限暂停，保留草稿、失败步骤与原会话继续入口。

真实模型使用新隔离数据集，不复用留有运行中回合的 r2。记录模型、会话、步骤、耗时、错误、计划测试与安装事实；达到运行时限主动停止目标回合。单测和脚本 Provider 不能替代真实生成。

后续单独验收：headless Agent 工具、原会话新工具接入、Hook、指定草稿继续等系统扩展。已安装待办更新已取得上述真实模型证据；其他扩展场景不列为首期小程序发布门槛。Robot 的 deferred/frozen 比较问题属于其他能力链路，不阻塞首期小程序。
