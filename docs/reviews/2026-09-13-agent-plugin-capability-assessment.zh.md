# Agent 全栈插件平台评估：当前基线与开放式组装架构

## 当前发布口径（2026-09-15，覆盖下文历史目标与排期）

按用户最新决定：**尽快上线，尽可能可替换，而非任意替换。** 本节及 [P0 发布台账](2026-09-15-plugin-release-readiness.zh.md) 是当前范围和发布判断入口；下文的 JS-only、全栈/任意部件全替换、五问整体关单、旧“下一批”和人周估计保留为历史，不再作为本次上线承诺或执行指令。

- 正式默认使用内置 Agent、内置模型链路和内置 UI。已有 Tool、Context、discovery、`before_model`、只读 Skill 按已支持边界及真实证据验收，不扩大为所有领域可替换。
- native Service / Rust SDK 已实现的事实保留，不撤销、不回滚；本次定位为默认关闭、须显式启用的可信开发者实验，不宣称操作系统权限沙箱或普通用户正式支持。插件 Agent 页面同样为实验能力，不替换正式默认 UI。
- 模型插件、整个 Agent runtime、Shell 和深层基础设施替换延期；不因已有模型合同、流式前置或 native 后端而启动其消费接线。
- 29 项改为长期能力评估清单，不是上线欠账、强制排期或必须全部清零的门槛。延期不等于已完成；若其中问题实际影响本次正式范围的安全或可用性，按具体证据进入 P0，而非整项扩张。

本轮已完成发布收敛改造与 Windows 定向回归：服务端实验页面准入、默认 UI 恢复、追加式数据库迁移、Hidden 调用准入与取消回收，以及临时提示词 patch 不进入持久回执正文。具体结果和剩余制品验收以发布台账为准，不能宣称正式上线已通过；其他 OS/架构仍未验证。跨 OS TODO 与后续可复制 prompt 也在该台账维护，不启动跨平台重构。

## 历史评估与实施记录（以下原日期、事实和编号保留）

日期：2026-09-13\
分支：`rf/agent-capability-platform-v2`\
基线提交：`08caa20d7b278423b09a56d126e465378eff6403`\
状态：已按 2026-09-15 最新授权加入 Rust 原生 Service 后端与 SDK；JS/Rust 共用 Product 平台，完整领域开放目标尚未完成

**开发安排阅读入口：** 最新同步、Rust 实施与交付范围看 §27.17 / 实施台账 §2.40；模型功能卡看 §27.16；29 行剩余状态看 [实施台账 §3](2026-09-14-agent-plugin-openness-implementation.zh.md#3-原始-29-个环节不得缩减的跟踪清单)。§27.13～§27.14 的 JS-only 排期是历史，最新授权已允许原生后端；不为等待 Rust 强做临时 JS 包装，但也不因增加后端而关闭尚无真实消费者的功能卡。下文早期的“下一批”及旧人周估计不再作为并行执行指令。

**2026-09-14 历史实施边界（Rust 排期已由 §27.17 更新）：** 用户明确要求，如果某项在现有 JS 插件支持下不是合理实现、更适合等 Rust 插件，则本期不做，避免临时方案和历史债务。因此，29 行继续作为长期能力清单，但不再要求每行本期强行交付 JS 第二实现。具体适配性与延期条件见 §27.13；延期项不记作已解决，也不为它们预建 Rust stub 或无人消费的协议。

最新页面切片（2026-09-14）：已接线显式 `agent_view` 发布、同 Catalog 的 UI 候选和真实 Agent 路由的页面局部选择；对应验证及剩余边界见 [实施台账 §2.21](2026-09-14-agent-plugin-openness-implementation.zh.md#221-agent-页面贡献发布与显式选择)。下面早期记录中的“系统页面仍固定装配”保留为当时状态，不再概括此切片。当前仍不是持久 UI 默认/Role 绑定、Shell 或完整恢复体验的交付，需求 2/5 不关单。

后续参考页面（2026-09-14）：在现有 Agent 页面增加“创建参考视图草稿”，沿原编辑/保存/构建/发布流程交付实际 HTML 源码，不新增原生辅助程序或执行后端。参考页只展示可变的持久化历史，分页重读，实时事件仅触发刷新；不拼接没有共同恢复游标的 token。支持文本发送、同意图显式重试与取消，预览无 Session 权限。实现和验证见 [实施台账 §2.22](2026-09-14-agent-plugin-openness-implementation.zh.md#222-agent-参考页面沿原草稿发布链交付)。这不是全部消息类型/真正浏览器断网恢复、持久 UI 默认或 Shell 的交付。

最新部件切片（2026-09-14）：发现策略已支持从现有 Product 源码构建/发布入口发布独立 capability，并直接选入 Agent；真实 Nomi ToolSearch → Node Service 调用、保存多策略冲突、非法结果、超时与停用后不回退均有定向证据。当前批次到此收口，不继续 UI 打磨。Product Role Provider、独立预览入口、schema 暴露策略/预算、大目录和全平台验收不在已完成范围；见 §27.14 和实施台账 §2.27。

五问复核的直接回答：**如果“这些工作”指 R1-JS 首期，不能解决全部五项问题；如果指 §27 的完整路线，则五项均有对应设计与验收，但仍有未细化的后续工作，当前代码也未全部实现。** 不新增 Rust 插件后端只减少一种执行后端的建设，不删除 UI、内部策略或完整 Agent Runtime 的开放目标。

前次补充核对（2026-09-14，五问与计划复核）：递归依赖图、公开入口守卫、Nomi 消费用途筛选和保存复用比较均已有实现；当时产品插件相关 30 项、adapter 22 项及 canonical `check` 通过。生成合同与正式 05 第二部分 §5.5 已同步，核心/Nomi 留存回归另见 §27.8 和实施台账 §2.15。这些记录不自动覆盖之后的工作区改动。

最近既有实施记录（2026-09-14）：已修复冷启动产品夹具并取得安装/保存/HTTP 查询证据，还接通附图及知识前缀场景的显式 Skill 消费。查询不启动 Agent/插件，命令按用户原文识别，不扫描检索内容，也不在自动续跑重放。详见 §18.4、§27.8 和台账 §2.16～2.17。系统页面仍固定装配、生产 Runtime 仍只有 Nomi、产品资源仍有固定 kind/单 binding 限制，五项完整需求均不能关单。阅读决策优先看本页五问表、§27.2 的 29 行标准和 §27.11 的开发顺序。

前次追问复核（2026-09-14）：只完善报告，不新增运行时代码或测试通过记录。重新静态核对上述 Runtime、页面与资源限制，以及 Skill 只读边界、direct/Role JS 调用路径。重点补清 §14.4 的双路径替换验收、§18.2 的当前使用影响和 §27.11 的交付优先级。

后续目标实施（2026-09-14）：Agent Context 已接入同一父作用域依赖调用链，包含 direct/Role、JS Host、产品 runtime lease 和 Nomi 求值身份；当前证据及扩展回归见实施台账 §2.18。这不是完整 Prompt 或非 Agent/资源/后台任务开放，29 行范围与 UI/Runtime 等未完成项不变。

2026-09-14 五问复核及后续实施：结合 `439bb385a` 与工作区既有改动，核对 Provider/Compiler、JS 依赖调用、包内 Skill、生产 Runtime、UI Router 和测试。历史实施与评估不混记；已验证切片及在途工作见 [实施台账](2026-09-14-agent-plugin-openness-implementation.zh.md) §2.1～2.18。Host 默认高并发的启动超时风险仍明确记录。当前证据见 §27.8，正式合同演进见 §27.9，Skill 剩余工作见 §18.4，JS-only 可行性与待证假设见 §27.12。

前次追问核查（2026-09-14，仅文档）：重新核对生产 Runtime、固定页面装配、资源绑定和包内 Skill 的实际边界；将已结束的 Context 消费测试结果同步到台账，不记作当次新跑测试。§27.11 补充后续大包的拆卡责任与交付物，避免用“策略继续开放”替代开发任务。结论仍是：完整路线针对全部五问，首期与当前实现都不等于全部解决。

五问核查及后续实施（2026-09-14）：核查发现 UI 产品测试的模型响应匹配冲突及 canonical 漂移；后续已按本轮用户输入区分响应，并断言取消前会话确为 running，最新版产品测试 1 项通过。重新生成合同并 `check` 通过，SDK 原生 MessagePort 3 项通过。历史失败和修复后的证据分别见 [实施台账](2026-09-14-agent-plugin-openness-implementation.zh.md) §2.19。系统页面仍固定装配，公开 Session events 仍返回不支持，不能据此核销完整页面替换；§27.11 的 P4 继续推进事件恢复、页面选择与联合验收。

后续 UI 事件切片（2026-09-14）：已复用现有用户事件总线和 `/ws`，接入按 Surface/会话/release 授权的实时投影、SDK 异步订阅及有界交付。原生 SDK 6 项、UI 8 项、后端授权 3 项及真实产品事件用例 1 项通过，具体范围见 [实施台账 §2.20](2026-09-14-agent-plugin-openness-implementation.zh.md#220-plugin-ui-实时事件复用现有总线不新增执行后端)。这是自然的 JS/UI 接缝，没有新增 Rust 插件后端或第二个 Session owner；仍未完成页面/Shell 选择与实际恢复体验。`observe` 是持久化消息对账，不是 token 重放，也不保证还原运行中的未持久化碎片。

本报告把**目标覆盖、版本安排、实现证据**分开。“新增 Tool”“替换系统部件”“替换整个引擎”不能互相代替；后续每个部件都必须同时满足用户选择、真实消费和统一组装，不能完成一个示范后就关闭需求 1 或需求 5。29 个环节的逐项标准见 §27.2。

当前增量可归为四组，均不代表五问整体关单：

- 包内 Skill 的精确只读正文/资源、`/skill:<精确ID>` 命令和补全已有证据（台账 §2.8～2.9）；冷启动产品验证见 §2.16，附图/装饰输入消费见 §2.17；其余来源及 shell/fork/hook 等模式仍未完成。
- 初始/动态 Context 已进入真实 Nomi 消费，`context_order` 经工作台保存冻结；不等于完整 Prompt、跨全部贡献预算或任意 middleware 已开放（§2.10～2.11）。
- Provider 的资源/特性需求可不同，`conflicts` 可独立声明并按实际组合检查（§2.7、§2.12）；递归图允许不同 `requires` 并区分公开贡献与内部依赖，核心/消费者定向回归与合同同步已有证据，产品及资源扩展的验证范围见本文 §27.8。
- 父作用域 Kernel 子调用及 JS Host/SDK、direct/Role adapter、父 runtime 租约复用已有实现与定向证据；Agent Context 后续接线见台账 §2.18。跨包夹具失败已修复，子包单独来源漂移也通过；Host 受控并发通过记录与默认高并发超时风险分开记录。不能据此宣称非 Agent 子调用、全部领域替换或历史计划迁移均已完成（§2.13～2.15/§2.18、本文 §14.5/§27.8）。

结论仍是**设计范围已覆盖，不等于全部可实施性已验证，更不等于已经解决**。§27.10 要求跨部件组合验收，§27.5 区分应删除的使用负担与必要的正确性边界，不能用若干独立演示或更多配置层数替代全栈组装。

本次修订目标：**用户可替换任意 Agent 主机部件的全栈插件平台**。这里的替换是用户选择不同实现，不是抢占系统 ID；“任意部件”包括 UI、Prompt、模型、Agent Loop、资源、存储和宿主服务，但不同部件适合在不同生命周期边界替换。

五项问题的决策摘要（当前状态不等于未来交付承诺）：

| 原始问题 | 是否纳入目标 | 当前尚不能宣称解决的原因 | 分期与详解 |
|---|---|---|---|
| 1. 同类 capability 并存，用户选择自己的替代内置 | 是；独立 capability 供 Agent 选择，共同契约的 Provider 供系统部件确定性替换，两条路径都保留 | 通用分发、选择/默认管理、冻结保存、不同资源/冲突/依赖及消费隔离已有切片；实际领域消费者、完整资源与历史兼容尚未全部接通 | R1 验证真实替换，后续逐部件完成；§14、§27.1 |
| 2. 用户插件替换系统 UI | 是；完整 Agent 页面、区域/结果呈现和 Shell，不限于主题或面板 | 显式选择、预设持久默认及会话前工作台配置/空会话入口见台账 §2.21/2.30/2.31；插件接管创建前欢迎页、模板传播、完整恢复体验、多视图和 Shell 仍未完成 | R1 Agent 页，R2 Shell/其余 UI；§15、§27.10 |
| 3. Agent 各环节开放，解决 Rust/JS 边界 | 是；默认 Nomi 的内部组件和整个替代引擎分别开放 | 初始 Tool/Context/Role 不能代替模型、历史/压缩、记忆、规划、编排、协作等；生产 Runtime 仍只有 Nomi | 29 行分别验收，部署服务另分层；§16～17、§27.2～27.4 |
| 4. Skill 完整装载与使用 | 是；这是可用性问题，不是可选美化 | 包内精确正文/资源、标准工具、只读命令及冷启动发现已有证据，附图/装饰输入已接线；脚本/fork/hook、其余来源、完整权限提示和历史制品保留/迁移未完成 | R1 支持模式闭环，R2 补完整模式；§18、台账 §2.8～2.9/§2.16～2.17 |
| 5. 更简单、开放、稳定的 Agent 组装 | 是；安装 → 选择/模板 → 保存并使用 | 统一主链已有基础，但仍有启动重编译、固定消费者和 Runtime 合同缺口；不能只增加入口而保留永久双轨 | 贯穿所有版本的退出条件；§19、§27.5 |

长期路线仍分三层：**R1-JS 交付首个真实部件替换、基础 Skill 和完整 Agent 页面；R2 补内部组件、替代引擎与 Shell；R3 分项交付部署服务替换。** 按用户最新取舍，这不再是要求所有后续实现都用 JS 完成的承诺：本期只安排 §27.13 筛选出的自然 JS/UI 实现，深层引擎需验证后纳入，原生密集及底层部署实现延期。若只批准 R1 的预算，就只能承诺 R1 的支持范围，不能承诺五问整体完成；已经验证的切片用于抵扣对应任务，不能用于跳过尚未接通的消费者。

**“任意”有必须明说的边界**：业务组件以可安装插件替换；存储、认证、Secret、监督等按启动依赖提供部署级选择；约束插件的最小信任根不能由该插件自行撤销。若要求所有这些都能由普通 JS 插件安装后接管，本方案不满足该字面目标，增加 Rust 插件也不能解决这个矛盾，见 §27.4。

**开发投入不能按五个小功能理解**：需求 1/5 横跨每个部件；不加 Rust 仍需改 Rust 宿主。§27.6 的 14～24 人周是原 R1-JS 基础预算，不是当前剩余工期，更不是五项完整目标的报价。Rust 与 JS 的执行后端层级见 §22；当前不新增 Rust 的范围、排期口径和下一批任务以 §27.3、§27.6、§27.11 为准。

§1～10、§12～13 描述原始基线；§11 总结架构判断；§14～21 为目标设计与研究证据；§22～26 将已接受的方向细化为开发计划；§27 核对原始需求并补齐遗漏的任务范围。暂不新增 Rust 插件的安排与覆盖判断以 §27 为准；§23～26 保留含 native 的对照计划。§20 的 A～G 只是主题路线，不是另一套需要重复执行的工作包。后文“建议/应当”不代表当前已有对应产品能力。原始研究轮次仅更新报告；后续实际代码改动与验证以 §27.8 和实施台账为准，不把未验证的工作区改动记为完成。

附录 A 保留工作区另行补充的统一实施基线，不代表本次研究已执行其中的代码、协议或数据库变更。§20 的数据迁移建议适用于需要保留真实用户数据的情况；开发数据重建等实施选择须按具体任务确认的范围执行。

## 1. 阅读范围与判断口径

本报告先回答两个现状问题，再研究上述五项目标架构问题：

1. 用户开发的插件目前可以替换系统什么级别、什么范围的能力；
2. 用户插件目前可以注入 Agent 的哪些环节。

报告区分以下四种能力状态：

- **已接通**：当前 NomiCore 产品链路已经能够完成安装/发布、组装、运行时调用；
- **部分接通**：部分链路已存在，但仍有运行时限制或缺少完整桥接；
- **合同/SPI 已存在**：代码和协议已经定义，但不能据此推断普通用户插件当前可用；
- **未开放**：由宿主、控制面或内核保留，用户插件当前不能替换。

另标注证据成熟度：**在途实现**表示工作区已有改动，但最新代码的测试、规范同步或实际产品消费尚未闭合，不能直接升级为“已接通”。历史测试只证明其执行时的代码，不自动覆盖后续修改。

本报告以当前产品实际启动的 **NomiCore** 为准。Fresh-v4 的 `AgentPlatform` 属于独立的未来迁移路径，不能直接当作当前用户插件能力。

## 2. 总体结论

> **历史基线（2026-09-13）**：本节的“当前”指原始评估时点。2026-09-14 工作区已新增用户 Context、Role/Provider 和包内只读 Skill 切片；最新五问结论以开头摘要及 §27.1/§27.8 为准，不应继续用本节概括全部现状。

当前重构已经兑现：

> **能力级插件化 + Agent Preset 显式组装 + Session 级精确锁定。**

最完整的用户扩展路径是：

- N1 JS Plugin 的 `FunctionTool`；
- M1 发布型插件的无资源 `Service Tool`。

因此，用户现在可以：

- 发布新的业务 capability；
- 实现具体的 Tool/action；
- 在 Agent Preset 中显式选择这些能力；
- 把能力放入初始 Tool 集或 on-demand Tool 集；
- 让 Agent 通过 Kernel 调用 JS Host 或 MiniApp Service Host；
- 以新的 capability 实现承担某个业务动作。

但当前还没有兑现：

- 整个 Agent Loop 的替换；
- 模型路由和决策循环的替换；
- Session authority、Snapshot authority 的替换；
- Kernel Registry 或 runtime supervisor 的替换；
- 面向普通用户插件的通用 TurnMiddleware、Event、Transport、Scheduler、BackgroundService；
- NomiFun Shell、系统主界面或 Agent UI 的替换。

这里的“替换”是**显式选择的能力替代**，不是透明覆盖：

```text
用户插件提供新的 capability ID
        ↓
用户在 Agent Preset 中显式选择
        ↓
该 capability 承担指定业务动作
```

用户插件不能直接覆盖系统内置 capability ID，也不能无配置地拦截所有同类调用。**这不是“系统能力不可替换”的目标约束。** 用户应当可以发布自己的 capability，在同一能力契约下与内置实现并列，并选择自己的实现作为默认或某个 Agent 的实现；当前通用替换链路尚未完成，见 §14。

## 3. 当前实际产品主机

当前产品入口明确构造 `NomiCoreApplication`：

- [`nomifun-app/src/lib.rs:60`](../../crates/backend/nomifun-app/src/lib.rs#L60)
- [`apps/web/src/main.rs:287`](../../apps/web/src/main.rs#L287)
- [`nomifun-app/src/desktop.rs:49`](../../crates/backend/nomifun-app/src/desktop.rs#L49)

Fresh-v4 的 `AgentPlatform` 目前是另一条主机路径。它发布的是内置 Rust registration，没有接入当前 NomiCore 使用的用户 JS Plugin `/api/plugins` 主链路。因此，Fresh-v4 中存在的通用组装 SPI，不能直接等同于当前产品已经开放给用户的插件能力。

## 4. 用户插件可以替换的级别与范围

> 下表保留原始基线，不是最新支持矩阵。Context 已接通初始/动态贡献与用户排序；通用 Role/Provider、选择/默认管理及 Skill 只读消费已有切片。完整目标仍未完成，当前范围见 §27.8 和实施台账 §2.1～2.11。

| 替换级别 | 当前状态 | 用户实际可以做什么 | 当前不能做什么 |
|---|---|---|---|
| 业务动作 / Tool | **已接通** | 发布新的 `FunctionTool`，实现查询、自动化、外部 API、设备或领域操作等业务能力 | 不能直接覆盖内置 capability ID |
| M1 Service action | **已接通** | 发布 Active Service，通过独立 Service Host 执行无资源 FunctionTool | 带非空 `resource_kinds` 的 Service action 当前运行时会拒绝 |
| Agent 工具组合 | **已接通** | 作为 initial 或 on-demand capability 进入 Preset | 安装插件不会自动修改已有 Agent |
| Agent Session 资源绑定 | **已接通，但宿主控制** | 使用宿主已经支持并绑定的 typed resource | 不能伪造 owner、resource ID 或 operation grant |
| Context / system prompt | **用户插件未开放** | 宿主批准的 bundled builtin 可以贡献上下文 | 普通用户 JS Plugin 不能直接写入 system prompt |
| ResourceProvider | **SPI 有，用户注入未闭环** | Kernel 有 acquire/release primitive | 当前没有普通用户 ResourceProvider 的 Agent admission |
| Skill | **部分接通** | package、catalog、compiler lock 已存在 | NomiCore 当前 Skill runtime 尚未完整接入 N1 Plugin Skill |
| MCP mapping | **部分接通** | mapping 可 materialize 并生成 lock | 当前 Nomi runtime 主要依赖显式 MCP Server binding |
| Turn Middleware / Event / Lifecycle | **内置能力已接通** | bundled、宿主批准的 Rust capability 可以参与 | 普通用户 JS Plugin 不能直接注入 |
| Agent Loop / 模型路由 | **未开放** | 无 | 不能替换 |
| Kernel / 控制面 / Snapshot authority | **未开放** | 无 | 不能替换 |
| 系统 Shell / 主 UI | **未开放** | M1 可以提供自己的插件 UI | 不能替换 NomiFun Shell 或 Agent 主界面 |

## 5. 已接通的 N1 JS Plugin Tool 路径

NomiCore 已经打通以下链路：

```text
插件安装、挂载、恢复
        ↓
JsKernelPluginAdapter
        ↓
动态 PluginRegistration
        ↓
Kernel Registry replace_all
        ↓
Catalog materialization
        ↓
Agent Preset Preview / Save
        ↓
精确 Snapshot
        ↓
Nomi Session Tool materialization
        ↓
Kernel → JS Host → 用户插件 JS action
```

相关实现：

- [`nomifun-app/src/router/state.rs:785`](../../crates/backend/nomifun-app/src/router/state.rs#L785)
- [`nomifun-app/src/router/state.rs:917`](../../crates/backend/nomifun-app/src/router/state.rs#L917)
- [`nomifun-app/src/router/plugin_platform.rs:478`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L478)
- [`nomifun-app/src/router/plugin_platform.rs:2152`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L2152)
- [`nomifun-js-kernel-adapter/src/lib.rs:263`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs#L263)

用户 Tool 只有在以下条件满足时才会成为 Agent 可用能力：

- capability kind 是 `Tool`；
- 至少存在一个 `FunctionTool` action；
- JS runtime 可用；
- 插件处于有效的 materialized mount；
- capability 通过当前宿主的可用性检查。

动态可用性判断位于：

- [`nomifun-app/src/router/plugin_platform.rs:2250`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L2250)

Tool 的 materialization 和调用位于：

- [`nomifun-ai-agent/src/plugin_tools.rs:1830`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L1830)
- [`nomifun-ai-agent/src/plugin_tools.rs:2980`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2980)
- [`nomifun-ai-agent/src/plugin_tools.rs:3337`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L3337)

## 6. M1 发布型插件的 Agent 路径

N1 安装聚合与 M1 发布聚合已经统一产品入口，但底层仍然保留不同的数据根和机器合同。M1 是用户开发插件可以使用的另一种运行形态，不应与 N1 JS package capability 简单混为同一个注册路径。

参考：

- [`docs/reviews/2026-09-13-plugin-unification-implementation.zh.md`](./2026-09-13-plugin-unification-implementation.zh.md)

M1 Agent 路径为：

```text
Active Release
        ↓
MiniApp Catalog publication
        ↓
Agent Preset 选择
        ↓
ResolvedMiniAppCapability
        ↓
materialize_miniapp_actions
        ↓
同一个 Nomi Tool session
        ↓
MiniApp Service Host
```

M1 action 只有在以下条件满足时才可执行：

- capability 支持 `Agent`；
- capability 支持 `MiniAppService`；
- MiniApp 已 enabled；
- 存在 Active Service；
- action 在 allowlist 中；
- action 是 `FunctionTool`；
- release、catalog digest、epoch 均未过期。

当前 M1 的主要运行时边界是：

> 零资源 Service Tool 已接通；带 typed resource binding 的 Service action 仍属于合同/目录层能力，当前调用会被拒绝。

拒绝逻辑位于：

- [`nomifun-plugin-platform/src/runtime/m1_application.rs:530`](../../crates/backend/nomifun-plugin-platform/src/runtime/m1_application.rs#L530)
- [`nomifun-plugin-platform/src/runtime/m1_application.rs:598`](../../crates/backend/nomifun-plugin-platform/src/runtime/m1_application.rs#L598)

## 7. Agent 的实际注入点

### 7.1 Catalog 与能力发现：已接通

插件可以进入：

- Catalog；
- Agent capability 列表；
- capability action 列表；
- required resource kind；
- dependency/conflict 声明；
- artifact/schema digest。

### 7.2 Agent Preset 组装：已接通

插件可以被显式放入：

- initial capability；
- on-demand capability；
- action allowlist；
- capability dependency；
- capability conflict；
- exact contribution lock。

相关编译逻辑：

- [`agent-control-plane/src/compiler.rs:424`](../../crates/backend/nomifun-agent-control-plane/src/compiler.rs#L424)
- [`agent-control-plane/src/compiler.rs:680`](../../crates/backend/nomifun-agent-control-plane/src/compiler.rs#L680)
- [`agent-control-plane/src/compiler.rs:1413`](../../crates/backend/nomifun-agent-control-plane/src/compiler.rs#L1413)

注意：插件安装不会自动修改现有 Agent Preset。用户需要重新 Preview、Save 和 Compile。

### 7.3 Session binding：已接通，但由宿主掌握权限

保存的 Snapshot 会在 Session 创建时重新编译和校验，并附加目标资源的 typed binding：

- [`nomi_core_session.rs:1020`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L1020)
- [`agent-kernel/src/compiler.rs:127`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L127)

插件不能自行伪造：

- resource owner；
- resource ID；
- operation grant；
- Snapshot authority；
- capability provenance。

### 7.4 初始 Tool 集：用户 Tool 已接通

用户 Tool 可以被编译进 initial capability，并直接进入 Nomi Registry，供模型在当前 Session 中调用。

### 7.5 延迟 Tool 集 / ToolSearch：用户 Tool 已接通

on-demand 用户 Tool 会先作为 deferred Tool 存在，经过 ToolSearch 激活后，在 turn boundary 进入可调用集合。

实现位于：

- [`plugin_tools.rs:2980`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2980)
- [`plugin_tools.rs:3337`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L3337)

### 7.6 System prompt Context：只有 bundled builtin 已接通

> **此限制已由后续实施部分解除。** 以下为历史断点说明；当前用户 JS Context 已能经同一冻结计划进入初始提示及每次主模型推理，并支持阶段内用户排序。完整 Prompt 管线、persona 等相对顺序与全部 middleware 仍未开放，见 §27.8、实施台账 §2.10～2.11。

Kernel 层存在 `ContextContributionFactory` 和 JS context proxy，但 NomiCore 的 Agent admission 只接纳：

- `ContributionSourceKind::PlatformBuiltin`；
- `PluginSourceKind::Bundled`；
- 宿主预先批准的 platform builtin。

因此：

> 普通用户 JS Plugin 的 `ContextContributor` 当前不能直接注入 Nomi Agent 的 system prompt，也不能作为普通用户插件通过 Agent Preset 使用。

实现和限制位于：

- [`plugin_tools.rs:2120`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2120)
- [`plugin_tools.rs:2231`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2231)
- [`plugin_platform.rs:2274`](../../crates/backend/nomifun-app/src/router/plugin_platform.rs#L2274)

### 7.7 Turn Middleware / Event / Lifecycle：可信 Rust builtin 已接通

宿主批准的 bundled Rust capability 可以参与：

- turn 前上下文；
- deferred lifecycle Tool；
- middleware turn context；
- lifecycle activation；
- EventSource、Transport、ResourceProvider、BackgroundService 等宿主生命周期。

但当前 admission 明确要求 bundled、PlatformBuiltin 和 typed host binding：

- [`plugin_tools.rs:330`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L330)
- [`plugin_tools.rs:2380`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs#L2380)

这不是普通用户 JS Plugin 的开放注入点。

## 8. N1 合同支持但当前未完整开放的能力

> **历史准入状态提示**：下文的 Role Contracts/Providers 禁令是原始基线，不是现行 N1 的一概禁令。当前 `plugin_n1.rs` 已有对应声明校验，JS adapter 已支持精确实现映射；具体资源/依赖和实际领域消费者仍有限制，见 §14.5/§27.8。不能据原禁令否定已完成的替换基础，也不能据解除禁令推断 Browser/Computer 全链路已可替换。

N1 Plugin Package v1 的合同允许声明：

- `Tool`；
- `ContextContributor`；
- `ResourceProvider`；
- `Skill`；
- MCP tool mapping。

同时明确禁止：

- Role Contracts；
- Role Providers。

JS v1 也不能直接声明 Browser/Computer Role Provider、通用 EventSource/EventConsumer、TurnMiddleware、Transport、Scheduler、BackgroundService、Agent loop 或 Runtime authority。

合同定义位于：

- [`plugin_n1.rs:3069`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs#L3069)
- [`plugin_n1.rs:3119`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs#L3119)
- [`plugin_n1.rs:3145`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs#L3145)

### 8.1 ResourceProvider

Kernel 已有完整的 acquire/release primitive：

- [`registry.rs:749`](../../crates/backend/nomifun-agent-kernel/src/registry.rs#L749)
- [`registry.rs:1000`](../../crates/backend/nomifun-agent-kernel/src/registry.rs#L1000)
- [`js-kernel-adapter/src/lib.rs:623`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs#L623)

但当前 NomiCore 没有发现把用户 ResourceProvider 自动装配成 Agent 可选择的独立 provider 的完整 admission/acquire 路径。

用户 Tool 可以声明 `resource_kinds`，但资源种类仍由 NomiCore 固定解析器控制，例如：

- `workspace`；
- `knowledge_base`；
- `mcp_server`；
- `robot`；
- `miniapp`。

相关代码：

- [`nomi_core_resource_bindings.rs:126`](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs#L126)
- [`nomi_core_resource_bindings.rs:295`](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs#L295)

### 8.2 Skill

> **原始装载断点已在只读范围修复。** 下文保留问题成因；当前包内精确正文/引用资源、标准 Skill 工具、显式命令及补全已有验证。shell/fork/hook 等执行模式、其余来源与历史制品保留仍未闭环，详见 §18、实施台账 §2.8～2.9。

Kernel 已经支持：

- Skill materialization；
- `requires_capabilities` 校验；
- `ResolvedSkillLock`；
- body digest 和 provenance 锁定。

相关编译逻辑：

- [`agent-kernel/src/compiler.rs:879`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L879)
- [`agent-kernel/src/compiler.rs:1413`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L1413)

但当前 NomiCore 的传统 Skill runtime 仍主要使用 `SkillPaths`、workspace 和 `SKILL.md`：

- [`skill_resolver.rs:41`](../../crates/backend/nomifun-conversation/src/skill_resolver.rs#L41)
- [`service.rs:4398`](../../crates/backend/nomifun-conversation/src/service.rs#L4398)

本次进一步复核发现：`nomi_core_agent_projection::project_internal` 已经把
`snapshot.content.skill_locks` 中的 Skill ID 投影到 `included_skills`，随后
Conversation 将其转换为 `agent_enabled_skills` 和 `extra.skills`。因此不是完全没有接线，
而是**精确 Skill 锁在运行时投影中降为名称列表**，尚未看到按该锁装载插件制品正文、
引用资源并交给 Nomi 执行的完整消费路径。详见 §18 及其中新增证据。

因此，当前只能确认：

> Skill 的 package/catalog/compiler 合同已经具备，但 N1 Plugin Skill 到当前 Nomi runtime 的完整装载和注入闭环尚未形成。

### 8.3 MCP

N1 Plugin 可以声明 MCP mapping，Kernel 可以：

- 校验 mapping 与 capability 一致；
- materialize 到 catalog；
- 编译 `ResolvedMcpToolLock`。

但当前 Nomi runtime 的 MCP 使用路径仍然主要是：

- Session 显式绑定 MCP Server；
- `mcp.connect`；
- `mcp.tool_proxy`；
- `mcp.resource`；
- native MCP manager/tool registration。

相关代码：

- [`materialize.rs:1000`](../../crates/backend/nomifun-agent-kernel/src/materialize.rs#L1000)
- [`agent-kernel/src/compiler.rs:1450`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L1450)
- [`nomi_core_agent_projection.rs:467`](../../crates/backend/nomifun-app/src/router/nomi_core_agent_projection.rs#L467)
- [`nomi_core_session.rs:619`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L619)

目前不能宣称“N1 用户插件 MCP mapping 已自动成为当前 Nomi Agent 的可调用工具”。

## 9. 当前完整调用链

```text
N1 插件安装 / M1 插件发布
        ↓
Catalog materialization
        ↓
Agent Preset initial / on-demand 选择
        ↓
依赖、冲突、资源、allowlist 校验
        ↓
immutable Snapshot
        ↓
artifact / schema / provenance / digest lock
        ↓
Session typed resource binding
        ↓
Nomi Tool / deferred Tool / builtin Context materialization
        ↓
ToolSearch 或生命周期激活
        ↓
Kernel authority check
        ↓
JS Host 或 MiniApp Service Host 执行
```

## 10. 版本、升级与替换规则

当前模型不是运行时热插拔覆盖，而是编译后锁定：

- 插件安装不会自动改变已有 Agent；
- Agent 必须重新 Preview/Save/Compile 才会使用新插件；
- Snapshot 会锁定 capability provenance、artifact digest、schema digest；
- 已创建的 Session 不会因为插件升级而透明漂移；
- M1 还会校验 active release、release digest、catalog digest 和 epoch；
- 插件升级后，旧 Snapshot 与新版本之间需要重新通过编译和 admission 校验。

因此，当前更接近：

> **不可变 Agent 配置 + 可验证 capability 版本锁**

而不是：

> **插件覆盖正在运行的 Agent 主机实现**

## 11. 本次问题的决策建议

原版把若干方向列为“是否开放”。按本次全栈平台目标，建议不再以封闭为默认：

1. **开放同类能力替换**：保留独立 capability ID，用共同契约选择 Provider；不建设 ID override 系统。
2. **开放 UI**：插件既能贡献区域，也能替换 Agent 页面和工作台 Shell；底层使用统一命令/查询/事件接口。
3. **开放 Context、Resource、Skill、MCP、Middleware、Event、Transport、Scheduler、Service**：按接口和授权准入，逐步取消以 bundled 来源作为功能资格的硬编码。
4. **开放模型、上下文管线与整个 Agent Runtime**：既能替换 Nomi 内部明确的策略组件，也能在同一会话协议后使用另一套完整引擎。
5. **可选支持 Rust 作者**：若投入原生插件，优先支持共用协议的 Rust 进程外插件；Wasm 作为有实测收益时增加的执行后端；不把原生 Rust 动态库作为公共插件 ABI。暂不新增 Rust 插件不阻塞其他开放工作，见 §27。
6. **尽快闭合 Skill**：保留文件来源，但统一运行时装载器，不能把“已锁定 ID”当成“已装载锁定内容”。
7. **缩短组装链**：一个能力目录、一套 Provider 绑定解析、一份冻结执行计划；Preview/Save/Open 不分别维护不同的解析规则。
8. **区分替换实现与伪造权限**：资源、存储、认证和宿主服务的实现可以被部署者替换；当前运行中的普通插件不能自行替换负责约束自己的信任根。全量宿主替换通过显式启动配置/部署完成，不要求热替换。

以上架构方向已获用户接受；本轮进一步制定实施计划，不代表重构已经完成，也不在本次文档任务中自动开始代码实施。

## 12. 一句话基线

当前系统已经是一个可以把用户开发的业务能力显式装配进 Agent 的插件平台；最成熟的是 **N1 JS Tool 和 M1 无资源 Service Tool**。它还不是一个允许用户插件替换 Agent 主机、运行时内核、Prompt 管线和控制面的全栈 Agent 平台。

## 13. MiniApp 与 Plugin 合并完整性复核（2026-09-13）

### 13.1 修正后的结论

上一期工作**没有把 MiniApp 从代码和领域模型中彻底移除**。已经完成的是：

- 产品入口统一到 `/plugins`；
- 公共 HTTP 路径统一到 `/api/plugins`；
- 创建、导入、工作区和运行入口统一到 Plugin 语义；
- `nomifun-miniapp-platform`、`nomifun-plugin-service` 的实现物理合并到 `nomifun-plugin-platform`；
- 对外 DTO 和顶层 CLI 基本改用 `plugin` / `plugin_id` / `plugins`。

但尚未完成的是领域层和运行时层的统一。更准确的表述是：

> **已完成产品入口统一、公开 API 统一和 crate 合并；未完成身份模型、Agent Snapshot、Catalog provenance、数据库聚合、运行时角色和内部命名的统一。**

因此，不能把本次结果描述成“系统中已经不存在 MiniApp 概念”，只能描述成“MiniApp 已经被纳入 Plugin 产品入口，但底层仍有独立的 MiniApp 运行分支”。

### 13.2 仍在生效的结构性残留

以下不是单纯的注释或历史字符串，而是会影响当前编译、快照、准入或运行时行为的活动代码：

1. **Agent Snapshot 仍有两套 capability 集合。**\
   `ResolvedMiniAppCapability` 与 `ResolvedCapability` 并列存在，Snapshot 仍有
   `initial_miniapp_capabilities` 和 `on_demand_miniapp_capabilities`。证据：
   [`preset.rs:470`](../../crates/backend/nomifun-agent-contracts/src/preset.rs#L470)、
   [`preset.rs:646`](../../crates/backend/nomifun-agent-contracts/src/preset.rs#L646)。

2. **Session 仍使用独立的 MiniApp materialization 和 invoker。**\
   当前会分别调用 `materialize_miniapp_actions`、`with_miniapp_actions`，
   并构造 `NomiCoreMiniAppToolInvoker`，而不是将 M1 作为统一 Plugin capability
   的 runtime role/profile。证据：
   [`nomi_core_session.rs:563`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L563)、
   [`nomi_core_session.rs:591`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs#L591)。

3. **Catalog 和 provenance 仍把 MiniApp 当作一等来源。**\
   `ContributionSourceKind::MiniAppActiveRelease`、MiniApp catalog publication
   和对应生命周期分支仍然存在。这意味着 Agent 能力来源仍不是“Plugin + role/profile”，
   而是包含一个独立的 MiniApp source kind。

4. **数据库仍以 MiniApp 为运行时数据根。**\
   当前运行时仍直接使用 `miniapp_products`、`miniapp_projects`、
   `miniapp_releases`、`miniapp_catalog_publications` 等表，并由
   `SqliteMiniAppM1Repository` 访问。历史 migration 文件可以保留，但活动 repository、
   identity 和外键仍是 MiniApp 领域模型，不能视为已完成统一。证据：
   基线文件 `crates/backend/nomifun-db/migrations/072_miniapp_m1_data_root.sql:8`（当前对应文件为
   [`072_plugin_runtime_data_root.sql`](../../crates/backend/nomifun-db/migrations/072_plugin_runtime_data_root.sql)）、
   基线文件 `crates/backend/nomifun-db/src/repository/sqlite_miniapp_m1.rs:108`。
   本轮排期补充时，该文件已在并行工作区中改名为
   [`sqlite_plugin_runtime.rs`](../../crates/backend/nomifun-db/src/repository/sqlite_plugin_runtime.rs)；
   此处保留原基线判断，不把文件改名本身作为领域统一已完成的证据。

5. **`plugin-platform` 主要是承载旧实现，领域类型尚未重命名。**\
   `miniapp_m1.rs`、`MiniAppReleaseV1Manifest`、`MiniAppBridgeSession`、
   `MiniAppServiceStorageDescriptor` 等仍是当前 runtime 的核心类型。也就是说，
   这是“旧 crate 合并 + 外部入口改名”，不是“领域对象统一”。

6. **后端内部路由和状态仍误导为 MiniApp。**\
   即使公开路径已经是 `/api/plugins/runtimes`，内部仍有
   `miniapp_m1_read_routes`、`miniapp_m1_write_routes`、`states.miniapp` 等名称。
   证据：
   [`routes.rs:852`](../../crates/backend/nomifun-app/src/router/routes.rs#L852)、
   [`plugin_runtime.rs:55`](../../crates/backend/nomifun-app/src/router/plugin_runtime.rs#L55)。

7. **UI 类型系统和 Agent 资源选择器仍暴露 MiniApp。**\
   `PluginRuntimeId` 仍定义为 `EntityId<'miniapp'>`，Agent 资源种类仍有
   `miniapp`，并且资源选择逻辑仍按该 kind 分支。证据：
   [`ids.ts:82`](../../ui/src/common/types/ids.ts#L82)、
   [`ids.ts:143`](../../ui/src/common/types/ids.ts#L143)、
   [`AgentResourcePicker.tsx:108`](../../ui/src/renderer/components/agent/AgentResourcePicker.tsx#L108)。

8. **Wave 3 内置 Agent 能力仍以 MiniApp 命名并注册。**\
   `miniapp.read`、`miniapp.edit`、`miniapp.publish`、`miniapp.serve`
   仍是独立 capability ID 和 host 分支。这会继续把 MiniApp 暴露为 Agent 的能力类别，
   而不是 Plugin 的一种 role/profile。

### 13.3 仍会造成误导的文档和脚本

当前仍有文档或验收材料描述已退出的产品入口或 crate，例如：

- `docs/architecture/backend-crates*.md` 仍引用 `nomifun-miniapp`；
- `docs/architecture/frontend*.md` 仍写 `/mini-apps`；
- `docs/reference/api-overview*.md` 仍写 `/api/miniapps`；
- `README.md`、`CHANGELOG.md` 仍把 Agent Mini Apps 当作独立产品；
- `docs/guides/presets.zh.md` 仍把 Plugin 和 MiniApp 写成两种能力来源；
- Windows 验收脚本仍存在以 MiniApp 为独立产品的命名和 scope；
- 生成的 preset/fixture 仍包含 `miniapp.*` capability ID。

这些内容会让开发者、测试人员和后续 Agent 得出错误结论：以为 MiniApp 仍是
当前公开产品，或者以为 Plugin 与 MiniApp 是两套并列扩展机制。

### 13.4 不应机械删除的残留

以下命中需要建立 allowlist，不应直接全局替换：

- 已发布的 bridge handshake / MessageChannel 事件名，例如
  `nomifun-miniapp-bridge-*-v1`；这是版本化传输协议；
- 历史 SQL migration、历史 spec、历史 changelog 中的原始名称；
- 测试 fixture 中仅用于模拟旧文件的 `miniapp.html`；
- 与 MiniApp 无关的 `MiniMap` 等普通词。

但协议兼容并不能解释 `ResolvedMiniAppCapability`、Agent resource kind、
数据库 repository、`miniapp.*` capability ID 这些当前活动领域对象；这些仍需要迁移。

### 13.5 当前真实架构形态

当前 Agent 能力来源仍近似如下：

```text
普通 Plugin capability
MiniApp Active Release capability
MCP capability
Builtin capability
```

如果上一期需求的验收标准是“用户只面对 Plugin，Agent 内部也只使用统一
Plugin capability，并通过 Service / Surface / Tool 等角色表达差异”，那么当前结果
尚未达标。建议下一阶段先定义 canonical 对象（例如 `PluginProduct`、
`PluginRelease`、`PluginService`、`PluginSurface`），再依次合并 Snapshot、
provenance、resource kind、数据库 repository、runtime role 和内部命名。

## 14. 问题一：同类 capability 的并存、选择与替换

### 14.1 结论：认同目标，但要区分“功能相似”与“契约可替换”

用户插件应能发布一个或多个 capability，进入与内置能力相同的目录，供 Agent 和其他消费者选择。插件是分发单位，capability 是消费单位；一个插件不必被限制成一个 capability。

需要同时支持两种用法：

1. **作为独立工具使用**：两个搜索插件可以都进入 Agent 工具集，由模型根据描述选择；这是当前 Tool 路径最接近的能力。
2. **作为某个部件的替代实现**：用户把默认搜索、上下文构造器、模型路由器或 Agent Runtime 指向自己的插件；系统对该部件的调用都通过选定实现。这需要稳定契约和绑定，不是仅把两个工具都展示给模型。

`CapabilityKind::Tool` 相同、展示名称相似、甚至输入 JSON Schema 相同，都不足以证明可替换。契约还应定义输出、错误、流式/取消行为和必要语义。例如“网页搜索”和“本地向量检索”都能返回文本，但不一定符合相同的搜索契约。

### 14.2 复用现有 Role/Provider，不新增平行替换框架

仓库已经有 `RoleContractManifest`、`RoleProviderContribution`、`InstallationRoleBinding`，Preset 也已有 `system_role_provider_overrides`。这些是最接近目标的基础，不需要再叠加一套独立的 Slot Registry、Override Registry 或 Provider Marketplace：

- [`package.rs:65`](../../crates/backend/nomifun-agent-contracts/src/package.rs#L65)：Role 契约、Provider 引用、安装级绑定。
- [`preset.rs:186`](../../crates/backend/nomifun-agent-contracts/src/preset.rs#L186)：Preset payload 中的 Provider override。
- [`plugin.rs:183`](../../crates/backend/nomifun-agent-kernel/src/plugin.rs#L183)：已解析的 Role member 与 Provider mount 上下文。
- [`plugin_n1.rs`](../../crates/backend/nomifun-agent-contracts/src/plugin_n1.rs)：原始基线的 N1 v1 明确拒绝用户发布 Role/Provider；2026-09-14 已扩展准入、精确映射与 JS typed export，并补候选目录/Agent 选择保存及产品默认管理；各领域完整替换仍未完成，见 §27.8。

建议约定：**Role 是一个可替换部件的契约，Provider 是插件对该契约的实现绑定；capability 仍是统一的可发现、可选择贡献。** Role 可包含一个或多个相关 member，用来表达同一个有状态服务的完整接口，而不是每个方法都创建一个插件层。

现有 Role member 使用契约侧的 capability 引用。推广时，允许用户的独立 capability 显式导出对应 member；该映射只在 Provider 注册处声明一次。系统 member 身份保留，用户实现身份也保留，两者不能通过复制相同 ID 混为一体。现有合同若不足以表达此映射，应修订该合同，而非外挂第二套解析器。

示意（下列 ID 仅为设计示例，不是当前已有接口）：

```text
契约 search.web/v1
    ├─ 内置实现：platform.search.web
    └─ 用户实现：acme.search.web

用户选择 acme.search.web
    → 编译 Role/Provider 绑定并锁定具体制品
    → Agent 的该契约调用落到 acme.search.web
```

独立的新能力可以先只发布 capability，不要求开发者先制定通用 Role。只有被系统或其他插件按契约依赖、需要多实现替换时才引入 Role。用户也应可发布自己的命名空间契约，系统保留的只是系统契约命名空间，不是契约创作权。

### 14.3 选择规则应少且可预测

- 单实例部件按 **Agent 显式选择 > 安装级用户默认 > 官方默认** 解析；官方默认仅是创建时的默认值，不是运行失败后的暗中回退。
- 多实例贡献，如工具集合、Context 列表、观察者，按用户选择的集合及明确顺序组装；不依赖加载先后或全局数字优先级。
- 同一单实例契约选择多个实现时，要求用户选择一个；需要轮询/路由时，使用一个明确的路由 Provider，它再消费多个实现，而不是隐式竞争。
- Agent 可在已授权、已锁定的工具/Provider 候选池内动态选择；不能借“模型自动选择”在 turn 中安装插件、改主机或提升授权。
- 依赖尽量指向契约；确实依赖某个具体实现时才精确依赖 capability。否则“用户替换 A 后，B 仍硬编码调用内置 A”会使替换名存实亡。
- 选择新默认影响新编译/新会话。已有 Session 继续使用冻结绑定；状态迁移另行显式完成。更改 UI 外观不必重建 Agent Session。

### 14.4 本项验收不是“安装成功”

同一契约至少有一个内置和一个用户实现，用户选择后，真实请求必须只调用选定 Provider；测试同时覆盖默认继承、Agent override、契约不兼容、旧会话不漂移、卸载后可解释失败。调用方不能再认识内置实现的具体 ID。这样才从“插件增加工具”升级为“插件替换部件”。

针对用户的原话，验收还应明确分成两条，而不是把 Role 当作所有插件的额外门槛：

| 用户操作 | 应观察到的结果 | 不能算通过的情况 |
|---|---|---|
| 将独立的用户 capability 加入 Agent；只想用自己的同类工具时，不选择内置工具 | 用户 capability 以自己的 ID 进入实际候选与调用链；是否同时保留内置工具由显式选择决定 | 安装后只在插件管理页可见；或用户已排除内置工具，运行时又隐式补入 |
| 把系统某个部件绑定到用户 Provider | 该契约的实际消费者执行所选实现；内置实现仍可安装、仍保留原 ID，但不再承担该绑定的请求 | 两个实现都在 Catalog 中，却仍硬编码调用内置；或仅修改 Prompt 要求模型“尽量用我的” |

因此，不覆盖内置 ID、不必卸载内置包，也应能达到替换效果。普通 Tool 的模型选择和系统部件的确定性绑定分别测试；“内部依赖仍可供选中 Provider 使用”与“把该依赖作为模型可直接调用的工具暴露”也须分开，避免替换表面成功、实际仍绕回旧路径。

### 14.5 对外契约兼容，不等于要求内部实现完全相同

这是需求 1 与需求 5 必须共同满足的条件。用户实现可以有自己的配置、内部依赖、资源需求和支持平台；如果要求它们全部复制内置实现，虽然两个 Provider 能通过测试，却没有充分实现开放式组装。

例如，一个搜索契约的内置实现调用远程 API，用户实现读取本地索引。两者可以遵守同一输入、输出、错误和取消契约，但凭据、索引资源和依赖不同。应区分：

- **对外保证**：Role member 的输入/输出、必要语义、副作用约束、流式与取消协议。首期可以要求显式实现同一版本的契约，不必发明通用 JSON Schema 子类型推断；不兼容时由作者提供明确适配。
- **实现需求**：Provider 自身的配置、依赖、资源和运行环境，由贡献声明表达。选择 Provider 后，现有 canonical Compiler 将这些需求纳入同一次依赖闭包、冲突检查和精确锁定；发现循环或缺件时一次解释清楚，不新增第二个求解器。
- **实际授权**：声明依赖不是授予权限。新的资源/操作需求必须满足当前授权，必要时请用户确认；内部依赖不因此自动成为模型可直接调用的 Tool。Provider 所需能力也不能绕过已有 operation grant。
- **可用性**：契约、Provider member 和映射实现的平台/host surface/runtime 要求须同时满足；不能只检查契约侧声明，映射后就绕过实现限制。

原始 Role 切片对依赖、资源等采用严格相等检查。已验证的后续切片中，`conflicts` 按台账 §2.12 开放为实现独立声明，并在实际组合中检查；Tool/Context 的私有资源需求可以不同，runtime feature 也不再要求清单相等。`requires` 相等检查已移除，递归图、消费筛选、公开入口和父调用冻结边校验已接线；核心/消费者回归与生成合同、正式规范同步已有证据（§27.8、台账 §2.15），不再列为全未验证。`ResourceProvider` 的输出 kind 仍须一致，契约规定的序列化目标资源不能被实现删掉，member 的需求声明须与映射实现一致。这些对外类型/身份约束有必要保留，不属于应全部删除的限制。

冲突检查保留 façade 的共同契约约束，并检查所选实现及实际使用的隐式资源工厂，可识别跨 Role 和反向指向内部实现的冲突。未选中候选/未消费成员不参与；仅属于内置实现的私有限制应放在实现中，而非写入所有 Provider 共用契约。无改动保存和具体非 Agent 准入复用同一检查；选择默认 Provider 不代表所有成员必须同时运行。检查所用集合不进入 enabled/allowlist/policy，不能据此声明内部依赖执行已授权。

`apply_role_requirements` 将所选实现的资源需求投影到原有 resolved capability/authority policy，并收集实际消费的资源工厂及实现的 runtime feature；编译与分发复用 `role_resource_members`，不把内部资源导出追加成公开 Tool。**这属于不同资源需求的局部进展，不是完整异构依赖已经解决。** 最新 helper 重构及无改动保存的需求投影比较已完成定向回归，05 §5.5 也已同步；真实 Node 验证 Tool/Context/非 Agent operation 消费资源参数，但不证明产品任意 resource kind、多 binding 或全部领域已经开放，详见 §27.8。

剩余演进归入 P2/P7 的绑定/资源与 P8 的计划完整性工作：递归依赖解析、消费用途、授权验证与放宽相等限制按同一切片验收；已取得的核心/消费者及合同证据保留回归，不重复建设。继续交付领域消费者、产品可扩展资源绑定、其余子调用类型和历史兼容。始终保留“同契约、不同内部 capability 依赖的实现可选择”和“新增未授权需求不能执行”这两个相反方向的验收。

**进一步重读代码后，依赖工作应按一次跨层合同演进验收，而非局部 validator 修改。** 最新 [`AgentPresetCompiler::compile`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs) 调用 [`compiler_dependencies.rs`](../../crates/backend/nomifun-agent-kernel/src/compiler_dependencies.rs)，把所选 Provider 和隐式资源工厂的需求纳入递归图，在原能力记录冻结用途与依赖边。[Nomi 消费者](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs) 的 Tool、初始/动态 Context 及生命周期投影已改用 `contributions()`；公开 Tool、direct/Role Context 入口已有 `require_contribution`；无改动保存已重算同一选中图并比较记录。这纠正了上一轮“这些接线尚未实现”的状态，但产品全链路、其他消费投影与旧计划切换仍须验证，具体证据及缺口见 §27.8。

建议在现有计划内明确三种不同事实，而不是新建三套 Registry：

| 事实 | 应由谁决定 | 不能推导出的权限或行为 |
|---|---|---|
| 某实现及其依赖属于本次冻结计划 | canonical Compiler 根据选择、依赖和精确制品解析 | 被纳入计划不等于模型可直接调用，也不等于可读写资源 |
| 某贡献向当前消费者开放 | 用户选择及契约规定的消费用途，由同一计划表达 | 仅供 Provider 内部使用的 Tool/Context 不自动进入模型工具或 Prompt |
| 某次调用允许哪些操作/资源 | 现有 authority 根据调用来源、授权范围和当前绑定校验 | 插件声明依赖、隐藏 Tool 或持有 capability ID 都不是授权凭据 |

内部实现和对外使用应共享同一精确能力记录；在 canonical 合同中表达用途/依赖关系，执行和 UI 投影消费这些关系，不另复制一份“JS 内部能力快照”。当前 `ResolvedCapability` 的 `consumption`/`dependency_refs` 已有主要消费者和图结构验证：精确边、重复边、循环、allowlist 一致性及内部节点从公开根可达性均被检查；生成 schema/摘要已包含此演进且 canonical `check` 通过，但工作区同步不等于产品发布。同一 capability 同时被用户显式选择与内部依赖时保持公开用途。选择闭包考虑 façade、所选实现及其递归依赖，复用原 Role resolver；不锁入所有未选中候选。对外副作用保证仍需满足；实现新增资源需求由一次清楚的授权差异说明处理，不要求用户把每个内部依赖手工添加为 Tool。

**兼容读取不等于语义迁移。** 旧记录缺少新字段时按 `Contribution`/空边反序列化，以保留旧序列化摘要；不能据此推断过去所有间接依赖本来就应公开，也不能凭空补出选中 Provider 的执行图。新保存须重编译，旧 Session 保留原锁定事实；当前产品打开会话仍重编译并比较完整 envelope，不一致可能明确拒绝，而不是已保证所有旧会话可继续。发布前应选择并验证兼容消费或显式迁移方案，不能默默改写旧 Snapshot，亦不直接删除现有校验。

这还涉及实际执行。最新 [`NodeRoleExport`](../../crates/backend/nomifun-js-kernel-adapter/src/role.rs) 的 Agent Tool 路径在验证精确锁后进入 `invoke_scoped`，与 direct Tool 共用父作用域依赖 caller。外层调用仍直接执行所选实现；实现发起的独立子调用才重新进入 Kernel，不能把这两种行为混成同一次 façade 二次分发。非 Agent operation 尚未因此获得该 API。这里限制的是平台受管调用；普通 Node 的 OS 权限问题仍按 §17.4 单独处理。

最新工作区已有 [`CapabilityDependencyCaller`](../../crates/backend/nomifun-agent-kernel/src/dependency_call.rs) 和 [`invoke_shared`](../../crates/backend/nomifun-agent-kernel/src/registry.rs)：父调用提供身份与冻结计划，只允许所选实现声明的直接依赖，子调用继续走原 Kernel 校验；父调用结束后保留的 caller 不再可用，并检查祖先来源/Provider 是否仍有效。JS Tool 与 Agent Context 均已接入 `dependencies.invoke({ capabilityId, actionId, callKey, input })`，由 Host 绑定父请求、精确 Mount/generation 与截止时间，不接受插件自报 owner、Session、Snapshot 或资源权限。Context 保留真实 access 证据，不伪造 Tool action；详见台账 §2.18。非 Agent 操作、资源工厂和后台任务仍未获得同等 API。稳定派生的调用标识不等于持久化副作用去重，取消也不撤销已提交效果。

JS 接线还涉及一个具体生命周期风险：父调用持有 runtime 读租约时，子调用若在已排队的切换写锁后重新申请读租约，会形成相互等待。当前 [`RuntimeBoundExtensionHost`](../../crates/backend/nomifun-app/src/router/plugin_runtime_host.rs) 已通过仅作用于受管 callback 的 task-local 复用同一 Host 的父级 [`RuntimeUseLease`](../../crates/backend/nomifun-js-runtime/src/authority.rs)；其他 Host 和脱离调用的任务不能借用。原 supervisor 的 pending request/service task 所有权继续处理结束、取消和截止时间，没有另建 runtime 管理器。产品 Host 的五项定向测试含真实 Node、写锁提前排队和取消释放场景；后续 adapter 同包/跨包与 Host 受控并发全套也已通过。默认高并发的启动超时风险及准确验证范围见 §27.8、台账 §2.14，不以这些通过记录宣称任意压力或其他消费类型均已完成。此项属于原 P2/P7 成本，不是增加 Rust 插件才能解决的问题。

该工作包应在一条真实 JS 替换链上同时验收：不同内部需求能够执行；内部 Tool/Context 不被隐式暴露；依赖缺失、冲突或循环在组装时说明；未获授权或已撤销的子调用被拒绝；实现依赖升级后新保存更新锁而旧 Session 不漂移。编译合同、消费投影、调用授权与回归测试缺一项，都不能关闭“异构实现可替换”。无需为此新增 Rust 插件后端。

## 15. 问题二：系统 UI 插件化的可实施性

### 15.1 结论与现有基础

**可实施，且不需要改用 Rust 编写 UI。** 当前前端本来就是 React/TypeScript；Rust 主机与 JS UI 通过版本化数据接口协作，没有天然的语言障碍。工作量主要在拆开 UI、业务状态和桌面特权，而不是渲染技术。

已有基础及其边界：

- [`package.rs:419`](../../crates/backend/nomifun-agent-contracts/src/package.rs#L419) 已定义 `UiContribution` 和 `CapabilityConsumer::Ui`，但枚举存在不代表产品已经支持 UI 替换。
- [`PluginRuntimeSurfacePanel.tsx:314`](../../ui/src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.tsx#L314) 有窗口/nonce 校验及 MessageChannel 握手；[`同文件:522`](../../ui/src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.tsx#L522) 用 `allow-scripts allow-forms` 沙箱 iframe 展示插件。
- 原始基线 `crates/backend/nomifun-agent-contracts/src/miniapp_m1.rs:1933` 的 Bridge target 是 Host KV 或 Service 调用，不是完整的 Session/导航/主题 SDK；工作区统一改造后的对应声明见 [`plugin_runtime.rs:1887`](../../crates/backend/nomifun-agent-contracts/src/plugin_runtime.rs#L1887)，仍是这两类 target。此处只核对声明，不代表统一改造或 UI 会话链已验收。
- [`ProtectedAppRuntime.tsx:18`](../../ui/src/renderer/components/layout/ProtectedAppRuntime.tsx#L18) 已把鉴权、深链、桌面同步等运行职责与可见布局分离；[`Router.tsx:13`](../../ui/src/renderer/components/layout/Router.tsx#L13) 的页面仍通过静态 import/route 组装。
- [`chatPort.ts:45`](../../ui/src/renderer/pages/creativeStudio/agent/chatPort.ts#L45) 已有不依赖 HTTP/IPC 的流式会话 port，是可参考的局部拆分，不应直接冒充通用插件 SDK。

### 15.2 UI 分层开放，不停留在主题和小挂件

| 范围 | 用户可以替换什么 | 最小改造与建议 |
|---|---|---|
| 主题与展示贡献 | 主题变量、图标、工具结果卡片、附件预览 | 主题配置可纯数据化；复杂渲染走 Surface，不开放任意宿主 DOM 修改 |
| 页面内区域 | 导航项、侧栏、Inspector、消息区域、输入区 | 为有独立产品意义的区域定义契约；多实例贡献与单实例替换分开 |
| 完整 Agent 页面 | 自定义聊天 UI、计划视图、Agent 组装界面 | Session command/query/event API；完整覆盖发送、流式输出、取消、历史和错误 |
| 工作台 Shell | 整体布局、导航、首页、页面组合 | 安装级选择一个 Shell Provider，内置 Shell 也实现同一接口 |
| 独立客户端 | 用户自己的 Web/桌面客户端 | 复用同一版本化应用 API；不要求导入 NomiFun React 内部组件 |
| 桌面容器/启动恢复 | 窗口、系统托盘、启动器等 | 服务实现可在部署/启动时配置；公共网页插件不直接获得 Tauri 或 OS 任意权限 |

建议第一条纵向切片选“完整 Agent 页面”，而不是先把每个按钮变成扩展点。它能最早验证核心 UI 可替换性，又不会制造过多细粒度接口。

### 15.3 一个 UI Host API，不复制业务后端

UI 插件发布 `UiContribution`，由 UI 消费者绑定到页面或 Shell 的 Role。它不是必须暴露给 LLM 的 Tool，也不必伪造 Agent Session 才能启动。后端、UI、Agent 是同一 Catalog 的不同消费者。

最小公共接口应覆盖：

- 应用导航、主题/语言、已授权的当前用户与工作区摘要；
- Session 创建、查询、发送、取消及带 cursor 的事件订阅；
- capability/Agent 配置读写的高层产品 API；
- 文件/剪贴板/窗口等特权操作的受限代理（按实际需求逐项加入）。

消息记录、活动 turn、权限及持久化真相仍在应用服务，UI 只保存界面状态。更换 UI 时重新查询状态和续订事件，不重新发起同一 turn。Session API 必须让插件 UI 能达到内置 UI 的功能完整性，不能只开放“发一句话，收一个字符串”。

跨边界使用结构化、版本化 DTO 和事件，不传 React component、闭包或 Zustand store。第三方可自由使用 React/Vue/Svelte 或原生 DOM；SDK 负责 transport 适配与类型生成，不规定业务框架。

### 15.4 渲染与稳定性选择

默认第三方 UI 建议沿用隔离 Surface，页面和完整 Shell 可以覆盖主内容区域。内置 UI 可以保留同进程渲染，但必须消费同一应用接口。不要把第三方 bundle 动态导入宿主 React 树当作沙箱：它会获得宿主页面环境，并耦合 React/CSS/内部状态版本。

iframe 的真实代价包括焦点/输入法、拖放、剪贴板、弹层、可访问性和主题同步；这些需要在桌面 WebView 与浏览器中验证。按页面/面板形成隔离边界，避免每个消息 token 或每个按钮创建 iframe。流式输出可批量更新，避免消息风暴。

保留一个插件之外的最小恢复入口：禁用故障 Shell、切回内置 UI、查看错误。它是恢复通道，不是偷偷切换正在执行的 Agent 实现。认证结果与授权真相不由 UI 自报；内容沙箱、Bridge 授权、服务端鉴权各自负责不同边界，不能只依靠 iframe。

### 15.5 实施成本判断

主题/区域贡献为中等改造，完整 Agent 页面为中到高，完整 Shell 为高；没有证据支持精确人日承诺。主要成本是提取公共 API、移除组件对全局业务 store 的直接依赖、补全事件恢复，以及跨平台交互验证。推荐“Agent 页面 → 页面/导航绑定 → Shell”顺序，持续保留内置 UI 的同契约实现。

## 16. 问题三：逐环节开放与组件化判断

### 16.1 判断原则

§7 的七个注入点只是当前已实现链路的分类，不是 Agent 架构的完整上限。本节把它们全部纳入，并扩展到模型、规划、压缩、记忆、执行、UI 和宿主服务。

开放应主要按**契约、所需资源、生命周期、运行隔离方式**判断，不按“Rust 能做、JS 不能做”判断。内置来源可以影响默认信任及执行位置，不应永久决定是否有资格参与某个环节。

表中“低/中/高”是相对改造成本，不是工期；现状来自静态代码复核，目标契约尚未实现。生命周期只有三类常用边界：turn（一次交互）、Session（一次会话）、安装/启动（共享主机）。不要为每个插件另造一套复杂状态机。

下表“当前普通用户插件状态”保留原始评估基线，不随每个在途补丁更新。2026-09-14 的初始 Context、协作取消、Role/Provider 映射及选择/默认管理切片统一见 §27.8；29 项逐一验收仍以 §27.2 为准。

### 16.2 全环节开放矩阵

本表的状态列保留原始评估基线，用于解释为什么要改造，不是当前进度表。此后 Context、通用 JS Role/Provider、不同资源需求及包内 Skill 只读消费已有实施切片；最新状态见 §27.1/§27.8 与实施台账，全部环节的交付要求仍按 §27.2 验收。

| 环节 | 原始基线的普通用户插件状态 | 建议开放方式 | 组装/替换时机与主要注意点 | 改造 |
|---|---|---|---|---|
| Catalog / 发现（§7.1） | Tool 等已进入目录 | 所有贡献进入统一 Catalog；插件可提供搜索/排序实现 | 安装后发现；搜索排名不直接授予能力 | 中 |
| Preset / 依赖（§7.2） | 能选择 capability，有依赖和锁 | 契约依赖、Provider 选择；允许插件发布不附带授权的模板 | 创建/保存时解析；模板只是配置种子 | 中 |
| Session 创建/资源绑定（§7.3） | 宿主固定资源解析 | 开放资源解析 Provider、初始化贡献和目标选择 UI | 创建时绑定；owner 与授权不能由插件伪造 | 中高 |
| 初始 Tool（§7.4） | N1 和无资源 M1 已通 | 延续现有 Tool 契约，统一执行适配 | Session 工具集；具体调用检查当前授权 | 中 |
| on-demand / ToolSearch（§7.5） | 动态 Tool 已通 | 搜索、排序、schema 暴露策略可替换 | 只从计划允许的工具中选择，遵守 turn 边界；模型可见性不等于新增执行授权 | 中 |
| Context / persona / system prompt（§7.6） | 普通插件准入未通 | 开放文本/结构化 Context；也允许选择完整 Prompt 管线 | Session 与每 turn；用户决定顺序和角色，标记来源与预算 | 中 |
| Skill（§8.2） | 元数据和锁有，精确运行消费未通 | 统一 Skill 装载器；包、文件目录只是来源适配器 | 初始索引/按需正文；正文与引用资源须对应锁定制品 | 中 |
| MCP（§8.3） | mapping 有锁，原生 Server 路径仍独立 | mapping、Server 连接由同一 capability/资源接口消费 | 启动或按需；避免同一工具双重注册、来源混淆 | 中 |
| ResourceProvider（§8.1） | JS proxy 有，产品准入未闭环 | 自定义 resource kind、列举/选择、acquire/release | Session 或操作级租约；保留真实资源 owner | 中高 |
| M1 Service 资源（§6） | 非空 resource_kinds 被拒绝 | 经统一资源代理传不透明 handle | 逐操作授权与撤销，不把宿主裸路径当权限 | 中 |
| Browser / Computer 等 Role | Rust 内置 Role 基础有，N1 禁止 | JS/Rust Provider 实现同一 Role，替换执行 owner | 通常 Session 固定；状态与平台权限不能自动转移 | 中高 |
| Turn Middleware（§7.7） | bundled 特定逻辑，非通用插件 hook | 显式 before/after 阶段，输入快照和结构化 patch | turn 边界；禁止共享任意可变 Engine 引用 | 中高 |
| EventSource / EventConsumer（§7.7） | bundled 生命周期路径 | 发布/订阅命名空间事件；声明所需主题 | 观察者异步，修改流程走明确命令；有界队列与取消 | 中 |
| Lifecycle / BackgroundService（§7.7） | bundled 路径 | start/stop/dispose 和健康状态，统一 supervisor 适配 | Session 或安装级；结束时释放资源，崩溃可定位 | 中高 |
| Transport / Channel | 宿主接线与内置能力 | 收消息、发消息、附件、身份映射 Provider | 安装级服务；通过 Session API，不直接操作会话库 | 中高 |
| Scheduler / Cron / Automation | 内置 owner，非通用用户扩展 | 触发器、调度策略、任务执行器可替换 | 安装级；沿用单一任务所有权与取消/重入语义 | 中高 |
| 模型 Provider / 模型路由 | 有 Rust provider 接口，无普通插件替换闭环 | 模型流式调用契约 + 可选路由策略 | Session 锁定策略和授权候选池；按请求路由可开放 | 高 |
| 上下文选择 / 历史 / 压缩 | Nomi Engine 内聚实现 | ContextPipeline、Compactor 等有意义的策略接口 | 每 turn/模型请求前；变换视图不篡改持久化事实 | 高 |
| 长期记忆 / RAG / embedding | 部分业务 capability，非完整管线替换 | 读写/检索/索引接口及资源 Provider | namespace 隔离；模型生成的记忆可追踪、可删除 | 中高 |
| Planner / 推理循环 / 停止策略 | Nomi 内部实现 | 明确策略组件；复杂算法可直接实现整个 Runtime | 一个 Session 一个 Runtime owner；预算与取消仍可执行 | 高 |
| 工具编排 / 并行 / 结果转换 | Nomi 执行链 | 调度策略、结果渲染/变换、重试建议接口 | 副作用工具不因传输超时默认重试 | 高 |
| 子 Agent / 多 Agent 协作 | 有内置委派能力，无通用插件装配 | 委派/协调 Provider 调用统一 Session 与能力接口 | 子任务继承或缩小授权；显式预算、取消传播 | 中高 |
| 整个 Agent Runtime / Loop | 生产 handle 为 Nomi 变体 | 会话命令/查询/事件/可选 checkpoint 契约 | 创建时选引擎；支持其他引擎，不强迫其使用 Nomi 内部策略 | 高 |
| 日志 / tracing / 评估 / 计量 | 宿主设施 | 观察者、导出器、评估器插件 | 默认异步、不阻塞 turn；敏感内容按授权提供 | 中 |
| UI / Agent 页面 / Shell | Surface 有，替换未通 | §15 的 UI Role 与应用 API | 视图可重载；独立于正在运行的 Session | 中高 |
| Session 存储 / 事件存储 / checkpoint 存储 | 宿主与引擎特定实现 | 按实际一致性需求提取存储 port，提供替代后端 | 安装/启动或维护窗口切换；数据迁移不能假装热替换 | 高 |
| Preset 编译策略 / Registry 后端 | 固定控制面与 Kernel | 开放选择策略、目录存储；最终计划校验器保持单一 | 编译时或启动配置；插件提出计划，统一校验后生效 | 中高 |
| 认证 / 授权策略 / Secret provider | 宿主权威 | 部署者可选认证、策略和密钥存储实现 | 安装/启动；普通插件不能自批权限或读取其他插件密钥 | 高 |
| supervisor / sandbox / Kernel 宿主实现 | 固定主机 | 执行后端与隔离 backend 接口可替换；整宿主支持重新部署 | 停机/受控重启；不能让被监管进程自己撤销监管 | 高 |

### 16.3 最小宿主不等于把所有重要功能永久封闭

推荐默认 Rust 宿主只保留：启动与恢复、契约与绑定的最终校验、调用授权、资源租约账本、执行生命周期监督，以及真实状态提交的仲裁。UI、Prompt、模型、循环、检索、调度等产品逻辑逐步退出这层。

其中存储、认证、Secret、隔离后端仍可通过部署级 Provider 替换；这些实现属于用户选择的信任根。不可同时保证“插件随时改写自己的授权/监督者”与“该监督者能约束插件”。这不是 Rust 或 JS 能解决的矛盾。

因此建议把目标分成：**产品部件通过公开插件契约替换；基础设施通过部署级接口替换；最小宿主仲裁实现通过独立宿主实现/发行版替换。** 后两者不能冒充“普通用户安装一个 JS 插件即可替换”，整宿主重编译也不是插件化完成证据。不承诺任意时刻无状态迁移地热换一切。如果原始目标严格要求普通插件接管包括信任根在内的所有部件，本方案并未完整满足这个字面目标；具体边界与原因见 §27.4。

### 16.4 Middleware 不应演变成任意 hook 拼装的流程迷宫

优先开放少量真实阶段：输入准备、上下文构建、模型调用前后、工具调用前后、turn 结束。观察事件和修改请求分开：观察者看快照；修改者返回明确 patch。对同一字段的多个修改按用户可见的管线顺序执行，不依赖并发竞态。

取消/超时/资源释放由公共执行边界统一处理；不为每种 middleware 新建 supervisor。组件不拿 `Engine` 可变引用，宿主也不应持有引擎大锁等待跨进程插件，以免回调重入造成死锁。

同时避免另一种极端：为了支持一个完全不同的 Agent Loop，要求它复刻几十个 Nomi hooks。替代引擎只需满足外层 Runtime 会话契约；内部阶段是否支持、支持哪些，应显式声明，由组装器提前提示不兼容。

### 16.5 自定义资源和授权的开放方式

当前 `NomiCoreResourceBindingResolverRegistry::product` 按固定 `SUPPORTED_RESOURCE_KINDS` 组装，且目前每种 kind 仅一个选择，见 [`nomi_core_resource_bindings.rs:120`](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs#L120)。这不是应该永久维持的产品限制。

建议资源 Provider 自带 schema、列举/选择能力、所需操作和生命周期；用户可以拥有插件自己产生的资源域，也可以明确把外部资源交给它管理。宿主维护跨插件 grant 和不透明租约，插件维护其资源内部实现。多工作区等需求通过有名称的 binding/multiplicity 表达，不靠不断新增固定 kind。

授权应尽量在安装/绑定时一次完成，只有新增权限或敏感不可逆操作才需要额外确认；不能把“每个 hook 都弹审批”当作稳定性设计。授权只是限制实际数据与副作用访问，不限制开发者提供哪种算法或 UI。

## 17. Rust 主机与 JS 插件：优雅边界及 Rust 插件收益

本节保留包含 Rust 插件时的技术选型分析，不表示 Rust 是开放其他环节的前提。按本轮“先不考虑增加 Rust 插件”的情景，先推进 JS 与内置 Rust 的共同消费边界，不实施 native adapter/SDK；具体调整见 §27.3。

### 17.1 首要问题是接口边界，不是语言对立

不能跨语言直接传递 Rust 的 trait object、借用、`Arc`、异步 Future 或内部服务引用，但可以传递请求、结构化结果、流、取消通知和资源 handle。大部分 Agent 扩展是粗粒度 I/O 与策略调用，适合这种边界；不需要把每次 Rust 函数调用都 RPC 化。

现有 [`JsKernelPluginAdapter`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs) 已把 JS 调用适配成 Kernel handler/context/resource 接口，证明方向可行。真正缺口是宿主准入和实际消费，而不是先换语言就能解决的问题。

建议公共契约以语言无关 DTO 定义；继续利用现有 Rust canonical contracts 及 schema/TS 生成设施，Rust trait 只是主机内部 port。JS 和 Rust SDK 都实现同一套线协议，不各自维护能力目录、Preset 格式或授权系统。

### 17.2 执行形态比较

| 方案 | 优点与适用场景 | 真实代价/限制 | 建议 |
|---|---|---|---|
| JS/TS + Node 进程 | npm 生态、网络/业务集成、UI 配套；已有主机 | npm 供应链、进程资源、共享进程故障；普通 Node 不是安全沙箱 | 保留主力，扩展契约与隔离能力 |
| Rust 独立进程 + 同一协议 | 原生 SDK、设备/浏览器控制、计算、完整 Agent Runtime；没有 Rust ABI 绑定 | 各 OS/架构制品、签名/升级、进程成本；跨界仍有序列化 | 确有原生库/性能需求时优先选此形态；不是 JS 开放路线前置 |
| Rust 编译为 Wasm component | 受限算法、过滤器、解析/评分/压缩策略；类型化接口与内存隔离 | host imports、WASI 支持、异步/流及线程要按所选工具链验证；不能直接用任意 OS crate | 有需求与实测后加入，非全平台改造前置 |
| 内置 Rust 静态模块 | 零 IPC、可复用现有 Rust 实现；默认高性能路径 | 需重编译宿主，进程内故障影响主机；不是用户安装型插件 | 作为官方实现方式，保持相同逻辑契约 |
| Rust 原生动态库 | 进程内调用，部分原生场景低延迟 | Rust ABI 无稳定保证；内存、allocator、panic、线程及卸载风险 | 不作为公共插件方案 |
| 嵌入 JS 引擎 | 部分纯 JS 可减少进程成本 | npm/Node API 兼容与原生模块问题；另建宿主维护面 | 当前无必要，不再增加一条主路径 |

这里的“Rust 插件支持”应是用户可安装的 Rust 制品 + SDK + 生命周期管理，不只是仓库里多写一个内置 crate。Rust 插件不会自动解决 UI 契约、Skill 内容装载、Provider 选择、M1 双分支或冻结会话问题。

### 17.3 若增加 Rust：JS 与 Rust 进程共用执行协议

长期包模型可容纳 UI 资源、JS 服务、特定平台的 Rust 服务，以及之后可选的 Wasm 制品；都是同一插件身份和 capability 贡献，执行器按发布声明选择。无需同时支持所有形态；若选择 Rust 路线，先增加 Rust process adapter，JS-only 路线则不实施该 adapter。多后端同包编排不是首期范围。

公共调用边界需要的基本信息是：请求标识、操作名、输入、截止时间/取消、结构化错误、事件流、资源 handle。身份与已授权上下文由宿主注入，不让插件自报 owner。现有 N1 RPC/进程管理可复用，但 N1 消息合同是特定 JS host 设计，不能仅把 `node` 换成一个可执行文件就宣称 Rust 插件接通。

建议先提取最小通用协议，保留各执行后端薄适配：

- JS adapter 在 Node 中装载模块；Rust adapter 启动对应目标平台可执行文件；未来 Wasm adapter 调用组件导出。
- 同一份接口数据定义生成 SDK；不同 wire encoding 可以适配，但不能手工维护三份不一致的业务语义。
- 流式模型响应采用有界事件流，支持消费者取消；大文件/图像传制品引用或资源流，不把所有数据塞进 JSON。
- 高频纯计算尽量成批调用；不要为每 token、每字节、每个内部函数跨进程往返。先测实际瓶颈，再决定是否需要 Wasm 或静态内置实现。

不要新建全局消息总线、Service Mesh 或自制 Rust FFI 框架来解决当前问题。已有 process supervisor 可共享进程生命周期 primitive，但 Nomi Runtime、JS module、UI Surface 的状态不必被强压成同一种内部状态机。

### 17.4 必须明确的隔离事实

当前 [`extension-host.mjs:226`](../../crates/backend/nomifun-js-host/assets/extension-host.mjs#L226) 使用普通 `import()` 加载模块；[`supervisor.rs:1217`](../../crates/backend/nomifun-js-host/src/supervisor.rs#L1217) 启动 Node 并配置 stdio。这里不能据此声称“第三方 JS 已受强沙箱隔离”。共享 host 的异常/无限循环也可能影响同 host 其他插件。

**进程隔离主要隔离崩溃，不自动隔离文件、网络、凭证或同权限进程。** Rust 独立进程同样如此。通过 SDK 校验 grant 不能阻止拥有 OS 权限的插件绕过 SDK 直接访问资源。

建议先明确产品执行档位，不做名义上的沙箱：

- 可信本机插件：安装者明确接受本机权限；仍隔离故障、限制资源和清理进程。
- 不可信插件：只在真实受限的 OS sandbox 或受控 Wasm imports 中执行；未实现的平台不能宣传强隔离。完整 npm/原生 SDK 兼容与严格隔离存在代价，需要按需求取舍。

可按插件/信任域分进程，避免所有用户插件共享一个易阻塞主机；这比仅增加 Rust 支持更直接改善可靠性。具体分组策略通过并发和内存测试确定，不预先创建复杂调度平台。

Wasm 隔离也不是万能：宿主授予的文件/网络 imports 仍代表真实权限；无限循环要有 fuel/epoch 等中断与资源限制，阻塞 host call 要单独有截止时间。它不能自动把任意 Rust crate 变成可沙箱运行的组件。

### 17.5 原生动态库为什么不推荐

Rust Reference 明确说明 Rust ABI 不提供稳定性保证。使用 `extern C` 能制定自己的稳定 C ABI，但仍要处理内存所有权、allocator、错误、并发、卸载和版本，且动态库与宿主共享地址空间。这正是本项目希望避免的长期复杂度。

因此：**若增加 Rust 插件，优先以进程协议接入；不建议增加任意 Rust dylib 装载。** 是否投入 Rust/Wasm 后端，应由代表性原生库、性能或安全需求验证收益；当前可先不投入，不妨碍 JS 插件开放其他部件。

本节外部技术依据（2026-09-13 查阅，非本仓库运行验证）：

- [Rust Reference：外部块 ABI](https://doc.rust-lang.org/reference/items/external-blocks.html#abi)：Rust ABI 无稳定保证，C ABI 按目标平台规定。
- [Node.js：VM](https://nodejs.org/api/vm.html)：明确指出 `node:vm` 不是安全机制，不能拿它作为运行不可信代码的沙箱替代。
- [Component Model：为什么需要组件模型](https://component-model.bytecodealliance.org/design/why-component-model.html)：WIT 与跨语言、独立编译组件的类型化调用边界。
- [Wasmtime：Security](https://docs.wasmtime.dev/security.html)：Wasm 隔离及 WASI capability-based 文件访问的边界。

## 18. 问题四：Skill 装载未闭环究竟影响什么

本节保留原始断点分析。2026-09-14 后续实施已验证包内精确来源锁、只读正文/资源及 Session/Bootstrap/标准 Skill 工具消费，并接通 `/skill:<精确ID>` 用户命令和输入框补全，证据见实施台账 §2.8～2.9 和本报告 §27.8；不再将这些子链路视为全未接通。冷启动产品用例已修复通过（台账 §2.16），附图/装饰输入消费见 §2.17；脚本/fork/hook、其余来源统一、完整权限提示与历史制品保留/迁移仍未完成。

### 18.1 原始断点与当前接通范围

原版“完整装载链未闭合”的方向成立，但必须补上“ID 已经传递”的事实。下面是原始基线链路，不再代表当前包内 Skill 的生产装载路径：

```text
N1 Skill 声明 / 制品
  → materialize SkillDefinition
  → 编译 SkillRef、body digest、required capabilities 与来源锁
  → project_internal 只投影 Skill ID
  → included_skills → agent_enabled_skills → extra.skills
  → 传统目录/名称解析 → Nomi 扫描并创建 SkillTool
                         ↑
          缺少按插件来源锁解析正文/引用资源、校验并装载的统一桥接
```

原始断点的代码定位（行号对应研究时版本，不作为最新代码位置保证）：

- [`compiler.rs:1411`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L1411) 生成 `ResolvedSkillLock`；[`compiler.rs:879`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs#L879) 校验 Skill provenance。
- [`nomi_core_agent_projection.rs:170`](../../crates/backend/nomifun-app/src/router/nomi_core_agent_projection.rs#L170) 只提取 `lock.skill.id`；没有把锁定内容交给 runtime。
- [`service.rs:4316`](../../crates/backend/nomifun-conversation/src/service.rs#L4316) 将 included skills 放入临时输入；[`service.rs:4384`](../../crates/backend/nomifun-conversation/src/service.rs#L4384) 计算名称快照。
- [`skill_resolver.rs:41`](../../crates/backend/nomifun-conversation/src/skill_resolver.rs#L41) 使用 SkillPaths；[`bootstrap.rs:694`](../../crates/agent/nomi-agent/src/bootstrap.rs#L694) 使用 `load_all_skills` 扫描。
- [`skill_tool.rs:20`](../../crates/agent/nomi-agent/src/skill_tool.rs#L20) 的 Skill 还有变量替换、shell、fork 等执行行为，不能把它简化为一段总是安全的 Markdown。

原始 `ResolvedSkillLock` 本身仅包含 SkillRef、body digest 和所需 capability，完整来源还依赖其他锁/制品元数据。当前已显式加入 contribution、Mount、source 和 artifact digest，并由同一 Compiler 生成与校验，不再沿用“单个 Skill lock 缺来源”的现状判断。

当前包内只读链路已是：

```text
保存时生成精确 Skill 锁 → 已鉴权 Session → 按锁读取并校验声明文件
  → NomiPluginToolSession.with_package_skills → Bootstrap 索引/标准 Skill 工具
  → 每次正文/资源读取检查来源与依赖资格
```

该链路在现有 [产品 Session](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs)、[包内装载器](../../crates/backend/nomifun-ai-agent/src/plugin_skills.rs) 和 [只读 Skill 消费者](../../crates/agent/nomi-agent/src/host_skills.rs) 中可定位。最后一步仍是只读消费，不是通用脚本执行器；含未支持模式的包内 Skill 会明确报错。台账 §2.8 记录既有定向测试，本轮仅静态复核这些接缝。

### 18.2 影响范围

下表说明**原始断点未修复时**的影响。当前包内只读链路已消除“只传 ID 再搜索同名目录”的问题，但不能据此消除其余来源、执行模式与版本保留问题。

| 情况 | 实际影响 |
|---|---|
| 发布了只有 N1 包里存在的 Skill | package/catalog/compile 成功不足以保证 runtime 找到正文；可能不可用或没有进入预期行为 |
| Tool + 配套 Skill 的插件 | Tool 可以独立调用；但模型可能没有得到插件作者提供的使用规则、工作流和限制，效果下降 |
| 本地恰有同名 Skill | 名称能解析不等于加载了所选插件版本；存在来源混淆风险，不能把偶然命中当作闭环 |
| 插件升级/删除与历史 Session | 目录文件与锁定制品若分离，无法保证老 Session 使用原版本内容；也可能找不到文件 |
| 依赖检查 | 编译时检查所需 capability，不等于 Skill 真正执行时资源已绑定、Tool 已进入模型可见集合或脚本已授权；不要把这些状态合并成一个“激活”开关 |
| 传统内置/工作区 Skill | 原有目录路径仍可工作；不能由此推断“所有 Skill 都坏了” |
| 不使用 Skill 的 Tool/UI 插件 | 不直接受此断点影响 |

这是插件平台正确性与体验问题，**不是单凭静态代码就能断言所有场景都会失败的运行故障**。原始研究未执行真实 Skill 安装 smoke；此后已有真实包导入/安装与生产接缝的定向测试，见台账 §2.8，但没有外部模型的全产品验收。上述历史风险不能全部照搬成当前失败结论。

对现在安排开发和使用的影响，应另看下面这张表，不能只阅读历史断点：

| 当前使用场景 | 当前结论与处理 |
|---|---|
| 包内只读说明、参数替换和声明的引用资源 | 已有精确装载、标准工具/显式命令及冷启动发现的切片证据；保留回归，不重做装载器。装载成功不保证模型一定遵循说明 |
| Skill 声明 shell、fork、hooks 或模型/工具覆盖 | 现有 `HostSkill::read_only` 明确拒绝这些模式，不能承诺这种 Skill 安装后即可执行；需要继续接生产执行、权限和取消链，而不是放开 validator 就算完成 |
| 长期保留会话、插件升级/撤下、其他 Skill 来源 | 仍需补制品保留、兼容/迁移及来源统一验收；不能以当前正文可读推断历史会话始终可恢复 |
| 仅使用独立 Tool，或开发替代 UI | 不应被 Skill 剩余模式整体阻塞；只有该功能明确依赖 Skill 时才产生依赖。UI/模型等开放工作可按各自公共接口推进 |

这意味着问题 4 的答案是：**会影响依赖 Skill 的插件是否真正可用和行为是否符合作者预期，但不是整个插件系统都不能用；目前基础只读装载已取得进展，完整执行能力仍未解决。**

排期上，它应是“包内 Skill 可用”承诺的发布前置，不是等全栈改造结束再处理的优化。当前 `ExtensionSkillResolver::resolve_skills` 出错时记录 warning 并返回空列表，说明上层也不能用“会话启动成功”证明所选 Skill 已装载。精确包来源应返回可定位到所选 Skill 的错误，不允许静默改用同名目录；传统目录来源的容错策略则单独保留和验证，避免为修复插件路径破坏原用法。

### 18.3 建议的最小闭环

1. 提供一个内部 `SkillSource`/装载 port：按精确 Skill 引用、来源锁和制品返回正文及资源清单；N1 制品、内置目录、工作区目录都是来源 adapter，而不是三套 runtime。
2. Session 启动时解析选中的 Skill，校验正文 digest、来源、引用资源与所需能力；生成完整运行时 Skill descriptor。不要退回“复制进某个同名全局文件夹后扫描”的隐式方案。
3. 使用现有 Nomi Skill loader/执行器的解析和执行能力，但让它能接收已解析描述，而不是只靠磁盘扫描。目录兼容能力保留，不代表保留两份技能事实。
4. 将索引注入、按需正文读取、附件/引用文件读取、脚本/fork 执行分别接通。Skill 不自动授予额外 Tool 权限；引用脚本必须走同一资源/执行授权边界。
5. Skill 使用前校验锁定依赖、当前执行权限与资源有效性；需要模型调用 deferred Tool 时，复用已有 ToolSearch/schema 暴露路径，不新增第二套 capability 激活状态机。当前 [`SessionCapabilityState::new`](../../crates/backend/nomifun-agent-kernel/src/session_capabilities.rs) 已将 Snapshot 的整个 capability allowlist 作为 active 集合；模型尚未看见工具 schema 不等于 Kernel 尚未授权该能力。内部依赖也不应仅因被 Skill 引用就自动对模型公开。不满足依赖时给出明确错误；正文进 prompt 不等于 LLM 一定遵循，验收验证装载内容及调用链。
6. 锁定的包制品按存活 Session/Revision 引用保留，卸载撤销执行资格与回收磁盘制品分开处理。可变工作区 Skill 若要纳入精确可复现会话，必须快照为制品；否则明确标为动态内容，不能宣称已有强版本锁。

验收至少包含：仅在插件包存在的 Skill 真正进入 runtime；同名不同来源不串用；加载内容 digest 一致；引用文件可读；缺失/越权依赖被识别；升级后旧会话不漂移；传统 Skill 仍可用。自动化使用可捕获真实装载文本的 runtime fixture，再补一个真实 Agent 调用 smoke。

### 18.4 剩余 Skill 工作必须分开排期，不能用独立执行器替代生产链路

**装载、显式命令、脚本执行和子 Agent 执行是不同交付项。** 前两项的包内只读切片已有证据，不应重复建设；后两项涉及真实副作用及其执行 owner，不能仅删除 `HostSkill::read_only` 中的拒绝检查。

当前代码提供了明确的排期依据：

- [`execute_fork_with_shell`](../../crates/agent/nomi-skills/src/executor.rs) 先准备正文（其中可以执行 shell），再调用 `AgentInvocationRunner`。因此 fork 的授权必须在任何脚本副作用之前成立，不能等子 Agent 启动时才检查。
- [`should_install_embedded_agent_execution`](../../crates/backend/nomifun-ai-agent/src/factory/nomi.rs) 仅为无 Platform Gateway 的可信 owner 会话安装嵌入式执行器；有 Gateway 的生产会话由 Gateway 持有执行权。不能为了支持 Skill 就绕过此规则再启动一套本地子 Agent。
- [`LocalAgentInvocationRunner`](../../crates/agent/nomi-agent/src/local_agent_invocation.rs) 的子工具目录目前是 Read/Write/Edit/Bash/Grep/Glob，不是父会话全部插件能力的通用组装器；[`SubagentHost`](../../crates/agent/nomi-agent/src/subagent_tools.rs) 提供已委派子任务的查询、发送、等待和取消，也不是直接创建任务的接口。已有这两个接缝，不代表任意包内 Skill 的 fork 已可在产品执行。
- 当前包内命令只把受校验正文追加到同一用户 turn。若支持有副作用的命令，必须进入受管 operation 和效果记录，不能在命令预处理时无归属地执行脚本或创建子任务。
- **冷启动发现已有产品切片证据。** [`useSlashCommands`](../../ui/src/renderer/hooks/chat/useSlashCommands.ts) 已移除非空 agent status 前置；Conversation 先检查会话归属，Registry 有 live runtime 时采用其命令列表，无 runtime 时查询现有 Session Provider。Provider 复用保存绑定的编译校验和 [`load_package_skills`](../../crates/backend/nomifun-ai-agent/src/plugin_skills.rs)，不调用运行时 `resolve()`。实际产品用例验证发现不激活 JS/Context、不创建 runtime、不改 Snapshot，错误 owner 和撤下均被拒绝；测试夹具枚举/必填字段错误已修复，详见台账 §2.16。
- **冷启动列表是包内命令建议，不是最终可执行清单。** 当前只按精确包来源和 `user_invocable` 等条件生成描述，不重建整套 Nomi 配置/deny 策略；只读发现 descriptor 明确不能授权执行。真正使用时仍复查来源、活动能力与运行时权限。它不等于目录/MCP 等所有来源的完整命令列表，也不能保证建议项在后续运行时一定可执行；完整权限拒绝提示仍需产品体验验收。发现还会读取精确正文/资源并复用编译校验，不能宣称已消除重复编译或冷启动成本。
- **附图和装饰输入不再直接退化为普通模型文字。** 原引擎单文本限制已对显式包内 Skill 放开，图片与附加文本保留，参数只来自用户命令。生产 manager 将本次原文传入原 turn 接缝，知识前缀不负责选择命令，自动续跑明确禁用命令重放。原 deny、用户可调用性、工具上限和来源检查仍在模型调用前执行；Bootstrap 捕获模型请求与否定场景见台账 §2.17，不将其扩大为所有附件适配器或真实 WebView 上传均已验收。

| 交付切片 / 原工作包 | 真正前置条件与可并行范围 | 退出条件 |
|---|---|---|
| 只读命令体验补齐（P3） | 冷启动与附图/装饰输入按台账 §2.16～2.17 核销已验证子项；保留回归，继续完整权限提示与其余来源；无需等待子 Agent 或新增 Runtime | 无 live runtime 时可显示所选包内命令建议；状态变化后采用实际命令列表，权限拒绝有明确提示，带附件/装饰输入语义明确；发现不启动 turn，也不授予执行权 |
| 包内受管 shell（P7） | 精确制品资源、进程/工作目录授权、turn operation 与取消/清理；不能以文件可读代替可执行授权 | 用户只读/拒绝授权时不会启动进程；取消不遗留进程，副作用有记录，命令与 Tool 不各造执行器 |
| 包内 fork（P7，与 P8 子任务接口协同） | 先复用或演进现有生产委派入口，明确父子权限、所需插件能力、预算、取消及效果归属；不必等待整个多 Agent 工作包结束 | Gateway 场景走原执行 owner，受支持的 standalone 场景另有证据；不放大权限、不静默换成本地执行，子任务结束/失败/取消可追踪 |
| hook / model / tool override 等模式（P7/P8） | 分别依赖具名生命周期、模型候选与作用域合同；声明支持后必须有实际消费点 | 缩窄权限可执行，扩大权限不被默许；不支持时在最早可判断的位置明确拒绝，不忽略字段伪装成功 |
| 其余来源与历史保留（P3/P7） | 统一来源适配、制品引用保留和旧 Snapshot 切换验证；可与执行模式开发并行 | 旧版本不漂移、撤销仍有效、引用资源可读；旧数据不伪造来源或自动清除 |

这不是新增一套 Skill 平台，也不是加 Rust 插件的理由。fork 与“子 Agent / 多 Agent 协作”共用一段执行基础，应在 P7/P8 中只计费一次；shell、UI 和其他独立切片不因此等待全部协作能力完成。若首发只交付只读 Skill，发布说明必须明确该支持范围，完整需求 4 仍保持未完成。

## 19. 问题五：更简单、更开放的 Agent 组装模型

### 19.1 现有架构的问题不是“有锁”，而是相同事实被多次翻译

应保留的基础是：插件身份清晰、显式选择、不可变会话计划、资源授权和可解释失败。它们有真实稳定性价值，不应为了“链路短”全部删除。

需要收敛的结构：

1. **原始基线的 N1 与 M1 两套解析结果及执行入口**：`ResolvedCapability` 与 `ResolvedMiniAppCapability`、独立 materialization/invoker，让每个消费者都必须认识来源类别，见 §13。`439bb385a` 已合入统一工程，此项按当前消费者核销剩余工作，不安排重新统一一次；它也不自动解决下列 Provider、Skill 或 Runtime 缺口。
2. **静态 native projection、用户 Tool、bundled Context/lifecycle 分开准入**：能力“能否执行”依赖多处白名单/分支，而不是贡献声明与执行器支持矩阵。
3. **保存/打开会话时重复解析**：[`compile_nomi_plugin_snapshot`](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs) 目前对当前 Registry 重编译，再与已保存 envelope 比较。这提供严格漂移检测，但也让历史会话启动依赖当前目录形态。后续已修复为从保存的 Provider locks 派生冻结选择，不跟随最新安装默认；这解决了默认漂移，并没有退出重复编译。退出它必须以完整 descriptor/锁定需求和等价校验为前置，不能直接删除这道检查。
4. **Skill 来源与消费统一尚未完成**：原始包内引用退化为字符串的断点已由台账 §2.8 的精确只读链路修复；剩余模式和来源继续复用该接缝，不再重建装载器，见 §18。
5. **引擎实际仍封闭**：[`AgentRuntimeHandle`](../../crates/backend/nomifun-ai-agent/src/runtime_handle.rs#L180) 的生产变体是 `Nomi`，Mock trait-object 是测试支持，不是用户 Runtime 扩展。
6. **已有接口与实际实现未分开组装**：[`LlmProvider`](../../crates/agent/nomi-providers/src/lib.rs#L143) 已是 Rust trait，但创建仍走 [`create_provider`](../../crates/agent/nomi-providers/src/lib.rs#L801)；Nomi 的压缩、hooks、循环逻辑仍聚合于 [`engine/mod.rs`](../../crates/agent/nomi-agent/src/engine/mod.rs)。Rust interface 存在不等于产品可安装、可选择、可替换。

### 19.2 推荐目标：一套对象，三段处理，不另造平台

只保留用户需要理解的三类对象：

- **插件**：可安装/发布的制品，可提供多个 capability；执行形态属于制品声明。
- **能力与契约**：capability 描述可消费贡献，Role 描述可替换部件，Provider 表达实现关系；都在同一目录中。
- **Agent 配置**：选择能力、Provider 和必要参数；保存为 Revision/Snapshot，具体资源在 Session 绑定。

内部主要处理链：

```text
插件发布 → 单一 Capability Catalog
                    ↓
用户 Agent 配置 → resolve/compile → 冻结 Snapshot（执行计划）
                                         ↓
                     Session bind：资源、当前授权、执行器就绪
                                         ↓
                    选定 Runtime + 公共能力执行接口
                      ├─ 默认 Nomi 与其策略组件
                      ├─ 内置 Rust adapter（进程内）
                      └─ JS 插件执行 adapter

UI Provider → 同一应用 command/query/event API → 上述 Session
```

这里的“执行计划”就是演进后的 `ResolvedSnapshotContent`，不是新增一个与 Snapshot 双写的 `AgentPlan` 存储。SDK、RPC、Rust trait 是同一逻辑调用的不同适配面，也不是需要依次穿过的三个业务 coordinator。

上图按当前 JS-only 范围表达：保留 Rust 宿主和内置 adapter，不建设用户 Rust/native 或 Wasm 插件执行器。未来是否增加其他后端另行决策，不能成为上述组装链的前置工作。

三段的职责应明确：

1. **发布**：验证包/贡献契约、生成 Catalog 条目与执行描述。N1/M1 差异在这里和执行 adapter 内吸收。
2. **编译**：解析依赖、Provider、顺序、schema 和制品版本，检查所选 Runtime 支持的接口，生成一份冻结计划。
3. **绑定/执行**：不重新选择实现，只解析具体资源、检查当前撤销/可用性、准备执行实例，并调度计划。实际有副作用的调用继续执行操作级授权。

发布/会话/调用的校验针对不同事实，不能合并成一次后永远信任；但同一份契约与绑定不应在每层重复推导。

**统一组装不等于统一成一个万能调用函数。** Tool/策略适合请求与结果，模型/Runtime 需要有界事件流，资源需要租约，UI 需要应用 API 与视图生命周期；它们共享身份、绑定和错误/取消语义，不必伪装成同一种 Tool。否则会把流、状态和权限塞进无类型参数，再由每个消费者二次解释，表面层数少，实际更难维护。拆分单位应是有第二实现价值的业务部件，不是一行代码一个插件点。

### 19.3 用户只做一次选择，系统完成编译

产品界面应是“选一个默认部件实现、添加若干能力、绑定所需资源”，不是让用户编辑 Role digest、artifact digest、mount ID、Snapshot ID。依赖和版本解析由后端完成，界面展示来源、能力、缺失配置和影响摘要。

示意配置（仅解释目标语义，不是当前 API；实现时沿用 canonical Preset/Role 字段，不再保存此平行格式）：

```yaml
agent: 我的研究助手
runtime: acme.research-runtime
providers:
  model: platform.model-gateway
  search.web: acme.search
  context.pipeline: acme.research-context
tools:
  - platform.files
skills:
  - acme.research-method
ui: acme.research-workbench
```

Runtime 若不支持 `context.pipeline` 替换，应在配置阶段说明，而不是运行时静默忽略。新用户可以只选官方模板，系统自动补齐默认；高级用户再替换任何公开部件。模板展开后成为普通配置，没有永远覆盖用户选择的隐藏模板逻辑。

这里的 `runtime` 是目标中的 Agent 算法实现选择，不是 Node 下载、进程启动命令或 Host 管理参数。现行 05 规范 §15.3.4 明确禁止 Revision 携带 `runtime selector`，上例不能直接转成新增 payload 字段。P8/P9 应先按 §27.9 明确并版本化演进契约；在此之前，生产合同仍以现行规范为准。

Preview 和 Save 调用同一个编译服务。可以提供“保存并使用”一次操作；若 Preview 后 Catalog/配置已变化，返回简明差异或要求确认，不让用户手工再走一串 Compile/Lock/Bind 页面。Snapshot 是稳定性实现，不应成为使用负担。

### 19.4 默认 Nomi 组件化与完整 Runtime 替换并行成立

不要一次性把 Engine 每个函数切成微插件。先提取具备独立替换价值的边界：模型调用、Context 管线、工具执行、历史/压缩、委派、输出事件；每个边界有真实第二实现时再验证抽象是否够用。

外层 Runtime 最小协议建议覆盖：

- 创建/打开会话并绑定冻结计划；
- 开始 turn、取消 turn、关闭会话；
- 查询当前状态、订阅有顺序的事件流并识别终态；
- 显式报告可选功能：steer、resume、fork、checkpoint，以及支持哪些内部可替换策略。

不要求所有引擎支持每一种可选功能；消费者根据支持声明呈现功能或在组装时报告不兼容。也不要求替代引擎使用 Nomi 的 prompt、压缩格式或内部 checkpoint。

宿主负责会话身份和接受执行结果，Runtime 负责一次会话的执行状态；同一 Session 不允许两个引擎同时充当执行 owner。不同 Runtime 的 checkpoint 不假定互通：不支持迁移时可显式新建会话/导入可读历史，而不是伪装“继续同一个内存状态”。

Runtime 通过公共 Tool/模型/资源服务使用已绑定能力。若用户选择完全自主访问网络/系统的可信 Runtime，应明确这是更宽的信任档位，不能一边允许绕过代理，一边声称所有操作都由 Kernel 细粒度管控。

复用当前 `NomiCoreSessionOwner` 等已统一消费边界；将 `AgentRuntimeRegistry` 的构造与 handle 演进为生产可注册的 Runtime factory/port。Fresh-v4 的契约可以作为参考或迁移输入，但不再维持两个同时竞争的生产 Session authority。

### 19.5 把锁定保留下来，把重复解析移出去

建议 `Save` 产生可直接执行的冻结计划；`Open` 只验证计划完整性、被引用制品仍保留、授权未撤销和所需执行器可用，不再从“当前 latest Catalog”重新求解同一依赖图。

这需要先实现精确版本制品寻址/保留和足够完整的执行描述，**不能现在直接删除 Session 重编译校验**。现有重编译包含正确性保护；只有新的等价校验与回归覆盖到位，才可替代它。

可缓存编译结果，缓存键覆盖配置、实际选中制品/契约及编译器语义版本，不让不相关插件发布无谓地使全部 Agent 失效。执行时撤销权限、禁用插件、资源丢失等仍必须生效；不可变计划不是永久授权。

### 19.6 内置能力必须真正走同一接口

内置与第三方可以使用不同执行后端，但不能使用不同产品能力等级：内置实现也发布 capability/Role，受同一绑定解析、生命周期与错误语义约束。官方静态 Rust adapter 可以零 IPC，不要求为了形式统一把所有内置模块移到子进程。

迁移验收必须检查原先的直接调用是否已退出，而不仅检查新接口存在。每开放一个部件，删除对应的内置 ID 分支、来源白名单或旧组装入口；“新接口 + 永久旧路径 + fallback”不是完成。

### 19.7 简化失败语义，不追求不存在的无条件稳定

- **必需组件失败**：停止相关操作并解释来源；不能把缺少模型/Context 管线的执行报告为成功。
- **明确可选的观察/展示贡献失败**：允许跳过，但留下诊断；是否可选由契约/用户配置确定，而不是统一吞错。
- **Provider 替换**：默认在新 Session 生效；有状态组件必须有显式迁移，否则重新建立实例。
- **副作用结果未知**：保留未知状态，不自动重试；普通纯函数失败不需要全套 durable receipt/outbox。
- **UI 故障**：可恢复内置视图，不取消或重放后台正在执行的 turn。

平台能保证接口一致、故障可见、资源可回收和不暗中切换；不能保证任意第三方算法都正确，或任意 native 插件都不会破坏其已获授权的资源。

## 20. 实施路线：每步交付真实替换，同时删除一条旧分支

不建议大爆炸重写，不建议先把所有 SPI 定义完再找消费者。按风险与依赖推进以下纵向切片；跨平台发布时验证真正涉及的平台行为，不把所有平台全量测试作为每次文档/局部改造的门槛。

| 阶段 | 交付内容 | 必须退出/收敛的旧结构 | 可观测验收 |
|---|---|---|---|
| A. 统一贡献与执行描述 | 确定统一 Plugin/Release/capability 身份；N1/M1 用执行 descriptor 表达差异 | 活动 Snapshot 双 capability 集、消费者中的 M1 专用判断；历史数据做明确迁移 | N1 与 Service Tool 经同一 Agent 调用入口；原有数据可读、失败可解释 |
| B. 第一个可替换契约 | 复用 Role/Provider；一内置一用户实现；开放 N1 相应合同与 adapter | 对该能力的具体内置 ID 直连和 bundled-only 准入 | 用户选自己实现后，内置路径不再被调用；旧 Session 保持原绑定 |
| C. 闭合现有声明能力 | Context、Skill、Resource、MCP、Service resource 按真实消费者逐项接通 | Skill 名称降级、重复 MCP 工具、固定资源特判逐项退出 | 插件包唯一 Skill 可用；Context 真注入；自定义资源被真实调用并释放 |
| D. 完整 Agent UI | 公共会话 API、插件 Agent 页面、之后扩展到 Shell | 页面直接耦合内部业务 store、插件仅能 KV/Service 的封闭边界 | 插件 UI 完成发送/流/取消/历史；切 UI 不重放 turn；恢复入口可用 |
| E. Rust process SDK | 同一契约与公共执行协议；平台制品选择、取消、升级/回收 | 不再让能力资格依赖语言；避免另建 Rust Catalog/Preset | JS/Rust 分别实现同一能力；崩溃/超时不拖死宿主；清理进程树 |
| F. Nomi 策略与 Runtime | 抽取模型/Context/执行接口，接入第二个真实 Runtime | 生产 Nomi-only handle、该路径重复 projection/组装 | 两种 Runtime 均可真实完成 open/turn/observe/cancel；状态归属唯一 |
| G. 深层宿主与可选 Wasm | 存储、调度、认证/隔离等按第二实现需求提取；评估 Wasm | 新端口接入时删除对应硬编码，而非永久双主链 | 存储迁移/重启恢复、权限边界；Wasm 有代表性性能和隔离收益 |

A/B 的边界确定后，C/D 可以按不相交模块安排实施；E 不必等全部 UI 完成。但本次只给出路线，没有启动多代理开发或任何代码迁移。

### 20.1 一条端到端样例胜过一套庞大的门禁体系

建议选一个“研究助手”参考插件：自定义搜索 Tool + 配套 Skill + Context + Agent 页面；之后让 Rust Provider 替代搜索，并接入一个不同 Runtime。它逐步验证整个发布、选择、装载、调用、观察、取消、升级链。

测试重点是行为：真实选中了谁、读到了哪个 Skill 正文、资源是否释放、事件是否连续、取消是否有效、故障能否恢复。结构测试/manifest/digest 可以辅助，但不能代替真实消费者，也不能成为必须长期双写的另一套系统。

### 20.2 防止架构复杂度继续增长的实施纪律

1. 每增加一个公共抽象，说明至少一个真实消费者与替代实现；没有替换收益的不拆。
2. 统一的是领域身份、选择规则与契约，不是强行统一所有后端内部实现。
3. 一份事实只持久化一处；投影可重建，避免 Registry/Catalog/Session 各存可独立修改的同一配置。
4. 每个迁移切片列出旧路径退出条件；数据迁移保留必要历史，临时协议适配明确期限。
5. 不把所有函数 RPC 化，不把所有失败 durable 化，不把每个按钮插件化。
6. 不以新增 Wasm、Rust dylib、第三个 JS runtime 来掩盖现有宿主接口缺失。
7. 公开接口必须有调用方与实现方的契约测试；接口版本发生不兼容变化时显式升级，不静默猜测。

## 21. 本次复核口径与最终判断

### 21.1 证据范围与限制

本次基于报告基线提交及当前工作区静态阅读，核对了 N1 校验、动态 Catalog 准入、Kernel Role/Provider/Context/Resource 接口、Skill 编译与 Nomi 装载、M1/Surface Bridge、React Router、Runtime handle、模型接口和 Session 重编译。相关代码证据已就近列在各节；外部语言/隔离判断引用官方技术文档。

研究期间工作区出现了并行代码修改及文末统一实施基线补充，均予以保留；附录只调整章节编号，未重写实施内容。本报告的现状以开头提交和本次实际读取的链路为依据，不将并行未完成改造计为已验证交付。

没有执行生产服务、真实模型请求、插件性能基准或跨平台 UI smoke。因此：

- 现状结论是代码链路判断，不新增“线上验证通过”或“三平台已兼容”的声明。
- Rust/Wasm 的性能收益、进程粒度和 UI 隔离交互需要代表性 spike 验证，不能从语言名称推导性能倍数。
- §14～20 的成本是相对工程复杂度。新增 §25 提供带范围和假设的规划级人周粗估，不是实测工期或交付承诺；需经 §26 的启动验证校准。
- 本次只修改这份报告，验证文档差异、引用目标与格式，不运行无关的完整构建/测试。

### 21.2 最终判断

**目标可实施，而且现有 capability、Role/Provider、JS adapter、Surface 和 Session facade 已提供有价值的基础；但当前仍是“业务能力可装配”，不能描述为“全栈主机部件可替换”。**

最值得投入的不是“把全部逻辑改成 JS”或“让 Rust 动态库接管宿主内存”，而是：

1. 让所有可替换实现进入同一 capability/Role 体系，用户显式选择，内置实现不例外；
2. 把 UI、Nomi 策略和完整 Runtime 变成真正有消费者的公开边界；
3. 修复 Skill/资源等已经声明但未完整消费的断链；
4. 以一份冻结执行计划缩短组装链，运行时只绑定与校验必要的动态事实；
5. 用共用协议支持 JS 与 Rust 进程插件，按实测需要引入 Wasm；
6. 把最小信任根与部署级替换讲清楚，不把必要安全不变量扩大成产品功能封锁。

可概括为：**同一能力体系、多种实现语言、显式组装、单一执行真相；默认内置组件也是可以被用户替换的实现。**

## 22. Rust 插件的设计层级：并列执行后端，不是第二套平台

### 22.1 先把三个容易混淆的 runtime 分开

| 概念 | 当前/目标职责 | Rust 是否需要另做一套 |
|---|---|---|
| 语言运行环境：当前 `nomifun-js-runtime` | 发现、下载、探测、选择 Node 及其版本 | **不需要对应的 Rust 环境管理器**；用户安装预编译 native 制品，不安装 rustc/Cargo |
| 插件执行后端：JS Host + Kernel adapter | 装载模块、调用 capability、管理实例、取消和回收 | **需要并列的 native-process adapter 与 Rust SDK**，与 JS 后端同级 |
| Agent Runtime：Nomi 或替代引擎 | 拥有 Agent Loop、模型/工具编排和会话执行状态 | 与实现语言正交；一个 Rust Tool 不是一套 Agent Runtime，完整替代引擎另列工作包 |

直接依据：

- [`nomifun-js-runtime/src/lib.rs`](../../crates/backend/nomifun-js-runtime/src/lib.rs) 明确只负责 Node discovery/probing/fingerprinting/selection，不拥有 Catalog 或插件部署。
- [`managed.rs`](../../crates/backend/nomifun-js-runtime/src/managed.rs) 处理 Node 下载/解包；Rust native 制品没有相同的运行环境下载需求。
- [`nomifun-js-host/src/lib.rs`](../../crates/backend/nomifun-js-host/src/lib.rs) 与 [`nomifun-js-kernel-adapter/src/lib.rs`](../../crates/backend/nomifun-js-kernel-adapter/src/lib.rs) 分别承担进程通信和 Kernel 注册适配。
- [`nomi-process-runtime/src/lib.rs`](../../crates/shared/nomi-process-runtime/src/lib.rs) 已提供 `ChildProcessBuilder`、进程树清理、监督和恢复 primitive；native 后端应复用它们。
- [`package.rs`](../../crates/backend/nomifun-agent-contracts/src/package.rs) 当前 entrypoint 枚举仍是 InProcess/JavaScript；因此 native 制品声明、校验和装载确实是新增工作，不是已有功能改个名称。

这些是本轮核对的职责事实，不代表工作区正在进行的 Plugin 统一改造已验收完成。

### 22.2 正确的同级关系

```text
统一 Plugin / Release / Capability / Role / Preset / Session
                            │
                  公共执行契约与授权上下文
                            │
       ┌────────────────────┼────────────────────┐
       │                    │                    │
 JS 模块执行后端      Native 进程执行后端     内置 Rust adapter
 Node + JS SDK       预编译程序 + Rust SDK    宿主静态链接
       │                    │
       └──── 复用进程管理、传输基础和制品管理 ────┘

Wasm：未来可增加的执行后端，本期不建设。
Agent Runtime：上述后端能承载的一种契约实现，不是第四种语言。
```

因此答案是：**在“插件执行后端”层面与 JS 同级；在“插件平台”层面共用一套；在“语言环境管理”层面不对称，不应照抄 JS runtime。**

推荐内部称 `native-process`，而不是把执行协议命名为 `rust-runtime`。Rust 是首个官方 native SDK；将来其他语言若实现同一协议，不需要新增平台类型。但首期不承诺其他语言 SDK。

### 22.3 共用什么、新增什么、暂时不做什么

| 类别 | 具体内容 | 安排 |
|---|---|---|
| 原样或少量适配复用 | 插件产品/发布/安装、Catalog、Role 选择、Snapshot、资源授权、日志入口 | 不建 Rust 专用副本，不新增独立 Rust 插件工作台 |
| 提取小公共边界 | 请求/结果/错误/取消/有界事件、握手版本、调用上下文 | 由共同工作包 P2 完成；JS 保留模块装载特有行为，native 保留启动特有行为 |
| 复用底层生命周期 | process supervisor、进程树终止、stderr/stdio、制品校验与路径边界 | 复用 primitive，不把 Node mount 状态机硬搬到 Rust |
| native 特有新增 | 按 OS/架构选择预编译制品、可执行入口、实例启动、协议握手、native Kernel adapter | 工作包 P5；首期 Windows x64 一个目标 |
| Rust 作者体验新增 | 小型 SDK、示例、构建/打包说明、调试日志和契约测试 | 作者机器或 CI 使用 Cargo；用户安装时不编译源码 |
| 不建设 | Rustup/Cargo 版本管理、另一套 registry、Rust 专用 Preset、dylib ABI、自定义编译云 | 不进入排期，不以“以后可能用到”为由预建 |

Rust 程序不需要语言解释器，但可能依赖系统库、MSVC runtime 或其他 native library。发布 manifest/打包检查仍需识别目标和依赖；“预编译”不等于“一个二进制在所有系统运行”。首期只接纳明确满足目标要求的制品，缺失依赖给出可解释错误。

公共协议采用适合现有代码的版本化 stdio 请求/响应与有界事件即可。标准输出用于协议，日志写标准错误；宿主验证请求关联和已握手版本。先支持实际需要的操作，不为尚未开放的模型流、任意 callback 或跨机 RPC 定义完整框架。

### 22.4 首期 Rust 支持的上限

首期交付：可信本机、预编译、Windows x64、一个 release 选择一个后端，支持 Tool 和本期已开放的 Context 契约，复用 Role 绑定与正常安装/升级/禁用流程。可附带 UI 资源，但不要求同一包同时协调 JS 服务与 Rust 服务。

启动进程按插件实例/授权作用域隔离，必要时惰性启动；实例只由一个生命周期 owner 管理。首期不做跨租户共享池、运行中热换二进制或任意多进程编排。即使是可信插件，也要有超时、取消、协议大小边界、故障诊断和进程树回收。

完整 Agent Runtime、跨平台 native 分发、Wasm、强沙箱、源码插件自动构建都不计入这个交付。它们不是 Rust Tool 插件的前置条件。用户对本机 native 代码的信任必须明确；digest 只校验完整性，不能证明作者可信。

## 23. 把大目标分成可以独立交付的版本

### 23.1 不把全栈目标压进第一个版本

整体确实是一个中大型架构项目；“开放所有环节”不是若干开关的工作量。最大的成本是拆出真正稳定的消费边界、迁移现有用户路径并删除旧分支，而不是增加一种编程语言。

建议按以下三个版本组织，版本名是本文计划标签，不要求新增产品版本体系：

| 版本 | 对用户能承诺什么 | 包含哪些工作包 | 明确不承诺什么 |
|---|---|---|---|
| R1：可替换能力与 Agent UI 的开发者预览 | 一个真实系统契约有内置/用户可选实现；Tool、Context、插件 Skill 真正消费；插件完整 Agent 页面；JS 与 Rust native 实现可安装 | P0～P6 | 不叫“任意主机部件全栈完成”；不含整个 Loop 替换、完整 Shell、任意资源 Provider 或强沙箱 |
| R2：开放式 Agent Runtime 平台 | 扩展资源/MCP/lifecycle；模型与关键 Nomi 策略可替换；第二个真实 Runtime；完整 Shell | P7～P10 | 不承诺不同引擎 checkpoint 通用或所有宿主设施均已可替换 |
| R3：部署级主机开放与目标平台发布 | 存储/调度/认证等选定宿主服务可在启动时替换；目标平台验证与发布 | P11～P12 | 不承诺无条件热换信任根，不把所有第三方实现的正确性包办 |

§16 的完整矩阵仍是方向，不能因为先发布 R1 就把剩余环节永久封闭。R2/R3 的开发清单在 R1 真实使用后细化；没有第二实现需求的内部函数不提前抽象。

### 23.2 R1 的范围要足够小，也要真实有价值

R1 以一个“研究助手”插件为纵向验收样例：选择自己的搜索/检索实现，加载包内 Skill 与 Context，在插件 Agent 页面完成一次真实会话。native SDK 再提供相同契约的 Rust 实现，验证替换不依赖语言。

至少选一个**原本由系统固定调用、会在真实产品路径中使用的部件契约**。不能只给两个新增 Tool 起相似名称就当作系统替换完成。P0 决定具体示范部件；若搜索并无真实系统固定调用点，就选已有 Context 或另一实际部件。

R1 的边界：

- Provider 的注册、选择、冻结与实际调用一致；无需每次手工 Preview/Compile，提供“保存并使用”的正常入口。
- Skill 优先交付锁定正文、只读引用资源、索引/按需读取与已支持的 Tool 依赖；shell/fork/hook 组合若不能完整保持授权语义，应明确拒绝该模式并留在 R2，不能静默降级。不是另写一个 Skill 解释器。
- Plugin Agent 页面覆盖发送、流式结果、取消、历史、错误与重新订阅；不是仅能显示一段对话的展示页。完整工作台 Shell 和像素级区域替换后置。
- 资源先复用现有 typed binding；自定义 kind、多实例资源、M1 带资源 Service 的全面开放放 P7。R1 首个替换例子不得依赖未完成的资源扩展。
- 先保留有正确性价值的 Session 重编译保护，只统一选择与投影语义；删除重复解析要等完整执行 descriptor 和等价校验就绪，归入 P8/P9。
- 一个后端入口可配 UI，多个后端的包内编排后置；不做新的插件市场、付费分发系统或在线编译平台。

### 23.3 可选的更小首发

按本轮暂不新增 Rust 插件的情景，先规划 **R1-JS**：保持同一公共边界，移出 P5 的 native 后端；P0 不再要求 native 小样，改为验证内置 Rust 与用户 JS 能共同消费契约。JS 的 Provider 替换、Skill/Context、插件 Agent 页面仍能形成实际产品价值。

Rust 保留为可选后续投资，不是 R2/R3 的阻塞依赖，也不表示用户已决定永久取消。只保持语言无关的数据边界，不预建空壳 native 框架。§27 给出该情景的完整任务对应；其余含 Rust 的版本/排期表用于对比，不应混排。

## 24. 可执行工作包、依赖与代码责任范围

### 24.1 R1 工作包

下表是规划级估算，1 人周 = 5 个全职工程工作日，假定熟悉 Rust/TS 与本仓库。工作量包含本包实现、定向测试、必要文档和正常 review；P6 只计跨包集成及发布验证，避免重复计算模块测试。多人参与一个包时按总人周计，不按等待时长计。

| 包 | 交付物与明确退出条件 | 主要责任范围（现有模块或拟新增薄适配） | 依赖 | 基础人周 |
|---|---|---|---|---|
| P0. 基线与两条验证 | 固定正在改造后的候选基线；选定真实替换部件；native 小样跑通调用/取消/崩溃；UI 走现有 API 完成一次发送/观察；据结果重估 | 报告、现有示例/测试、contracts/runtime/UI port 的最小 spike | 无；不等待所有长期功能设计 | 1～2 |
| P1. 统一主链收口 | 验收当前 Plugin 身份、Catalog、Snapshot 与 Session 合流；相关旧分支退出；N1 与发布型 Service 使用统一消费入口 | contracts、control-plane、kernel、plugin-platform、DB、app；已有统一改造优先完成 | P0；若并行工作已验收则扣除已完成部分 | 2～4 |
| P2. 第一个真实替换与公共协议 | 内置 + 用户 JS Provider 同契约；默认/Agent override；共用调用 DTO、错误、取消；Preview/Save 同一选择语义；真实调用不再直连内置 ID | contracts/kernel、JS adapter/host、app composition、Agent 工作台选择 UI | P1 的契约及统一主链稳定 | 3～5 |
| P3. Context 与 Skill 闭环 | 用户 Context 真注入；Skill 按精确制品装载正文/资源；依赖与不支持模式明确报错；旧目录来源继续可用 | plugin_tools、conversation skill_resolver、skill-library、Nomi bootstrap/SkillTool | P2 公共边界；先定 runtime descriptor | 3～5 |
| P4. 插件 Agent 页面 | UI contribution/绑定；会话 command/query/event Bridge；插件界面收发、取消、读历史、恢复订阅；内置界面可恢复 | UI Surface/Router、公共 bridge/types、app 会话 API；业务 API 仍由现有 owner 提供 | P2 的选择/身份边界；不依赖 P5 | 3～5 |
| P5. Native 后端与 Rust SDK | 预编译 native 制品正式安装、选中、调用与升级；Rust Tool/Context；错误、取消、退出清理；同一产品入口 | 新 native-process adapter/SDK、共享 process-runtime、少量制品校验/app 接线 | P2 协议稳定；Context 联调依赖 P3，但 Tool 可先完成 | 3～5 |
| P6. R1 集成与开发者预览 | 参考插件全链路、故障恢复、升级不漂移、Windows Web/桌面验证、可复现打包；文档和诊断可交给外部开发者 | 集成测试、发布脚本/示例、各包 owner 联合修复 | P3/P4/P5 | 2～3 |
| **合计** | **R1 基础工程量，不含风险余量** | — | — | **17～29** |

P1 不是要求把正在进行的统一工程再重做一次。当前工作区已包含大量 contracts/DB/app/UI 的统一改动；本轮没有验证其完整性。P0 按退出条件区分“已验收”“仍需集成”“尚未完成”，只估剩余工作，不把 Git 修改文件数量当进度。若发现基础功能损坏需要大范围修复，应重估 P1，而不是把修复隐藏在 P5 的 Rust 预算中。

具体 crate 名称只是责任定位，不预先规定新增十几个 crate。优先在现有模块内建立清楚接口；native adapter 和可独立依赖的 Rust SDK 确有发布/依赖隔离需要时再建小 crate。

### 24.2 Rust 专属新增工作量如何构成

P5 的 3～5 人周对应以下 15～25 人日。共同能力契约/协议提取已计入 P2，P6 只做跨产品联合验收，三者不能重复报账。

| native 子任务 | 基础人日 | 交付边界 |
|---|---|---|
| 目标/入口声明与制品校验 | 2～3 | Windows x64，精确 release/entrypoint，缺失目标/依赖报错；不执行安装期任意构建脚本 |
| 启动、握手、调用、取消、回收适配 | 4～6 | 复用 process-runtime；native 与 JS 使用公共调用语义，但不复制 Node module loader |
| Rust SDK 与参考 Provider | 2～4 | 作者实现 Tool/Context，不手写协议；可运行的同契约替代实现 |
| 构建/打包与产品状态接线 | 2～4 | 作者/CI 构建，复用安装入口；产品显示目标、就绪/错误和日志 |
| native 故障契约测试、文档与 review | 5～8 | 崩溃、超时、协议错误、旧版本、禁用、重启与进程树清理 |
| **合计** | **15～25** | **不含跨平台/强隔离/完整 Agent Runtime** |

这是“公共基础已就绪后”的边际预算，不是从零开发整个插件平台只需 3～5 人周。若实测表明必须重写现有进程管理或全面改造多版本制品存储，该前提不成立，应回到 P0/P2 重估；不能继续沿用这个数字。

### 24.3 后续版本工作包

| 包 | 具体范围与退出条件 | 依赖 | 基础人周 |
|---|---|---|---|
| P7. 扩展贡献闭环 | 自定义 ResourceProvider 与 Service typed resources；MCP mapping 统一调用；有限明确的 middleware/event/lifecycle 契约；补齐 R1 暂不支持的 Skill 执行模式 | R1；复用同一授权/执行接口 | 5～9 |
| P8. Nomi 关键策略与执行计划 | 模型调用/路由、Context 管线、压缩/历史视图与工具编排的有意义接口；有第二实现；完整 descriptor/等价校验到位后退出重复求解 | R1；依赖资源的策略需 P7；与 P9 共用 Runtime 边界评审 | 6～10 |
| P9. 完整 Runtime 替换 | Nomi 与一个真实外部 Runtime 通过统一 Session 接口运行；事件/取消/可选恢复一致；只保留一个执行 owner | R1；与 P8 先固定外层契约，不要求 P8 所有内部策略先完成 | 5～9 |
| P10. 完整 Shell | 导航、页面/布局选择、应用状态重建、恢复入口；不与 UI 插件共享宿主内部可变 store | P4/P6；独立于 P8/P9 引擎内部实现 | 3～6 |
| P11. 部署级服务开放 | 按真实需求依次提供 Transport/Scheduler、存储、认证/Secret、监督/隔离后端的替换接口；默认实现同接口；选定第二实现与维护窗口切换 | R2 已验证的 Session/资源边界 | 8～14 |
| P12. 目标平台与发布收口 | 已选定 native/UI/runtime 组合在 Windows x64、Linux x64、macOS arm64 验证；对应打包、依赖、必要签名/公证和发布说明 | 可提前做单平台 spike；正式收口依赖所发布范围稳定 | 4～7 |
| **后续合计** | **R2：19～34；R3：12～21** | — | **31～55** |

P7～P12 是低置信度的预算占位，不能直接作为开发承诺。其中 P11 跨越多个领域，正式启动前必须拆成独立服务小包；8～14 人周以复用现有设施并选择有限、代表性的第二实现为前提，不包括重做认证产品、数据库系统或自制 OS sandbox。§27.2 已将此前未明确分配的策略、UI 子范围等补入对应包；这些补项需拆卡重估，不能默认全部被本表原预算吸收。

完成上述工作代表所列主干部件具有真实替换路径。原始五项需求及 §27.2 的 29 个环节仍属于目标范围；旧估算未拆细的部分是需要补估的缺项，不能改称用户后来扩展了需求。每个历史实现、额外 native 平台及所有策略组合的穷举验证则不由这张粗估表承诺，应按发布支持矩阵另定。不能把主干预算包装成完整目标的固定总价。

### 24.4 依赖与允许并行的位置

```text
P0 → P1 → P2 ─┬→ P3 ─┐
               ├→ P4 ─┼→ P6 / R1
               └→ P5 ─┘   （P5 的 Context 联调与 P3 汇合）

R1 ─┬→ P7 ─────────────┐
    ├→ P8 ↔ P9 ───────┼→ R2 → P11 ─┐
    └→ P10 ───────────┘             ├→ P12 / R3
           目标平台 spike 可提前 ────┘
```

图中的 P8 ↔ P9 表示共享契约协调，不是编译时相互依赖；应先由共同 owner 固定外层 Runtime port，再分别做 Nomi 内部策略和外部引擎适配。

P1/P2 是首期关键路径，不宜让多人同时改同一份 contracts/compiler。P3/P4/P5 在接口稳定后才是适合并行的任务；它们共享的 app composition 接线由一个集成 owner 管理。阶段依赖满足即可合流，不要求先完成与该切片无关的全部未来 SPI。

## 25. 工作量、团队配置和日历安排

### 25.1 粗估的前提与可信度

以下是用于预算和组队的区间估算，不是根据现有历史交付速度计算出的承诺。需要在 P0 后首次校准，在 P2/R1 结束后再次更新。

估算前提：

1. 核心开发者熟悉本仓库、Rust async、TS/React 和跨进程通信；新人学习成本另计。
2. 复用当前插件发布/制品/进程管理/Session 设施；并行统一改造会先形成可验证基线，不从零重写平台。
3. R1 优先 Windows x64 的 Web 与桌面路径，native 插件为显式信任的本机预编译程序；不含恶意插件强沙箱。
4. 不建设新市场、远程构建、自动迁移任意引擎状态；R1 只支持 §23 明确的功能子集。
5. 团队能持续投入，且有真实模型测试条件；发布平台设备、签名/公证账户等可获得。外部等待可能延长日历，但不直接等于工程人周。
6. 普通模块测试/review 已计入工作包；集成包处理跨路径问题。额外 20%～30% 是未知返工风险，不是把正常测试漏到最后。

已有模块职责的复用判断可信度较高；R1 工程量可信度中低；R2/R3 尤其深层服务拆分可信度低。使用 AI 辅助可以提高部分实现效率，但没有本项目实测前，不按固定倍数缩短测试、集成和架构校验时间。

### 25.2 总量与 Rust 的占比

| 交付范围 | 基础工程量 | 加 20%～30% 风险余量后的预算区间 | 解释 |
|---|---|---|---|
| R1-JS：暂缓 native 正式支持 | 14～24 人周 | 17～32 人周 | 保守地仅扣除 P5；P0 改做内置 Rust/JS 契约验证，P2 保留；不额外乐观扣减 P0/P6 |
| R1：包含 Rust native | 17～29 人周 | 21～38 人周 | 首个有实用价值的开放平台预览，不是全栈终点 |
| R2 增量 | 19～34 人周 | 23～45 人周 | 模型/策略、Runtime、资源/lifecycle、Shell |
| R3 增量 | 12～21 人周 | 15～28 人周 | 部署级服务及所列目标平台收口 |
| **R1 + R2 + R3 主干累计** | **48～84 人周** | **58～110 人周** | 对基础合计统一加余量；分项四舍五入可能略有差异 |

P5 的 Rust 专属新增是 3～5 人周，占 R1 约六分之一的量级；这个比例只帮助判断重点，不代表每个实际项目都相同。公共协议/Provider 改造对 JS 本身也是必要投入，不能全部算成“为了 Rust 额外付出的成本”。

但也不能说“Rust 很便宜，只有一个 adapter”：native 的打包、版本、故障清理和用户信任边界都在 P5 内。若要求一开始同时支持三平台、强沙箱、完整 Agent Runtime，新增范围会明显超过 P5。

结论：**整体工作量确实大；Rust 不是让它翻倍的主要因素。** 优先控制首发范围及统一链路，比取消 Rust 或多开几个开发分支更能控制总成本。

### 25.3 推荐团队：两名后端 + 一名前端，集成职责明确

| 角色 | 主责任 | 关键配合边界 |
|---|---|---|
| 后端 A / 技术负责人 | contracts、Role/Provider、compiler/Kernel、Session 一致性；统一接口与合流 | P1/P2 单一接口 owner，P8/P9 外层 Runtime 契约负责人 |
| 后端 B / 运行集成 | Context/Skill、native adapter/SDK、process/资源接入、故障测试 | R1 同时有 P3/P5，需与 A 在 P2 完成后分担，不能假定两包由 B 同时满速完成 |
| 前端 / 产品集成 | Provider 选择体验、UI Host API 的前端部分、插件 Agent 页面、后续 Shell | 与后端 A 约定 Session API；不复制业务 store 或另建会话后端 |
| 测试/发布责任 | 模块 owner 自带测试，一名主责组织 E2E/发布 | 可由上述人员轮值；若另配 QA/平台工程师，其容量和成本单列，不当作免费劳动力 |

不建议一开始拆成“JS 团队”和“Rust 团队”各设计一套契约。应按公共内核、运行集成、UI/产品边界拆分，语言后端只是其中的实现任务。

### 25.4 三人团队的参考日历

本节日历按含 native 的 R1 编排；JS-only 不执行其中 P5/native 验证安排，使用 §27.3 的依赖和 §27.6 的规划窗口。

按约 70%～80% 的可排开发容量考虑 review、沟通与日常维护，另受关键路径制约，R1 可先按 **12～18 周日历**做预算窗口。不是把 17～29 人周直接除以三。

| 时间窗口（允许交叠） | 后端 A | 后端 B | 前端 | 里程碑 |
|---|---|---|---|---|
| 第 1～2 周 | P0 基线/替换部件与契约确认 | P0 native 协议/取消验证，核查 P1 剩余项 | P0 会话 UI API 验证，列现有入口缺口 | 第一次重估，明确首发范围 |
| 第 2～4 周 | P1 contracts/compiler 主链收口，启动 P2 | P1 app/运行集成，准备 P3 descriptor | P1 UI 类型合流；P2 选择界面 | 统一基线可用，不再让新功能依赖两套 Snapshot |
| 第 4～7 周 | P2 真实 Provider 替换与公共协议 | 配合 P2 JS host/adapter；随后启动 P3 | P2 产品入口，随后 P4 页面与 Bridge | 首个内置部件被用户实现实际替换 |
| 第 6～12 周 | P2 收口后承担 P5 主体与 app 接线 | P3 Context/Skill，支持 P5 Context 联调 | P4 插件 Agent 页面、故障/恢复交互 | 三条支线汇合；内部开发者可试用 |
| 第 12～14 周 | P6 联合修复/发布检查 | P6 native/Skill 故障验证 | P6 Web/桌面真实流程验证 | 条件满足时发布 R1 |
| 最迟预算至第 18 周 | 按实际风险重排，不新增范围 | 同左 | 同左 | 覆盖未知集成返工；超窗必须重估，不无期限顺延 |

窗口重叠表示契约稳定后允许后续工作开始，不表示同一个人能在多个包各投入 100%。若 P1 已由当前并行工程验收完成，计划可前移；若 P2 需要大改调用语义，则下游日期相应重估。

其他团队规模的 R1 日历参考：

| 持续投入人数 | 含正常协作与风险的规划窗口 | 说明 |
|---|---|---|
| 1 名全栈/后端主力 | 约 26～48 周 | 还要兼顾 UI；若不熟悉前端则更长，宜先发 R1-JS 或减少非核心 UI 范围 |
| 2 名互补开发者 | 约 14～26 周 | 并行度受后端共享文件与测试牵制 |
| 3 名上述配置 | 约 12～18 周 | 推荐的首期组队规模 |
| 4 名以上 | 约 10～16 周起评估 | 可以增加测试/平台专人，但 P1/P2 关键路径不能按人数无限缩短 |

以三人稳定团队看，R1～R3 主干累计可先按 **约 7～12 个月量级**理解，而不是把 R1 的窗口当全部完成时间；实际会受 R1 反馈、R2/R3 选定服务数量和平台条件影响。若要求一人承担完整目标，则是更长期工程，应该按版本分批投入。

### 25.5 最容易让预算失真的附加要求

| 附加范围 | 是否已经包含 | 处理方式 |
|---|---|---|
| Linux x64 / macOS arm64 native 与桌面发布 | R1 不含，P12 含所列组合 | 如果前移到 R1，就搬移相应预算和平台等待，不能要求首期同价提前完成 |
| Wasm 首个受限执行后端 | 不含 | 可先占 4～8 人周基础探索/接入预算；正式支持按 imports、异步与工具链 spike 重估 |
| 不可信 native/Node 插件的跨平台强沙箱 | 不含 | 是独立安全工程；可先留 8～16+ 人周探索/整合占位，但未选定 OS 机制前没有可靠总价 |
| 多后端同包编排、远程插件、在线构建服务 | 不含 | 单独提出产品需求和预算，不能塞入 native adapter |
| 任意旧开发数据库/历史协议全部无损兼容 | 不含无限兼容 | 按需要保护的真实数据确认一次性迁移范围；不能借此自动清空数据 |
| 全部内置功能逐个建立第二实现 | 不含无限展开 | 每个领域单独定义消费者与验收，按实际范围累加 |

上述附加数字同样是低置信度预算占位，不应直接相加得出合同报价。尤其强沙箱的威胁模型、系统支持范围不同，成本可能超出占位。

## 26. 如何启动、验收与控制范围

### 26.1 建议现在只下达第一批明确任务

**先启动 P0，并收口正在进行的统一改造；不要同时启动 P0～P12。** 本轮只是形成此计划，实际代码任务应另行下达。

P0 的建议执行顺序：

1. 固定一个可验证候选提交，记录当前统一工程已通过的测试和未完成项，保护工作区其他修改。确认开发数据处理范围，不在核对阶段做破坏式重建。
2. 从实际产品调用点选一个替换部件，写出“内置实现/用户实现分别何时被调用”的小测试；据此定义最小 Role/Provider 合同。
3. 用一个最小 Rust 程序验证宿主启动、握手、调用、超时取消、崩溃与回收；同一调用数据走一次 JS，验证公共协议没有 Node 私有对象。这个小样不是正式 SDK 或插件发布完成证据。
4. 用现有 Session API 验证插件页面所需的发送、事件、取消、历史；列出真正缺失的 Bridge 能力，避免先造大而全 UI SDK。
5. 给出 P1 剩余工作、P2 接口草案、首发支持矩阵和更新后预算。P0 总投入控制在 1～2 人周；并行资源不足时缩减验证范围，不把它扩大为完整新平台。

JS-only 情景用“内置 Rust adapter 与用户 JS Provider 的相同输入/输出、错误、取消契约测试”替换第 3 步，不开发新的 native 入口；其他步骤保留。

这些短验证通过后，启动 P1/P2；P2 契约稳定后再把 P3/P4/P5 并行安排。P0 小样能直接转为测试或示例就保留，否则删除，不长期留下第四条生产路径。

### 26.2 每包使用同一份轻量任务卡

只需在现有任务跟踪中写清以下内容，无需新建一套证据平台：

- 负责人、允许修改的模块和与其他任务共享的接口；
- 一个可演示的产品结果、明确不做项；
- 前置契约/版本、进入条件；
- 必须退出的旧分支或重复事实；
- 能证明行为的定向测试、人工/真实集成步骤；
- 合流方式、失败时撤销该切片的办法、当前剩余估算。

如果使用多名开发者或 AI 辅助，也按这些职责边界拆任务；不要让多个执行者同时重写 contracts/compiler/app composition。测试与接口 review 由具体 owner 负责，而不是假设增加并行数就自然获得质量。

### 26.3 R1 必须看到的结果

1. 一次真实产品调用分别绑定内置/JS/native 实现，调用记录能确认选中者；不选择的实现不被暗中调用。
2. 插件 Skill 只存在于包内，也能按锁读取正确正文与引用资源；同名来源不混淆；不支持的执行模式提前报错。
3. 用户 Context 真正影响传给模型的输入；不是 Catalog 中有一个条目而 runtime 忽略。
4. 插件 Agent 页面完成发送、流式观察、取消、历史和重载恢复；恢复界面不重发已有操作。
5. 禁用、升级、超时、进程崩溃的行为明确，资源和子进程可回收；旧会话不悄悄改绑新实现。
6. 现有 Nomi/JS/Service 基础流程仍可用；关键旧分支已退出；示例插件作者可以按文档独立构建、安装和调试。

R1-JS 的第 1 项只要求内置/用户 JS 两种实现；不要求 native 安装或故障验收。其余产品退出条件不因省略 Rust 插件而降低。

验证按风险选择：contracts/compiler 变更跑相关 Rust 契约/编译测试，Skill 跑精确装载测试，UI 跑交互与 Bridge 测试，native 跑真实子进程故障测试。跨模块集成或候选发布再跑相应更宽检查；不为每个局部修改要求全仓全平台检查。

### 26.4 三个固定复盘点

- **P0 后**：决定预算前提是否成立，确认 P1 实际剩余量、平台范围和首个替换部件。
- **P2 后**：用实际交付速度更新 P3～P6；若公共接口尚不稳定，不强行并行后续。决定完整 R1 一起发布还是先发 R1-JS。
- **R1 后**：按外部开发者真正受阻的环节排序 P7～P12；用真实数据细化 R2/R3，不继续沿用未经校准的长期人周估计。

若任一包剩余估算超出上界约 30%，或必须新增第二套 Catalog/Session authority、独立语言权限系统、全面 Node Host 重写，暂停扩大该包并重估方案。优先调整范围/拆包/删除不必要抽象，不通过降低验收或无期限延期维持原计划数字。

### 26.5 开发安排的最终建议

以下保留含 Rust 路线的安排；当前讨论的 JS-only 情景以 §27 为准，不要求先完成 Rust 小样或 P5。

推荐采用 **三人核心团队、先做 P0/P1/P2、R1 控制在 12～18 周规划窗口**的安排；正式投入前以 P0 的实测校准。Rust 不单独成立另一条平台主线，而是在公共边界稳定后交付 P5。

资源较少时先发 R1-JS，或者只增加少量角色投入测试/发布；不要同时开展 Wasm、强沙箱、全部内部策略、完整 Shell 和深层存储替换。完整目标保留，但按真实可用版本逐步推进。

这不是把大项目说成小项目，而是让每一笔投入都产生可验证的用户能力，并能在 R1/R2 之后自主决定下一阶段投入，而不必等待“全栈全部完成”才获得价值。

## 27. 回到原始五项需求：是否真正解决，以及 JS-only 路线的完成标准

### 27.1 直接结论与逐项对应

**完整设计针对这五项需求，但不能说“第一版全部解决”，也不能说“已有工作包已足够精细地覆盖所有环节”。** 本轮核对发现：P7/P8/P11 仍是大范围占位，工具发现、记忆、规划、协作、观测和部分 UI 的责任与验收此前不够明确。本节补齐对应关系，不把研究结论当作实施完成。

这里“解决”采用同一个标准：**用户可发布 → 可在产品中选择 → 真实消费者执行所选实现 → 失败/撤销/升级行为可验证 → 原内置旁路退出。** 只有 SDK、trait、manifest 字段或测试替身不算解决；需求 1/5 是所有部件的共同标准，而不是做完一个 Provider 示例后就独立关单。完整设计也保留明确例外，不能将“任意”解释成无契约兼容要求、无授权边界或任意时刻无迁移热换。

回答“这些工作都有解决吗”时，必须使用同一个范围：**设计覆盖：五项都有；当前实现：五项都只能部分计入或尚未闭环；完整交付：须包含 R2/R3，不能只交 R1-JS。** “任意部件”若指所有部件均可由普通插件随时接管，则方案仍不满足；§27.4 的部署级与信任根边界是需要明确接受的产品口径，不能当作用户已经同意缩减目标。

暂不增加 Rust 插件，主要放弃的是用户分发预编译原生实现的便利，不是放弃 JS 插件对 Agent 业务部件的替换能力。仍需修改 Rust 宿主的实际调用边界；“不新增 Rust 插件”绝不等于“不改 Rust 代码”。

用于开发决策的口径是：**接受 JS-only 技术路线，不等于接受缩减五项需求；接受分期，也不等于接受把未交付项记为已解决。** 若下一批只安排 Provider、Context 和 Skill 而没有 P4 的系统页面替换，需求 2 就没有被这一批解决；若后续只做 P9 的外部引擎而不做 P8 的 Nomi 内部组件，需求 3/5 也仍未满足。判断范围看实际交付物，而不是“全栈插件”名称。

| 原始需求 | 方案与当前完成状态（2026-09-14） | R1-JS 计划交付，不代表已完成 | 完整需求仍需什么 |
|---|---|---|---|
| 1. 用户 capability 与内置并存并可选用替换 | **部分实现**。独立 ID、通用 JS 分发、候选目录、Agent/产品默认选择保存、Provider 漂移检查、跨包恢复、独立冲突及递归依赖/用途隔离已有切片证据；升级影响、其余调用类型与全量领域闭环未完成，详见 §14/§27.8、台账 §2.15 | P1/P2：统一目录与选择，至少一个真实内置部件被用户 JS 实现替换 | P7/P8/P9/P11：把后续每个部件的消费者接入绑定；不同内部依赖/资源可组装；用户契约可跨包消费；不能以一个样例宣称全部内置可替换 |
| 2. 用户替换系统 UI | **完整替换闭环仍未完成**。已有隔离 Surface、命令/流接缝、可选页面、预设默认与会话前工作台入口切片（台账 §2.19～2.23、§2.30～2.31）；完整浏览器恢复及 Shell 不能被这些证据抵扣。方案仍为 UI contribution + 公共应用 API，详见 §15/§27.8 | P4/P6：完整 Agent 页面，不只是装饰面板 | P10：Shell、主题/结果呈现及有独立意义的页面区域；独立客户端验证；桌面特权通过 P11 的部署接口，不等于 iframe 接管桌面容器 |
| 3. 全环节进一步插件化 | **仅有部分环节的证据**。Tool、初始/动态 Context 与通用 Role 接缝不能代表全部；动态 Context 已进入每次主模型推理，但不是完整 Prompt 管线；采用粗粒度接口和 JS DTO/流/资源 handle，详见 §16/§17、台账 §2.10 | P2/P3：第一个部件、Context、Skill；Tool 原路径统一 | 按 §27.2 的 29 行逐个闭环；P7/P8 开放内部组件，P9 开放整个 Runtime，P11 处理部署服务；Rust 后端不是前置条件 |
| 4. Skill 装载链未闭合 | **部分实现**。包内精确来源/制品锁、只读正文/资源、Session/Bootstrap、标准工具和显式命令/补全已有定向证据；包内来源不再靠 ID 搜目录，详见 §18/§27.8、台账 §2.8～2.9 | P3：包内正文、只读引用资源、索引/按需加载和支持的 Tool 依赖；核销已验证子项，不重复实现装载器 | P7：shell/fork/hook 等剩余执行模式及授权、其余来源统一与历史制品保留；fork 须接现有生产委派 owner，见 §18.4；不支持模式明确拒绝，旧格式 Snapshot 不自动补伪造来源 |
| 5. 更简单、开放、稳定的组装 | **有统一主链基础，尚未完成**。目标是一套 Catalog/绑定解析、一份冻结计划、一个 Session 执行 owner；生产 Runtime 仍为 Nomi，详见 §19 | P1/P2：用户选择后“保存并使用”，默认继承和真实调用一致 | P8/P9：完整 descriptor、等价校验、退出重复求解与硬编码调用；P7/P10/P11 不得再各造一套组装系统；按 §27.5 验收简单性 |

这也明确纠正“有相关工作包，所以已经解决”的表述：**上表五项目前都不能按完整原始需求关单。** 但这不否定已经验证的 Tool/Context/Role 子项；它们应在实施台账中保留并抵扣后续工作，而不是重复开发。普通工具由模型选择，与系统部件按用户绑定被替换，必须分别演示。

对原始诉求还有五条不可缩减的口径：

- **需求 1 不只是注册成功。** 用户可以发布独立 capability，交给 Agent 选择；若要接替系统已经调用的部件，还必须实现共同契约并让该消费者使用用户绑定。两条路径都保留，不能要求每个普通工具都先定义 Role，也不能让模型猜测来代替确定的系统实现选择。
- **需求 2 不只是插件面板。** 独立插件页面和完整 Shell 替换均在目标中；使用现有隔离 Surface 是实现手段，不是替换系统 UI 的验收结果。
- **需求 3 不只是替换整个引擎。** 默认 Nomi 内部的模型、上下文、记忆、规划和编排等有意义的部件也要能单独替换；否则用户仍需重写整个 Agent 才能改一项行为。29 行中不能用 Runtime 一行代替其他行。
- **需求 4 是可用性缺口。** 原始包内 Skill 即使已安装、可选择，仍可能没有正文进入运行时，或误用同名目录版本；该只读断点现已修复。剩余执行模式必须继续分别验收，不能把“正文/资源可读”当作脚本/fork/hook 已授权执行，也不把“模型自行猜出了类似做法”当作装载成功。
- **需求 5 是贯穿各包的退出条件。** 每开放一个部件，都要让原内置实现经过同一选择接口，并消除该处重复组装事实；不能等所有接口加完后，再另起一次“大一统重构”清理双轨架构。

因此，五项需求的完成不是分别交五个示例：需求 2/3/4 决定要开放哪些消费者，需求 1 要求这些消费者真正执行用户选择，需求 5 要求它们共用简单、稳定的组装链。只有“逐环节替换成立 + 跨插件组合成立 + 使用与故障体验成立”同时满足，才能关闭相应完整范围。用户定义的新契约也应能被其他插件消费，而不是只能从官方预设 Role 清单中选；但语义相似、名称相同不构成契约兼容证明。

因此，**首发交付的是“可替换平台的真实闭环”，R2 才覆盖主要 Agent 业务环节，R3 才继续覆盖部署级服务。** 每个版本都应公布尚未开放项，不把未来版本承诺算进当前能力。部署级替换和最小信任根的特殊边界见 §27.4。

### 27.2 全部 29 个环节与工作包逐一对账

当前解释：本节保留 29 项长期评估及历史验收标准，不是本次上线欠账。旧 R1/R2、工作包和“每行都要验证”只适用于未来决定交付该能力时，不要求本次全部实现；本次发布门槛以 [六项 P0](2026-09-15-plugin-release-readiness.zh.md) 为准。

下表是 §16.2 的任务/验收补充，不是第二套架构，也不新增平行注册中心。编号仅用于需求跟踪。所有“验收”都是待实施标准，不表示当前已通过。

2026-09-14 的当前代码增量另见 §27.8：初始/动态用户 Context、通用 Role/Provider 及选择/默认管理已有实现切片，但不足以关闭 02/06/11 或完整 Prompt 替换。表中的“R1/R2”表示交付范围归属，不表示该版本现在已经完成。

**每行都要验证公共声明/准入、用户绑定、实际消费者、故障行为和旧分支退出。** 可共享契约测试基础，但不能用一行成功推断其他行成功；若某行延期，应明确登记为未完成。表中 P2 等先交付基础、P7/P8 等补全的行，不能只完成前半段就关单。

| 编号 / §16 环节 | 对应工作包 / 阶段 | 逐项待满足的完成标准（非通过记录） |
|---|---|---|
| 01 Catalog / 发现 | P1/P2：统一目录；P8：发现策略（R1/R2） | 内置/用户贡献同目录可见；可选发现/排序实现，排序不绕过准入或授权 |
| 02 Preset / 依赖 / 模板 | P2：已有契约绑定；P7：用户契约与模板发布（R1/R2） | 自定义命名空间契约能被另一插件依赖；模板一键组装但不携带额外授权，缺失依赖可解释 |
| 03 Session 创建 / 资源绑定 / 初始化 | P2：统一计划；P7：可扩展绑定与初始化（R1/R2） | 新 resource kind 和初始化贡献能实际运行，失败可清理；不依赖固定 kind 白名单或重复资源 owner |
| 04 初始 Tool | P1/P2/P6（R1） | 内置、用户 JS、发布型 Service 经统一入口调用，未选中实现不被暗中调用 |
| 05 on-demand / ToolSearch | P8（R2） | 用户搜索、排序、schema 暴露策略改变实际候选/调用路径；不扩大已授权计划，也不为 deferred Tool 另建 Kernel 激活状态机 |
| 06 Context / persona / system prompt | P3：Context；P8：完整 Prompt 管线（R1/R2） | 实际模型输入可捕获并核对来源/顺序；替换整条管线后无隐藏内置 Prompt 拼接 |
| 07 Skill | P3：正文/资源；P7：剩余执行模式（R1/R2） | 包内独有 Skill 被按锁装载；同名不串用；脚本/fork/hook 的执行与取消、权限行为有测试 |
| 08 MCP | P7（R2） | Server 连接与 mapping 通过同一能力/资源路径使用，无双重注册，断连和释放可验证 |
| 09 ResourceProvider | P7（R2） | 自定义 kind、多个命名 binding、列举/获取/释放与撤销闭环，资源实现可更换 |
| 10 发布型 Service 资源（原 M1） | P7（R2） | 带资源 Service 真正接收并消费授权 handle，禁用/撤销生效，不再仅支持无资源子集 |
| 11 Browser / Computer 等 Role | P2：通用绑定；P7：领域消费者（R1 基础/R2 完成） | 分别验证用户 JS Provider 执行真实浏览器/桌面动作并接替内置 owner，权限/状态生命周期明确 |
| 12 Turn Middleware | P7：阶段接口；P8：引擎消费（R2） | §16.4 声明的阶段均有真实消费点；有序 patch、失败、取消、重入得到验证，不是任意可变引用回调 |
| 13 EventSource / EventConsumer | P7（R2） | 用户事件源与消费者都能工作；订阅、背压、取消和敏感字段过滤可验证 |
| 14 Lifecycle / BackgroundService | P7（R2） | 安装/Session 作用域 start/stop/dispose、健康与恢复真实接通，不遗留子进程或租约 |
| 15 Transport / Channel | P11（R3） | 替代 Channel 收发、附件与身份映射进入同一 Session API；停用后不重复消费消息 |
| 16 Scheduler / Cron / Automation | P11（R3） | 触发器、调度策略与任务执行器有明确接口；替代实现的重入、取消、重启恢复有真实验证 |
| 17 模型 Provider / 路由 | P8（R2） | JS 模型实现参与真实会话的流式文本/工具调用、取消与错误路径；路由只选锁定候选，不硬编码内置模型 |
| 18 上下文选择 / 历史 / 压缩 | P8（R2） | 用户管线与压缩器改变请求上下文；历史视图与持久化事实分离，预算与恢复行为清楚 |
| 19 长期记忆 / RAG / embedding | P7：资源；P8：策略（R2） | 记忆写入/读取、检索、embedding 实现可分别绑定；索引资源、来源与删除有验证，不只是新增一个搜索 Tool |
| 20 Planner / 推理循环 / 停止策略 | P8：Nomi 组件；P9：整个循环（R2） | 在 Nomi 内可替换规划/停止策略；替代引擎另走 Runtime 契约；两者均可取消，预算控制不失效 |
| 21 工具编排 / 并行 / 结果转换 | P8（R2） | 自定义编排改变真实调度与结果消费；副作用工具不因不明结果被隐式重试，插件不绕过授权 |
| 22 子 Agent / 多 Agent 协作 | P8：委派/协调；P9：跨 Runtime 联调（R2） | 用户协调策略实际创建/管理子任务，授权不扩大，预算/取消传播和结果归属清楚 |
| 23 整个 Agent Runtime / Loop | P9（R2） | 一个真实 JS 外部 Runtime 可替代 Nomi，统一发送/事件/取消；无暗中回到 Nomi；恢复能力显式声明 |
| 24 日志 / tracing / 评估 / 计量 | P7：事件出口；P8：评估消费（R2） | 用户 exporter、evaluator、meter 能消费授权数据；慢/失败观察者不拖住 turn，非权威观察结果不能改写安全审计 |
| 25 UI / Agent 页面 / Shell | P4：Agent 页；P10：其余 UI；P11：桌面服务（R1/R2/R3） | Agent 页及 Shell 真正替换；主题、工具卡片/附件呈现、已定义区域可独立贡献/替换；恢复入口和公共 API 功能对等 |
| 26 Session / 事件 / checkpoint 存储 | P11（R3） | 分别按一致性要求验证替代后端，含事务/顺序/恢复与维护窗口迁移；不要求不同引擎 checkpoint 格式相同 |
| 27 Preset 编译策略 / Registry 后端 | P2：统一绑定；P8：选择策略；P11：目录存储（R1/R2/R3） | 插件可提出组装/选择方案并经过同一最终校验；Registry 存储可换，不引入第二个 Catalog 权威 |
| 28 认证 / 授权策略 / Secret provider | P11（R3，部署者选择） | 三类接口分别验证第二实现与启动配置；失效行为明确，插件不能自行审批授权或读取他人密钥 |
| 29 supervisor / sandbox / Kernel 宿主 | P11/P12（R3，部署级且有例外） | 可换监督/隔离后端在已选平台有实际测试；整 Kernel 替换另属宿主实现，不计作普通 JS 插件完成；强沙箱产品化另估 |

逐项可实施性结论：**01～25 的业务边界有 JS/UI 插件化路径，但不代表其每一种实现都应使用 JS。** 按用户最新要求，§27.13 将天然适配项与依赖原生技术的实现分开；不为完成矩阵强行添加代理或第二执行器。17～23 的模型流、历史/压缩、记忆、规划、编排、协作和完整 Runtime，须先验证性能、取消和状态归属，不适合的具体实现延期。26～29 拆分策略与权威/存储/引导实现：27 的选择策略可走普通插件，目录存储按部署处理；28 的策略与 Secret 后端可有替代实现，但最终授权不能由被授权插件自行决定；29 的原生监督/隔离后端不列为本期 JS 实现，整个 Kernel 也不计作普通插件功能。延期不等于永久封闭，更不等于已完成。

相较旧工作包，本轮明确补入：01/05 的发现与工具暴露策略、02 的用户契约/模板、11 的 Browser/Computer 消费、19/20/22 的记忆/规划/协作、24 的观测评估、25 的 UI 子范围、27 的选择策略与目录存储。不是新增用户需求，而是此前计划粒度不足。它们不能继续藏在“等策略”三个字里。

P7/P8/P10/P11 启动前应按上表拆成有 owner 的小任务；共用 DTO、事件、资源与测试设施，避免一行一个新框架。内置默认组件也必须经过对应逻辑接口；为性能保留进程内 adapter 可以，绕过选择规则的内置特例不可以。

### 27.3 暂不新增 Rust 插件：工作怎么改，而不是目标怎么缩水

本节保留原全量 JS-only 包映射用于追踪；**本期执行以用户后续确认的 §27.13 适配性筛选为准**。保留一个主题不等于必须立即以 JS 实现该主题下的原生计算、设备、存储或监督后端；明确延期不视为范围遗漏，但不得计作完成。

本情景只保留现有 **Rust 宿主/内置 adapter + JS 插件后端 + UI Surface**。不新建 native-process 后端、Rust 作者 SDK、原生插件制品目标管理或 Wasm 后端。

| 工作包 | JS-only 安排 |
|---|---|
| P0 | 保留基线确认、真实替换部件、UI API 验证；用内置 Rust/JS 同契约测试代替 native 小样，不要求 Cargo 示例 |
| P1/P2 | 全部保留；统一 Catalog/绑定与语言无关 DTO 本来就是 JS 替换 Rust 内置组件所需，不是为 Rust 插件预建平台 |
| P3/P4 | 全部保留；Skill、Context、完整 Agent UI 与 Rust 插件无依赖 |
| P5 | 移出本情景范围；不以 stub、空 crate 或未使用入口形式偷偷保留 |
| P6 | 依赖改为 P3/P4；验证内置/JS 与现有 Service 路径，不等待 native 安装、升级或发布 |
| P7/P8 | 按 §27.2 开放 JS 消费接口；Rust 内部 trait 需在宿主适配为结构化异步接口，不把内部对象直接跨进程暴露 |
| P9 | 保留完整 Runtime 替换，验收第二个实际 JS Runtime；不是“JS 只能写 Tool，所以本包取消” |
| P10/P11 | UI 与部署服务拆分保留；可被 JS 实现的服务通过公开 port 接入，必须早于插件加载的部分按 §27.4 处理 |
| P12 | 仍验证所选 OS 上 Rust 宿主、Node/JS、UI 和 Runtime；删除新增 Rust 插件的目标制品/SDK 验证，不误删宿主桌面签名与发布工作 |

首期以 `P0/P1 公共基线 → P2 首条替换闭环 → P6 联合验收` 为集成主线，P3/P4 按各自所需接口接入 P6；**不要求整个 P2 或 P7 完成后才启动 Skill/UI**。P3 依赖精确制品/Session descriptor 和相应执行授权接口，P4 依赖公共应用 API、UI 绑定和事件恢复接口；这些最小接缝稳定即可分别推进。已验证的装载与绑定子项只保留回归。无 P5 前置；R2/R3 不得因“以后可能加 Rust”而等待原生后端。

优雅性的关键不是把所有调用变成 JSON-RPC，而是提取**有业务意义的粗粒度边界**：模型请求及有界流、一次上下文构造、一个资源租约、一轮调度决策。公共类型尽量从已有 canonical contracts 生成；一套错误/取消语义；不跨边界传 Rust 引用、不持引擎大锁等待 JS、不逐 token 同步往返、不另建 JS 专用组装器。

JS-only 的实际损失：纯 Rust/native 库不能作为新类型插件直接打包安装，某些高频计算或设备集成可能不适合该执行形态。可以按实际需要使用现有公开服务或宿主受控代理，但新增代理仍要计入工作量；不能临时允许插件任意启动二进制，变相绕过被延期的 native 制品与生命周期设计。确认有无法接受的功能/性能差距后，再单独评估 P5。

对于“以后是否还是必须补 Rust 插件”，决策依据应是具体缺口：若缺的是系统没有开放模型、页面或资源接口，增加 Rust 后端不会替代这项拆分；若公共接口已经成立，但实际测量表明 JS 实现受原生库、吞吐或设备访问限制，再评估另一执行后端。它届时应与 JS Host 同属执行适配层，共用 Catalog、Compiler、授权和 Session 协议，不与整个平台或 Agent Loop 平行再建一套体系。本期不为这种可能性加入空框架。

### 27.4 “任意部件”的边界必须诚实，不把部署替换冒充普通插件替换

区分三个层级，产品能力清单必须标明所属层级：

1. **普通可安装 JS/UI 插件**：Tool、Context、Skill、资源、middleware、模型、规划/记忆/编排、整个 Runtime、Agent 页面和 Shell。目标是用户安装、选择后直接使用；不要求用户重编译宿主，也不要求改官方源码才能替换。
2. **部署者选择的服务实现**：Channel、Scheduler、存储、认证/Secret、监督/隔离后端等。其中一般业务服务可做成安装级 JS 服务；涉及启动顺序、持久化权威或权限根的服务需在启动/维护窗口选择，由独立 bootstrap 加载和监管。接口应公开，但是否可通过同一插件安装入口交付，要在 P11 分项证明，不能一概宣称普通插件已经可换。
3. **最小 bootstrap 与最终权限/状态仲裁本身**：约束插件的最后一层，不能由被约束插件自行取消。替换它通常是另一个宿主实现/发行版。公开协议可以支持这种可移植性，但这不等于当前宿主内的插件功能，也不在 P11/P12 粗估中承诺另写一套完整 Kernel。

这些区别不是把存储、认证等永远写死的理由。P11 应拆开“策略实现”和“最终执行/提交”：插件可提供认证、授权策略或候选计划，宿主按部署者配置调用，最终结果仍由唯一权威落实。早期启动服务不能依赖尚未装载的 Catalog/Session，避免“必须先读插件数据库，才能启动插件数据库实现”的循环依赖；启动配置只选实现和引导凭据，不发展成第二套运行期目录。

因此，如果目标严格解释成“每一行都能以普通 JS 插件安装，甚至替换约束自己的信任根”，**目前方案不能声称全部满足**；增加 Rust 插件同样不能消除这个安全与所有权矛盾。部署开放与普通插件开放应分别验收，不以“可以 fork 项目”交差。也不默认承诺会话中途热换引擎、存储或设备 owner。

为避免“可替换”在实现时变成两种承诺，使用以下生效规则。它们是目标验收语义，不代表当前已有热切换功能：

| 用户改变的对象 | 选择何时生效 | 必须保留的使用体验与稳定性 |
|---|---|---|
| Tool/模型/Prompt/规划等 Agent 组件或整个 Runtime | 保存成新 Revision，后续会话使用新计划；现有会话保留原锁定实现 | 一次选择与保存即可；无需理解内部依赖。撤销仍立即影响执行资格，不能以冻结为由继续使用已撤销实现 |
| 组件在每轮作出的策略决定 | 已选定的策略按每轮/每请求输入运行；例如模型路由只在锁定且获授权的候选中决定 | 冻结的是实现及允许范围，不是把所有运行时决策写死；不借动态策略偷偷换版本或扩大授权 |
| Agent 页面、区域或 Shell 视图 | 由 UI 消费者管理视图绑定/重载，独立于 Agent 执行计划 | 可连接同一正在运行的 Session；恢复只查询/续订，不重放 turn，不暗中更换引擎 |
| 有状态资源 owner、存储、认证或监督后端 | 按租约结束、服务重启或维护窗口切换；具体策略由该域契约声明 | 停机/迁移要求在选择前可见；不承诺状态会自动转移，更不让普通插件自行替换其权限仲裁者 |

这不是要求用户逐阶段审批。默认流程仍是安装、选择、保存并使用；只有权限差异、现存状态迁移或需要重启时，才说明额外动作。把所有类型塞进“统一热替换”会制造不必要的状态迁移框架，反而背离简单性目标。

### 27.5 简单性也是验收项，不是只有功能清单

组装体验应是：**安装插件 → 选择实现/使用模板 → 保存并使用**。已有授权可继承时不逐阶段确认；只有新增权限或确有风险的操作才需要额外确认。Preview 是解释与诊断能力，不是用户每次必须手工执行的步骤。

以下条件共同约束架构，避免为开放性叠加框架：

- **一套事实**：一个 Catalog、一份 Role/Provider 绑定解析结果、一份精确制品/资源执行计划；UI、conversation、Runtime 不各自猜测默认实现。资源 lease 等运行状态仍有各自明确 owner，不强塞入静态快照。
- **一套用户选择语义**：Agent override > 安装默认 > 官方默认；集合按用户可见顺序；错误可追溯到具体契约/实现。不同语言、UI、资源不再创建专用配置链。
- **旧硬编码退出**：已开放部件的生产消费者只依赖契约，不直接调用内置 ID；内置和用户实现都能用同一消费测试验证。不能只把新路径加上而永久保留两条生产主链。
- **编译与运行各司其职**：保存/创建时解析并锁定；运行时消费计划。P8/P9 在 descriptor 完整、等价校验覆盖后退出重复求解，但保留当前撤销状态、权限、资源有效性检查；“少一层”不等于跳过安全校验。
- **一个 Session 执行 owner**：外部 Runtime 不同时被 Nomi Loop 二次调度。共享公共状态/API，不强迫不同引擎实现 Nomi 的内部 hooks。
- **不强迫作者走长链**：独立 Tool 不必先写 Role；默认由 manifest/SDK 生成重复样板；只为有替换需求的组件制定契约，不让每个 helper function 都插件化。
- **能力不被静默忽略**：所选 Runtime 必须消费已声明支持的贡献；不支持的 middleware/Skill 模式在组装时明确提示。不能“装得上、选得上、运行没效果”。
- **故障与性能可度量**：每个异步边界验证超时/取消/清理，记录启动、首响应、IPC 次数/负载和内存相对基线；按 P0 实测设可接受阈值，不凭空保证零开销。

P6 验收首条闭环；P8/P9 验收执行计划与引擎切换；P10/P11 验收新增消费者未复制组装和授权逻辑。删除有重复事实的链路比单纯减少函数/模块数量更重要。

针对“不希望设计各种约束导致不好用”，实施时区分下面两类，不能把当前实现限制一律解释为稳定性要求：

| 应消除的实现限制或操作负担 | 仍需保留的最小边界 / 用户体验 |
|---|---|
| 要求用户覆盖系统 ID 才能替换，或只能使用官方定义的契约 | 独立身份与契约兼容；用户选实现，系统自动解析绑定；普通独立 Tool 无需先定义 Role |
| 要求替代实现内部依赖与内置完全相同 | 对外行为/输入输出契约一致，私有依赖可不同；系统自动解析依赖，新增权限一次说明，不让用户手工把依赖全加为 Tool |
| 每保存一次都要手工 Preview、填写 digest/Mount、逐层审批 | 一个保存入口自动校验并冻结；仅新增授权或真实风险需要确认；缺件与冲突集中解释，不逐个试错 |
| 为替换一个策略必须重写整个 Agent，为改页面必须复制后端 | 默认 Nomi 的有意义组件可单独替换；完整 Runtime/UI 也可独立替换，共用公开应用协议，不直接跨边界暴露宿主可变对象 |
| 所有变更都全局重启，或反过来要求所有部件都能无状态热换 | 按 §27.4 的生命周期生效：视图可重载，Agent 绑定默认影响新 Revision/Session，有状态服务按迁移/维护边界切换；界面说明生效范围 |
| 失败时静默换回内置、安装成功却运行时忽略贡献 | 保存前说明当前 Runtime 不支持的贡献；运行失败可定位到插件并恢复操作入口，不能偷偷更换业务实现或重放副作用 |

这些是原工作包的设计与验收要求，不新增用户审批流程，也不新增独立“插件规则引擎”。兼容性、来源、权限、生命周期和取消是跨实现协作所需的边界；重复配置、重复求解、硬编码消费者和无必要的内部一致性要求才是应持续删除的负担。

### 27.6 工作量口径：哪些数字还可用，哪些必须重估

| JS-only 范围 | 基础工程量参考 | 当前可如何使用 |
|---|---|---|
| R1-JS | 14～24 人周；含 20%～30% 余量约 17～32 人周 | 范围仍按 §23.2 的首条纵向闭环；P0/P2 后校准，不新增“所有环节”验收 |
| 到 R2 的原主干累计 | 33～58 人周（14～24 + 19～34） | 旧工作包算术参考；不是本轮 29 行细化后的全量承诺 |
| 到 R3 的原主干累计 | 45～79 人周（再加 12～21） | 同样只是扣除 P5 后的低置信度主干参考，不含另写 Kernel/强沙箱产品化 |

这里只保守扣除 P5 的 3～5 人周。P0/P6/P12 确有部分 native 专属工作不再执行，但有其他集成工作保留，在拆项前不再乐观扣减，避免重复计算收益。

按两名熟悉 Rust/运行集成的后端加一名前端，R1-JS 可先按 **约 10～16 周日历**安排预算窗口，受 P1/P2 关键路径约束，不是人周直接除以三；若当前并行统一工程已验收，按实际剩余量扣除。已有代码改动数量不代表 P1 完成。

截至 2026-09-14，`439bb385a` 已合入 Plugin 统一工程，工作区已有初始 Context、共享协作取消、通用 JS Role 映射/分发、候选目录/Agent 选择、跨包恢复和产品默认管理切片；因此上述数字是**原定范围的预算参考，不是从当前代码起算的剩余工期**。下一轮排期应先复核 P1 的消费者与旧分支退出情况，从 P2/P3 扣除实施台账中已验证的对应子项；递归依赖按 §2.15 的已验证范围扣除，Provider 全量领域替换/升级影响、其余依赖调用类型、完整 Prompt、Skill 剩余模式、插件 UI 等不得跟随扣除。§27.9 的合同演进和 §27.10 的验收归入原有工作包，不另造重复计费项目，但拆卡后必须重估原包是否足额。

需要修正此前整体估算的解读：**“不加 Rust 后主干约 45～79 人周”不等于“原始所有环节完整满足只需 45～79 人周”。** §27.2 补明确的项目此前未逐项定量，可能使 P7/P8/P10/P11 超出原区间；20%～30% 风险余量也不能拿来吞掉明确的范围缺项。全量开发日历在这些包拆卡前只能称中长期项目，不能继续把粗略半年到一年当成全目标保证。

P0 建立 29 行覆盖台账，P2 固定公共边界，R1 后逐领域拆包重估；每项记录“已实现并验证 / 部分完成 / 未开始 / 部署级 / 明确例外”及证据。没有范围改变，就不得默默删掉难做的环节；确要减少范围则单列产品决策。

从当前代码安排开发时，任务卡应分开记账：已验证的切片只计后续集成/回归；在途改动计剩余验证、合同同步与产品接线。不同资源需求的投影/定向消费已按台账 §2.7 核销，包内 Skill 只读装载及显式命令/补全按 §2.8～2.9 核销；产品任意资源解析/生命周期、Skill 剩余来源与执行模式、UI、深层策略等不能跟随核销。不能用“29 行中有几行有代码”计算完成百分比，也不能把原预算减去代码行数估算剩余工期。R1 的预算承诺只对应首发结果；五项完整目标的预算必须包含 R2/R3 及明确列出的例外，不把它们藏进首发余量。

### 27.7 开发决策建议

当前建议是：**先不新增 Rust 插件后端；先交付 R1-JS，同时把完整开放目标保留在逐项台账中。** P0/P1 先核销已有统一工程与证据，不从头重做；按 §27.11 安排 P2/P7 的领域消费与依赖、P3/P7 的 Skill 剩余工作、P4 的 UI/公共 API，再以 P6 验收首发支持范围。首发所需公共接口稳定后，Skill 和 UI 可并行，不等待整个 P7 完成；不把原生 SDK 当成开放性的前提。

后续用 P7/P8/P9/P10 完成 Agent 业务层与 UI，P11/P12 逐个完成部署级服务和已选平台。不以只发布接口、单个示范插件或存在 Rust trait 判定全部完成。是否补 Rust 插件，应由实际无法满足的原生依赖/性能需求驱动，而不是为了让架构图看起来对称。

对本次问题的最终回答是：**五项需求都有相应设计路径；首期只部分解决；旧计划的若干后续环节尚未细化，本节已补入任务和验收，但仍需重新估算。普通插件不能替换最小信任根的例外也必须明说，不能拿部署/重编译替换冒充插件替换。**

### 27.8 当前代码复核：不能把已有切片扩大解释为需求完成

本节基线为 `439bb385a` 加工作区已有改动；不改写 §1～13 的历史事实。初次文档复核仅做静态检查和既有记录核对；后续实施已补齐下表 Tool/Context 共享协作取消，并运行 Node Host 定向/集成验证，详见实施台账 §2.1。外部模型与桌面交互不在该切片验证范围内。

**前次复核记录：递归依赖改造已有跨层实现、核心/消费者回归与合同同步证据。** 下表替换当时已过时的“旧负例未修复、schema/规范未同步”判断；其中“本轮”指该次递归图复核，不表示最新实施又运行了这些检查。前次追问仅做代码/记录核对与文档更新，最新 Skill 实施证据单列如下，不能用早先绿灯覆盖之后新增的测试。

**最新 Skill 实施：冷启动发现及附图/装饰输入。** 冷启动产品夹具的 E0599 与后续必填字段缺失已修复，改用正式 DTO 后产品目录/冷启动 2 项通过，原 app 插件相关 30 项通过；UI 6 项交互、typecheck 与桌面边界通过，见台账 §2.16。随后修复引擎的单文本命令限制与知识前缀遮蔽，沿原宿主 turn 接缝传递用户原文，Nomi unit 633 / Bootstrap 26、后端 ai-agent unit 515 / consumer 28 项通过，产品目录/冷启动 2 项在最终代码上再次通过，见 §2.17。原 app 30 项记录属于引擎输入改动前；不把这些测试当成完整 Skill 执行模式、全部 UI 或 29 行目标完成。

**后续 Context 依赖消费证据。** 台账 §2.18 的留存结果为 Kernel 定向 19、JS adapter 定向 4、Host 定向 9、产品 runtime wrapper 6，以及后端 ai-agent unit 515 / consumer 29 项通过。Nomi 新用例验证初始/动态 Context 使用私有依赖结果、同 Session 重建后求值身份不混用；真实 Node 分发另由 adapter/Host 用例验证，不能拼称完整产品 UI/远程模型端到端验收。本次只核对这些记录；新增作者 SDK 类型的 authoring 测试及最新 Context 改动后的更广 Kernel/Host/adapter 回归仍未取得本次通过记录。

**UI 命令接缝：产品命令已验证，完整页面仍未收口。** 静态核对 [Surface 应用](../../crates/backend/nomifun-plugin-platform/src/runtime/m1_application.rs)、[Session 适配器](../../crates/backend/nomifun-app/src/router/plugin_ui_sessions.rs) 和 [产品 SDK](../../crates/backend/nomifun-plugin-platform/src/assets/product-sdk.js)，工作区已有以下改动：宿主明确授予某个既有 Session 的访问范围，绑定所选 UI release digest；Bridge 从 Surface 授权记录获取会话身份，而不是接受 iframe 自报 owner/Session；`observe/turn/cancel` 复用现有 Session owner，发送复用幂等入口。内置 HTTP 与插件路径共享会话身份校验，结构化 Session 错误保留；异步请求返回时再检查 Surface，撤销后不交付私有成功结果或错误。准确证据见台账 §2.19，不再一概描述为从未验证，也不升级为页面完成。

本次复核的结果及其含义：

- `node --test crates/backend/nomifun-plugin-platform/tests/product_sdk.test.mjs`：3 项通过，执行实际 SDK 与原生 MessagePort，覆盖命令/错误、超时不重试与克隆失败清理；不是浏览器页面替换验收。
- `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1`：1 项失败。模型 Mock #0 收到 2 次请求、Mock #1 收到 0 次；静态核对两者同为 POST/同路径匹配，新增的延迟取消场景未命中。失败点是测试夹具验证，不能据此断言生产取消有缺陷，也不能因为前面断言走完就计作运行中取消已通过。
- `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`：失败，首个报告为 `contracts/runtime/runtime-release-fixture.json` canonical payload drift；应按当前源合同生成、审查并再次检查，本轮不代为修改生成文件。之前图切片的 `check` 通过是历史证据，不能覆盖本次新增 UI 合同。
- 留存的 app 插件单元 32 项、DB 定向 1 项、contracts/API 类型 41/28 项通过，包含撤销后隐去私有结果/错误、会话归属及删除撤销等范围。app 命令的 `router::plugin_` 过滤同时过滤掉了产品 integration，后者实际运行 0 项；不可将该绿灯算作最新版产品测试通过。早期产品 1 项通过也不覆盖后来新增的运行中取消场景。

后续实施核销：模型夹具现按请求中最后一条用户消息匹配，避免第二轮历史中的首轮文本误命中；取消前新增 `head.status == running` 断言。最新版无过滤产品测试 1 项通过；canonical `write` 后 `check` 通过。上面的两项失败保留为历史诊断，不再作为当前未修复项。命令层取得产品证据不等于下列页面/恢复缺口已解决，细节见台账 §2.19。

P4 仍有三个不能被该桥接抵扣的缺口：

- **流与恢复**：[公开 Session events 处理器](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs) 仍返回 `unsupported_session_events`；`build_session_observation` 返回持久化消息投影且 `events` 为空。消息分页游标不是 token 流重放游标；查询历史不能替代实时流、断线续订和缺口处理。现有 Conversation 流式设施也不自动等于该插件 API 已闭环。
- **实际页面选择**：[Router](../../ui/src/renderer/components/layout/Router.tsx) 仍固定装配 `AgentSessionPage`；尚未有同一 Catalog 中的 UI 贡献绑定接管这个消费者。插件库里能打开一个 Surface，不等于用户已替换系统 Agent 页面；Shell 仍属 P10。
- **视图生命周期与验证**：[Surface 表](../../crates/backend/nomifun-db/migrations/080_plugin_surface_sessions.sql) 仍对 `plugin_product_id` 唯一，一个插件重开视图会替换原 Surface。多会话/多窗口支持需明确设计和验证，不能默认已支持。SDK/解析器、部分授权撤销及会话删除已有定向证据，命令合同和产品测试后续已核销；升级及真实页面恢复仍须收口，不能用这些局部通过替代。

该在途改动直接调整了 `080` 的 fresh baseline，不能据此声称旧数据集已兼容或已完成迁移。本次只读核查与文档更新未启动产品、重置或迁移用户数据；后续实施需以隔离数据集验证，并明确真实数据集的切换策略。它属于原 P4 的接缝建设，不另立第二个 Session 管理器或插件授权平台，也不改变 29 行的未完成范围。

| 当前变更 / 代码位置 | 本轮确认的实现状态 | 尚缺的闭环与下一批影响 |
|---|---|---|
| `preset.rs`、`compiler_dependencies.rs` 及 `ResolvedCapability` 构造点 | 同一计划记录用途/精确边，按所选实现递归解析并校验图；外部 PluginProduct 投影不能自报内部字段；schema/摘要/夹具已生成且 `check` 通过 | 保留生成一致性回归；当前 UI 无需另一份手写图类型，不因“同步 TS”新建平行计划；历史图语义迁移另列 |
| `compiler.rs::require_contribution`、`registry.rs`、`dependency_call.rs` | 公开 Tool、direct/Role Context 先检查用途；子调用核对父实现声明和冻结边后走原授权链；Agent Context 发起调用的后续增量见台账 §2.18；内部 Context 仍被公开入口拒绝 | 非 Agent/资源工厂/后台任务发起受管子调用仍未开放；不能给隐式资源工厂一律加公开用途限制而破坏合法内部执行 |
| `plugin_tools.rs` 的 Tool、Context、生命周期消费者 | 六处遍历用 `contributions()`；内部 Context/Tool 不进入初始/动态 Prompt 或模型 actions 的回归已通过；Context 顺序和 MCP 锁只投影公开贡献 | Service 的扩展还需真实消费；平台 owner-scoped Catalog 是执行事实，不是模型工具清单，保留完整 authority/active 范围，不新增激活状态机 |
| `role_providers_unchanged` 与旧 Snapshot | 从 Revision 公开选择重算所选图，比较 Role 锁、用途、边与完整能力/资源需求；过时图拒绝复用有 Kernel 测试记录 | 产品正常 save/open、旧格式及历史锁定的新测试单独记录于下表；缺字段默认值只保留序列化，不构成图迁移或旧会话可继续的保证 |
| `materialize.rs`、adapter 测试、正式 05 第二部分 §5.5 | `requires` 差异已允许；删除过时的依赖相等负例，保留 schema/effect/resource 不兼容负例，最新 22 项 adapter 回归通过；正式规范已写入选中图与用途合同 | 不恢复内部依赖完全相等限制；保持对外契约检查。通用 Tool/Context 切片不代表真实 Browser/Computer 等所有领域已替换 |

本批已有证据的关键场景包括：所选 Provider 引入的循环/缺失精确版本；依赖本身又有 Role 默认/override；显式选择兼内部依赖保留公开用途；未选候选不进入图；内部 Tool/Context 不对外暴露；来源漂移或权限撤销后子调用被拒绝。上述接线与通过用例不再列为待开发。隐式资源工厂的依赖也纳入实际需求，但不得因此变成额外公开 Tool 或制造自依赖；其新增集成用例与产品用例单独记录，不用旧结果代替。以上均属于 P2/P7，不新建第二套 Snapshot、Registry 或授权状态机。

| 递归图切片的检查 / 日志 | 结果与时点 | 证据边界 |
|---|---|---|
| `.tmp-open-graph-core-final.out`（留存回归） | contracts 93、Control Plane 41、Kernel 47 项通过 | 包含递归 Role 默认/override 与 MCP 投影调整；替代早先 Kernel 46 项记录，不代表全工作区发布验证 |
| `.tmp-open-graph-consumers-final.out`（留存回归） | Nomi 27、adapter 21 项通过 | 含真实 JS 多跳、内部 Context 公开拒绝且无 Node 激活、Nomi 不消费内部 Tool/Context；早先 adapter 19/1 失败已修复。之后新增隐式资源测试不在此计数内 |
| `target/debug/agent-v2-contract.exe check`（本轮执行）及正式规范核对 | 退出码 0；生成 schema 已含用途/边，05 第二部分 §5.5 同步 | 未新增 Node RPC；工作区合同一致不等于已发布，也不证明旧图语义已迁移 |
| `cargo test -p nomifun-app --lib router::plugin_ --no-default-features -- --test-threads=2`（本轮执行，`.tmp-open-graph-app-fixed.out`） | 30 项通过，含新增异构 Provider 安装/保存/打开/切换/撤下测试 | 正常安装流程、SQLite 控制面、生产 Nomi Snapshot 准入与 materialize、Kernel 真实 Node 调用；仅一个公开 Tool，私有 Context 不消费。切换后旧计划仍用旧实现，撤下不回退；不是外部模型或 UI 验收 |
| `cargo test -p nomifun-js-kernel-adapter --test kernel_adapter --no-default-features -- --test-threads=2`（本轮执行，`.tmp-open-graph-adapter-final.out`） | 22 项通过，覆盖之前新增的隐式资源工厂依赖用例 | 缺 feature 拒绝、依赖锁入内部图但不成为公开贡献、真实 JS 资源消费/释放；不等于产品自定义 kind、多 binding 或完整租约竞态验收 |

新增产品用例还验证了：构造的旧格式 envelope 结构合法，但与重编译结果不一致时产品 open 明确拒绝，持久化 Snapshot 不被修改。这证明“不静默解释为新图”，**不证明已有历史会话全部可继续使用**。此前 app 首次编译的 `CapabilityInvocationRequest` 导入错误已由既有修正消除，本轮在修正后的代码上取得上述结果；测试夹具仍是受控包，不冒充真实 Browser/Computer 领域完成。

这些日志是本工作区的诊断记录，不是可分发的验证制品。后续核销应在实施台账记录准确命令、候选代码状态和结果；不能仅引用未纳入版本库的 `.tmp-*` 文件证明一个版本已完成。

| 原始需求 / 当前接缝 | 当前证据 | 可成立的判断与仍缺的部分 |
|---|---|---|
| 1：用户 capability 与内置 Provider 替换 | [`package.rs`](../../crates/backend/nomifun-agent-contracts/src/package.rs) 的 `implementation` 与 N1 准入已接线；[`role.rs`](../../crates/backend/nomifun-js-kernel-adapter/src/role.rs) 验证冻结 Provider 后复用 Tool/Context/Resource 执行。已有 Catalog/Agent 选择、产品安装默认管理、Provider-aware 保存及跨包恢复切片；真实 JS 安装后经 save/read/生产 Snapshot 接缝进入 Nomi Context，详见台账 §2.2～2.6 | 通用替换与默认管理基础已有证据，不代表需求完成：Browser/Computer 等实际领域消费者、升级影响清单、异构依赖/授权、全局故障诊断和资源竞态仍需继续。默认管理与恢复不再列为全未做，也不由这些切片推断完整领域已通过；依赖相等限制按 §14.5 演进 |
| 1/3/5：不同实现的资源与特性需求 | [`materialize.rs`](../../crates/backend/nomifun-agent-kernel/src/materialize.rs) 区分私有资源与对外资源类型；[`compiler.rs`](../../crates/backend/nomifun-agent-kernel/src/compiler.rs) 的 `apply_role_requirements` 投影所选需求；[`requirements.rs`](../../crates/backend/nomifun-js-kernel-adapter/tests/kernel_adapter/requirements.rs) 有真实 Node 资源消费回归 | **部分已验证**：最新 helper/保存校验已复跑并同步 05 规范；不同资源需求与过时投影重新保存有证据。不再描述为资源一律相等，也不记为任意异构依赖、产品自定义 kind、多 binding 或资源生命周期已完成 |
| 1/5：实现独立冲突声明 | [`conflicts.rs`](../../crates/backend/nomifun-js-kernel-adapter/tests/kernel_adapter/conflicts.rs) 验证差异准入、真实 JS 消费、内部目标及隐式资源；Control Plane 与产品安装测试验证正常保存的冲突拒绝和合法消费，详见台账 §2.12 | `conflicts` 不再要求相等；按所选公开贡献/实现/资源工厂检查，默认不是强制同时消费，未选候选不影响组装，内部集合不变成权限。此切片不关闭 `requires` 差异或受管子调用 |
| 1/3/5：父调用作用域的依赖调用 | [`dependency_call.rs`](../../crates/backend/nomifun-agent-kernel/src/dependency_call.rs) 复用 Kernel 与冻结 Snapshot；[`extension-host.mjs`](../../crates/backend/nomifun-js-host/assets/extension-host.mjs) 已有 Tool/Agent Context SDK 接线；[`plugin_runtime_host.rs`](../../crates/backend/nomifun-app/src/router/plugin_runtime_host.rs) 复用父级租约 | **JS 同包/跨包调用和产品接缝已有定向验证**；递归图/消费用途与合同同步见台账 §2.13～2.15，Context 新增证据见 §2.18。Host 默认高并发启动风险保留；非 Agent/资源工厂/后台调用仍未开放该 API |
| 2：系统 UI | [`Router.tsx`](../../ui/src/renderer/components/layout/Router.tsx) 仍静态组合 Agent 页面；已有 [`PluginRuntimeSurfacePanel`](../../ui/src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.tsx) 是隔离 Surface 容器；会话命令桥接有定向证据，后续修复产品测试和合同漂移通过，见台账 §2.19 | Surface 可展示、命令有产品证据均不等于系统页/Shell 可替换；P4/P10 的流与恢复、同目录选择、内置 UI 同契约消费及视图生命周期仍要完成 |
| 3：Context 切片 | [`assemble_initial_capability_context`](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs) 与 [`plugin_context.rs`](../../crates/backend/nomifun-ai-agent/src/plugin_context.rs) 消费同一冻结 `context_order`；[`AgentContextOrder`](../../ui/src/renderer/pages/agentSettings/AgentContextOrder.tsx) 提交原保存入口；实际包/Node 安装与 Nomi Engine 验证见台账 §2.10～2.11 | 初始/动态 Context 与用户贡献排序已有实现，不是完整 Prompt 替换。顺序在各自声明阶段内生效，不能调整 persona/内置 lifecycle 等全部组件相对位置；两类批次各共享 5 秒/64 KiB，不是整个 Session 或所有 middleware 的统一预算 |
| 3/5：异步稳定性 | [`extension-host.mjs`](../../crates/backend/nomifun-js-host/assets/extension-host.mjs) 的 Tool/Context 已共用 request-scoped AbortController；[`supervisor.rs`](../../crates/backend/nomifun-js-host/src/supervisor.rs) 使用原有请求账本和有界取消通道处理丢弃的等待者 | 共享协作取消已有真实 Node 验证；Rust 超时仍不等于任意 JS 算法已停止，不响应取消时依赖原 deadline 与 generation 清理。总启动预算、资源租约/SDK 子调用及未来流式接口的完整取消仍须分别验证 |
| 4：Skill | [`ResolvedSkillLock`](../../crates/backend/nomifun-agent-contracts/src/preset.rs) 包含来源/制品字段；[`plugin_skills.rs`](../../crates/backend/nomifun-ai-agent/src/plugin_skills.rs) 经产品 Session 获取精确正文/资源与来源守卫；manager 传给 [`AgentBootstrap`](../../crates/agent/nomi-agent/src/bootstrap.rs)、原 Skill 工具与同 descriptor 的显式命令。PluginMount 的旧目录 ID 投影已退出 | **只读切片有证据**：真实包安装/save/read 后读取精确正文/资源、标准工具/Bootstrap、目录同名、撤下与制品变化见台账 §2.8；命令/补全见 §2.9，冷启动产品证据见 §2.16，附图/装饰输入见 §2.17；其余来源/模式、历史保留及迁移不因此完成，也不是外部模型或真实 WebView 验收 |
| 3/5：Runtime 与组装 | [`AgentRuntimeHandle`](../../crates/backend/nomifun-ai-agent/src/runtime_handle.rs) 的生产变体仍是 Nomi，trait-object Mock 是测试接缝；[`AgentPresetRevisionPayload`](../../crates/backend/nomifun-agent-contracts/src/preset.rs) 没有公开 Runtime 选择合同 | 统一 Plugin 主链已有工程基础，但不能因此宣布整个 Runtime 可替换或组装简化完成；P8/P9 还要建立真实生产消费者并退出重复求解/硬编码 |

**递归图改造之前，受管子调用基础的验证状态**：五问复核发现跨包夹具失败后，后续实施已修正并复跑。以下不代表整个脏工作区的发布认证，也不覆盖本节上方递归图新增变更；历史失败和各时点结果完整保留在台账 §2.14。

| 既有日志 / 检查范围 | 记录结果 | 对本次评估的含义 |
|---|---|---|
| `.tmp-open-js-dependencies-core-verified.out`：contracts / Control Plane / Kernel 单元测试 | 分别 91 / 41 / 44 项通过 | 父作用域授权等基础有定向证据，不代表异构依赖编译已完成 |
| `.tmp-open-js-dependencies-adapter-verified.out`、`consumers-fixed.out`：adapter / Nomi | adapter unit 2、integration 20 项通过；Nomi consumer 26 项通过 | 同包/跨包 × direct/Role 的真实 Node 子调用通过；另覆盖父包不变、依赖包来源漂移的拒绝，不调用 builtin。不是仅根准入失败的测试 |
| `.tmp-open-js-dependencies-host-final-bounded.out`、`host-verified.out`：Host 不同并发运行 | 最终两线程 unit 16 / integration 79 项通过；此前默认高并发为 77 通过 / 2 启动或 activation 超时 | 新增六个依赖用例已通过，两个计时用例分离了 Node 启动但未放宽生产 deadline。默认并发敏感性保留，不能宣称任意压力下无 flaky |
| `.tmp-open-js-dependencies-product-final.out`：产品 plugin 路由/运行接缝 | 29 项通过，含 runtime Host 5 项与安装/恢复 7 项 | 含真实 Node 嵌套调用、切换写锁排队、取消后租约释放，以及不同 Host/脱离任务不能借用租约；不是外部模型或跨平台发布验收 |
| `.tmp-open-js-dependencies-authoring.out`、SDK 类型及 canonical `check` | 脚手架 15 项、内存 TypeScript 类型检查、五处 JS 语法与合同一致性检查通过 | SDK 的 caller 字段与 Mount 权限边界有类型验证；实际授权仍由 Kernel/Host 执行，生成产物使用当前 `target/debug` 二进制核对 |

接续开发沿同一 canonical 计划推进剩余领域与资源消费，保留递归图回归和默认高并发启动风险；已验证的图与子调用不用从零建设。上述历史测试修复只调整测试制品/测量边界和补充断言，没有放宽生产 validator、增大生产 timeout 或增加另一套调用账本。后续图改造则确实演进了准入和 Snapshot 合同，其证据独立见本节上表，不混用历史绿灯。

既有验证记录中，产品安装测试有 1 项通过，Agent 投影回归有 19 项通过；[实施台账](2026-09-14-agent-plugin-openness-implementation.zh.md) 另记录消费测试由基线 8 项增至 13 项。后续取消实现新增 4 个真实 Node 场景，Host 集成共 73 项通过。证据各自覆盖对应接缝，不等于 29 环节验收，更不等于模型一定遵循 Context、UI 可替换或 JS Runtime 已接入；详细命令与复跑范围以台账为准。

后续 Provider 实施已运行 contracts 87、Kernel 31、adapter unit 2、真实 Node integration 6 项回归，产品安装测试扩展后 1 项通过，生成合同检查通过。该证据补充先前仅静态复核的状态，详见实施台账 §2.2；没有运行 UI 或跨平台原生验证，也不能由通用 fixture 推断所有系统部件已可替换。

五问静态复核及后续实施补充三个容易误判的证据边界（后续测试范围见实施台账 §2.3～2.4）：

- [`AgentCatalogResponse`](../../crates/backend/nomifun-api-types/src/agent_platform.rs) 原先只有 capabilities、skills、mcp_tools，后续已加入 roles 精确候选投影；[`AgentPresetEditor`](../../ui/src/renderer/pages/agentSettings/AgentPresetEditor.tsx) 与模板界面已共用部件选择器，工作台通过一个完整 Catalog 请求读取。新增 6 项交互测试、控制面投影/路由测试及真实安装后 save/read/Context 消费已验证。候选不是“必然可执行”的标签；安装默认管理与真实 WebView 交互尚不能从这些证据推断完成。后续另以 6 项回归补齐 Provider-aware 无改动保存校验，不用首次保存的证据代替升级场景。
- [`role_implementation_matches`](../../crates/backend/nomifun-agent-kernel/src/materialize.rs) 的 requires 相等检查已移除；内部依赖图、消费隔离、保存复用及生成合同同步有独立证据，见本节上表和台账 §2.15。§14.5 是可替换性需求，不只是性能优化；既不能用冲突/资源差异的历史结果代替图回归，也不能由通用图测试推断全部领域、资源与历史兼容已经交付。
- Skill 原先只有 ID 投影；台账 §2.8 已把包内来源接到实际 Session 和装载器，并测试只读正文/资源，不再将其描述为完全未接通。生产 Runtime handle 仍只有 Nomi（Mock 仅供测试），Skill 剩余模式也仍未完成；不能从 Context/Skill 切片成功推断 Runtime 替换或完整需求 3/4/5 已成立。若 R1 对外承诺包内 Skill 可用，P3 的精确装载与声明的支持模式就是发布前置条件，不是可以默默延期的体验优化。

Skill 增量的合同边界：`ResolvedSkillLock` 新字段改变持久化 Snapshot 合同，已同步生成 schema/摘要、结构验证和 05 §3.3；不增加另一个 Skill Registry。正文及引用资源来自锁定制品，装载后撤下仍受当前来源检查，包内同名、缺文件、摘要不符不回退到目录版本。当前只读入口明确拒绝 shell/fork/hooks/model/tool override 等模式，这是未支持范围的显式报错，不是这些模式已经完成开放。旧格式非空 Skill Snapshot 不自动迁移，历史制品保留也没有本批交付保证；需要保留该类历史会话的数据集须另行完成切换验证，不自动清库。装载及后续命令的测试范围分别以台账 §2.8、§2.9 为准，§2.7 等历史绿灯不替代它们，也不能证明整个工作区可发布。

Provider 无改动保存增量：[`PresetRevisionCompiler`](../../crates/backend/nomifun-agent-control-plane/src/compiler.rs) 现在在复用旧 Snapshot 前调用 Kernel 同一 Role resolver 检查精确锁，而非只比较 façade capability。选中 Provider 的制品/贡献等变化会触发新保存编译；撤下、契约不匹配、缺 member 或平台不支持会明确失败；无关 Provider 变化不造成版本抖动。默认目标变化仅影响新保存结果，显式 override 与旧 Revision/Snapshot 保持不变。Control Plane 35 项、Kernel 31 项回归已通过，包含正常 save/read 的升级和撤下测试；详见实施台账 §2.4。该切片当时仍使用构造时传入的默认环境，实时默认管理和跨包恢复由下述后续切片补充；升级影响清单和所有 Snapshot 输入漂移仍不能宣称已解决。完整五项需求的状态不因此改成完成。

跨包恢复增量：原先启动时先发布空目录，再按 Mount 顺序逐包加入，会漏掉先于契约装载的 Provider。现在启动与安装/启停后的对账都从现有安装 inventory 重建完整候选，由同一 Kernel `replace_all` 校验后一次发布。可定位到用户 Mount 的错误只排除该 Mount 后重新校验整批；不会新建依赖排序器、另存一套注册事实或选择其他 Provider。契约禁用时依赖它的 Provider 从 live Registry/Catalog 撤下，但不删除安装或改变其选择；契约重新启用后自动恢复原精确 Provider，无需用户逐个 retry。安装失败的目标仍向调用方报告对账错误，不因隔离成功就伪装成可用。

本次证据是实际包导入/测试/安装、同一 SQLite pool/data root 上重新组合宿主、公开 Catalog、正常保存和真实 JS/Nomi Context 消费；不是另起产品进程重开磁盘数据库，也不是外部模型验收。Kernel 无法给出安全归属的全局错误（如依赖环）仍明确失败、保留上一代目录；尚未建设逐插件健康诊断 UI。撤下会清理 Kernel 已持有的相关资源，并保留无关 Mount；并发 acquire 完成与撤下的全生命周期竞态仍需资源工作包单独验收。实现及测试明细见台账 §2.5，不能据此关闭完整 P2/P7、资源生命周期或其余 29 环节。

安装默认管理增量（实施台账 §2.6）：实际 Nomi 产品已通过单表持久化与版本 CAS、owner-scoped API 和工作台入口管理 `InstallationRoleBinding`；不是只修改构造时的测试环境。新建/重新保存会从该存储读取当前默认，显式 override 优先，Provider 检查仍由原 Kernel resolver 完成。执行校验只从持久化 Snapshot 的 Provider locks 派生冻结选择，不读取当前默认。真实 JS 包、公开 PUT、SQLite Agent 保存、默认变更和 Nomi 消费测试通过，并验证旧实现撤下不回退；UI 保留失效选择，冲突不自动覆盖。此处补足的是管理/消费基础，不代表独立 Fresh-v4 host、全部内置角色初始化或各非 Agent/部署服务都已接入。详细验证及未完成边界以台账为准，前文各历史切片“默认尚未接入”的状态由本增量更新。

资源需求增量（实施台账 §2.7）：最终 adapter integration 12 项通过，其中 6 个资源需求相关测试覆盖 Tool/Context/非 Agent operation 实际消费参数、缺绑定/错误 owner/内部直接调用拒绝、隐式工厂平台/特性、替换而非并集旧私有需求、保留对外资源类型/序列化目标，以及锁相同但需求投影过时不能复用。控制面另补正常 save/read 回归，验证新版本修复投影、旧版本不改、再次 clean-save 稳定复用；Control Plane 36、Kernel 31、adapter unit 2 项通过，产品接缝 7 项与 Nomi 消费 13 项复跑通过，canonical contract check 通过。05 §5.5 已同步，未增加 wire 字段。此处替代先前 11 项旧日志及“最新 helper 尚未验证”的状态，但仍是受控 fixture 与现有产品接缝证据，不是任意产品资源 kind、多 binding、完整租约竞态或递归依赖已经开放；不关闭 02/09/11 或完整需求 1。

### 27.9 实施前先消除合同冲突，不在旧限制旁边堆一条新通道

“继续开放”需要演进现有合同，而不是绕过它。以下事项纳入现有工作包，由同一个公共合同负责人收口；设计建议和已实施部分分别标明，不能把未实施建议当成现行协议。2026-09-14 已实施的 Role 映射已同步规范与生成合同；后续资源需求切片已将 05 §5.5 的严格相等表述修订为私有需求可不同、对外资源类型保留，并完成定向回归。其余条目仍需各自演进，不以修改评估报告替代正式合同修订。

1. **P2/P7：从独立 capability 到可替换实现。** 现有 Role member 表达契约侧 capability；2026-09-14 已在 canonical contribution 中加入精确实现映射，调整 N1 准入、Kernel/JS 分发与生成 schema，并同步 05 §5.5/06 增量说明；后续已补 Catalog/Agent 选择、安装默认管理、Provider-aware 保存与跨包恢复基础。接下来仍需不同实现需求、升级/关联失效诊断和各领域消费者；普通 Tool 不强制写 Role。不得复制系统 capability ID，也不得另建 override 注册表。只有 façade 内部的 Provider 不应伪装成另一个可独立调用的公共 capability；需要独立消费的用户 capability 保留自己的身份。不同实现的依赖与资源需求按 §14.5 编译和授权，不把“兼容”永久等同于实现清单完全相等；也不把现有 enabled 集合直接当作内部依赖、模型可见性与调用授权的共同开关。
2. **P8/P9：分清 Agent 引擎与执行宿主。** [05 规范 §15.1/§15.3.4](../specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md#1534-明确禁止的字段和能力) 将 Runtime 管理留在基础设施域，并禁止 Revision runtime selector。目标建议把 Agent 算法实现作为平台发布、用户选择、会话锁定的 Runtime Role/Provider；Node 版本、进程监督、安装权限、启动参数仍归平台。要支持逐 Agent 的引擎选择，就需明确修订原禁令的适用范围、锁定/摘要和消费者合同，不能宣称现行规范已经允许，也不能恢复 `preferred_agent_id`、任意启动命令或 fallback engine。若仅做系统级引擎选择而不修订合同，必须标为逐 Agent 定制尚未完成。
3. **P3/P8：声明“追加 Context”与“替换 Prompt 管线”是两种不同消费模式。** 前者组装贡献，后者决定完整模型请求的提示词构成；只有选中的实现执行。Context 内部顺序现已由 typed `context_order` 保存/冻结/消费（台账 §2.11），不是继续按 ID 写死。完整管线的角色、来源与总预算仍须有类型化合同；安全授权在提示词之外执行，不靠偷偷追加内置 Prompt 保证权限。
4. **P4/P10：UI 用同目录、不同消费者和生命周期。** 页面/Shell 绑定不能假扮 LLM Tool，也不能依赖先创建 Agent Session 才能显示。公开 Session API 及应用状态查询由现有业务域持有，插件 UI 只负责呈现。桌面/WebUI 维持仓库 880×600 最小视口合同，不顺带扩展手机/平板产品范围。
5. **P11：逐项定义可安装服务和引导期服务。** Channel/调度等普通业务服务优先复用 JS 安装生命周期；存储、认证、Secret、监督/隔离按各自启动依赖选择部署接口。引导配置只负责装载信任根，不另建运行期 Catalog；不能把“公开了 Rust trait”或“可重编译宿主”计作用户插件替换完成。

这种演进不需要第二套 JS 平台，也不需要先做 Rust 插件后端。重要的是每个公开接口有对应生产消费点；合同、生成类型和消费者在同一工作包中交付，不先堆积无人使用的 SPI。

### 27.10 回答“到底有没有解决”的五组用户验收

§27.2 是完整环节清单；下面以用户操作补充验收，不再创建新工作包。完成一组代表相应场景成立，不能代替 29 行剩余范围。

**下表是待达到的验收要求，不是通过记录。** “首期证据”列表示 R1 发布时须提供的证据；当前已取得的子项证据只以 §27.8/实施台账为准。

| 用户验收 | 首期证据 | 完整目标追加证据 |
|---|---|---|
| 1. 我选自己的实现，系统真的用它 | P2/P6：安装实现同一真实系统契约的 JS 插件，设为默认或 Agent override，真实请求只落到所选实现；保留独立 capability ID | P7/P8/P9/P11：每个开放部件重复验证；插件 A 依赖契约时也能调用用户选定的插件 B；两种实现可有不同内部依赖/资源，但内部贡献不隐式暴露或提权；新默认不漂移老 Session，撤下实现明确失败 |
| 2. 我能换掉系统交互界面 | P4/P6：通过插件 Agent 页创建/打开会话、发送、流式观察、取消、查历史、断线续订 | P10：替代 Shell、页面区域和结果呈现；内置 UI 不再独占业务 API；关闭故障 UI 后可恢复原会话，不重复启动 turn |
| 3. 我能改变 Agent 怎么工作，而不只是增加工具 | P2/P3：真实部件替换及初始 Context；这些只算起点 | P7/P8/P9：在默认 Nomi 中分别替换模型/上下文/压缩/规划/编排/协作，再以真实 JS Runtime 运行同一会话外层协议；各环节的取消、预算、资源释放分别通过，不以替换整个引擎掩盖内部组件未开放 |
| 4. 我发布的 Skill 确实被使用 | P3：仅存在于包内的 Skill 按精确制品装载，正文/引用资源可核对；同名目录不能替代它；缺失依赖明确报错 | P7：脚本/fork/hook 的执行与授权闭环；老 Session 内容不因升级漂移。传统目录 Skill 仍可用；不以“模型回答看起来像用了 Skill”作为唯一证据 |
| 5. 我不需要理解平台内部链路才能组装 | P1/P2/P6：安装 → 选择/模板 → 保存并使用；无需手填 digest/mount 或逐阶段审批 | P8/P9/P10/P11：保存后的绑定只有一份事实，启动不重新选择实现，UI/Runtime/部署服务不各造解析器；缺件一次说明，故障可定位，权限撤销仍生效 |

安排开发时先核销 P1 已完成部分，优先 P2 的真实替换；运行集成推进 P3，前端在公共 API 稳定后推进 P4，再以 P6 联合验收首发。随后按 §27.2 逐域推进 P7～P11，P12 按实际发布平台收口。**减少首期范围只能减少首期承诺；不增加 Rust 后端不能被用来减少这五组完整目标。**

还须补一项**跨部件组合验收**，纳入现有 P6/P8/P9/P10 联调，不另建平台或工作包：

- 在同一个 Agent 中选择来自不同插件的模型、Prompt/Context 和工具 Provider，通过原保存入口生成同一冻结执行计划；插件 UI 通过同目录的页面/Shell 绑定连接该会话，不把视图绑定强塞进 Session 执行锁。记录实际调用身份，证明这些部件可以同时使用，而不是四个互不兼容的演示环境。
- 只改其中一个绑定再保存：新 Revision 使用新实现，其他显式选择不被重置；旧会话保持原锁定版本。若旧实现被撤下，应明确报错，不能借“恢复”静默换回内置。UI 视图恢复不应重新提交模型请求或重放工具副作用。
- 同一个 UI 分别连接 Nomi 和真实 JS Runtime，验证公共会话 API 不依赖 Nomi 私有对象。引擎不支持的贡献须在组装时说明；不要求所有引擎实现所有 Nomi 内部阶段，也不允许静默忽略选择。
- 注入慢插件、流中断、权限撤销和界面崩溃，核对取消传播、资源清理及唯一 Session owner。验证的是已声明支持的组合及边界，不承诺所有第三方插件任意组合天然兼容。

这项验收区分“部件能单独替换”和“平台可定制组装”。需求 1/5 贯穿全部 29 行；完整需求关闭时，台账每行都应链接可复现的安装/选择、生产消费、失败路径和旧分支退出证据，并标出普通插件、部署服务或信任根例外。仅有接口、Mock 或测试数量不能作为完成依据；记录留在现有台账，不增加用户组装步骤或审批链。

### 27.11 如何判断开发计划确实回答了五个问题

本节保留五问对应关系与原工作包拆解；当前已交付内容以实施台账为准，执行顺序由 §27.14 统一替代本节历史“下一批”建议。P 编号仍可用于归属和抵扣，不意味着所有包同时开工。

不新增另一套阶段编号，继续使用 P0～P12；安排开发和对外说明时，采用下面的交付口径：

| 交付范围 | 可以对用户承诺的结果 | 仍不能宣称的结果 |
|---|---|---|
| R1-JS：P0/P1 剩余项、P2/P3/P4/P6 | 用户独立 capability 可被发现/选择；一个真实系统部件有 JS 替代实现；包内 Skill 基础装载与初始 Context 闭环；完整 Agent 页面可替换；一个简明的保存使用入口 | 全部系统部件、所有 Skill 执行模式、完整 Prompt/Loop、Shell 或部署服务均可替换 |
| R2：P7/P8/P9/P10 | 按 29 行清单完成 Agent 业务层、Nomi 内部有意义的策略、真实 JS Runtime、Shell/区域 UI；不同实现的依赖与资源可统一组装 | 所有替代引擎都支持 Nomi 的内部 hooks；普通插件能接管存储/认证的启动权威；任何会话都能无迁移热换引擎 |
| R3：P11/P12 | 分项验证部署服务的选择/切换与所发布平台；明确可安装服务与引导期服务，发布完整支持矩阵 | 普通插件可撤销约束自己的信任根；仅开放 Rust trait 或允许 fork 就算插件替换完成 |

**这五个问题不是五个互不相关的功能包。** 需求 1 和需求 5 是每个后续部件都必须遵守的横向标准；需求 2 跨页面与 Shell 两阶段；需求 3 必须按 29 行验收；需求 4 则可以较早形成独立、可验证的价值。不能完成 P2 的一个样例后就把需求 1 关闭，再让其他领域继续硬编码内置实现。

**本次核对后的优先级建议：下一批应同时有“真实部件替换”和“完整 Agent 页面替换”两项可见交付，不能长期只推进后端通用基础。** 前者检验需求 1，后者检验目前尚无替换闭环的需求 2；包内 Skill 已验证子项抵扣 P3，剩余执行模式单列，不能阻塞不依赖它们的页面工作。人员有限时可串行，但不能从 R1 验收中删除页面后仍沿用原承诺。

对需求 3/5 的高风险假设也应提前验证：在 P8 原包内安排一个真实流式模型或完整 Prompt 消费切片，验证既有执行边界的流、取消及状态所有权；这只是提前取得深层接口证据，不把所有内部组件提前塞进 R1，也不把一个试验算作 P8 完成。P9 的真实 JS Runtime 仍单独验收，不能替代默认 Nomi 内部的可定制性。

| 如果开发最终只交付这些 | 对五问的实际回答 |
|---|---|
| 更多 Tool、通用 Provider/依赖图、只读 Skill | 需求 1/4 的部分基础；需求 2 没有解决，需求 3/5 仍缺主要部分 |
| R1-JS 的真实部件替换、完整 Agent 页和基础组装闭环 | 五问相关的首批场景成立；需求 3 仅涉及少数环节，不能据此说模型/规划等已有交付；必须列出 R2/R3 未完成范围 |
| 再完成 R2 的内部组件、真实 JS Runtime、Shell，以及 R3 的分项部署替换 | 才能按 §27.2 逐行申请完整范围验收；最小信任根的例外仍须明确，不能宣传成普通插件接管一切 |

因此，排期既不能把“大目标”压成 Tool/Skill 工程，也不能把所有工作打成一个无法交付的大版本。范围有意缩减时，应明确说“本次先解决哪些问题”，而不是说“已完成原需求，只剩增强”。工作量仍按 §27.6 区分首发预算、历史主干参考和待拆分的剩余工程量。

据此，开发从已经接通的 Provider 分发和默认管理继续往领域闭环推进，不重复实现 adapter、选择器或默认存储。下面按当前剩余工作安排，已经验证的候选/Agent 选择、默认管理、保存漂移检查和恢复基础只保留回归（实施台账 §2.3～2.6），不再列为待开发功能：

| 现有包 / 责任 | 下一步可以直接安排的工作 | 本批退出条件 |
|---|---|---|
| P2/P7：公共合同、Compiler、授权与运行集成 | 核销台账 §2.15 已验证的递归图、守卫/消费者、保存比较与合同同步，以及 §2.18 已验证的 Context 子调用范围；继续产品可扩展 kind/multi-binding、非 Agent/资源/后台子调用和历史计划兼容；Host 默认并发风险继续跟踪，不重做同一图与 SDK | 新增资源和调用类型有真实消费、缺件/越权/撤销可解释；历史计划能否继续或需要显式迁移有明确结果。通用图和产品接缝通过均不关闭完整需求 1 |
| P2/P7：领域消费者 + 产品诊断 | 将已有选择/默认链接入 Browser/Computer 等实际调用点；补升级/撤下影响清单与全局恢复错误诊断，保留已有跨包恢复回归 | 用户选择确实改变领域动作落点；关联受影响 Agent/依赖可解释；旧内置旁路退出，失败不自动换 Provider；资源撤下竞态另有验收 |
| P3/P7：Skill 运行集成，与 P8 共享子任务接缝 | 保留台账 §2.8～2.9 的精确包锁/只读装载/Session 与显式命令/补全回归，核销 §2.16～2.17 冷启动及附图/装饰输入的已验证子项；按 §18.4 分开安排完整权限提示、受管 shell、生产 fork、其他模式与来源/历史保留；fork 先明确现有委派入口、权限/预算/效果归属，不另起本地执行平台 | 所声明的 Skill 模式确实执行且不扩大授权，缺制品明确失败；Gateway 不静默回退到嵌入式 runner；旧版本不漂移，历史数据切换有证据；命令/装载完成不代表执行模式全完成 |
| P4：UI/公共应用 API | 用完整插件 Agent 页面验证查询、发送、流、取消、历史与续订；不复制业务状态 owner | 切换界面不重放 turn，故障可恢复；仍不将这批记为 Shell 已完成 |

共享合同由一个 owner 收口，接口稳定后 Skill 运行集成与 UI 可并行；P2/P7 的重叠表示基础与完整能力分期，不表示重复计费或互相等待整个包结束。完整目标仍继续进入 P7～P11。现有 Context/取消、Role 分发和 Plugin 统一工程按证据抵扣对应子项，不重新估一遍已完成工作。

**P4 的可执行拆分（纳入原包，不另加预算包）：** 当前命令桥接已有实现与产品测试，下一步不是从零重写，也不能直接跳到“页面完成”。台账 §2.19 的测试夹具失败与生成漂移已在原命令任务内修复，不另起一条桥接链。

| P4 子任务 / 主责 | 实际交付与退出条件 |
|---|---|
| 命令与授权闭环 / 后端接缝、合同 | 核销 §2.19 已验证的 SDK、严格命令、撤销竞态、DB 及产品命令子项，当前 canonical 检查已通过；后续页面接线保留这些回归并补端到端授权/升级组合。保留结构化错误，结果不明不自动重试；旧 release 使用新 SDK 须重新构建发布 |
| 流式观察与恢复 / Session API、前端适配 | 定义实时事件和持久化历史的关系、续订位置及缺口处理；验证刷新、断线、慢消费者和取消。复用原 Session/事件设施，不为 UI 创建第二套执行日志；不要求为每个 token 新建持久化账本 |
| 同目录页面绑定与恢复入口 / 前端、Control Plane | 用户在产品中选择 UI contribution 后，真实 Agent 路由使用该界面；内置页面走相同公共应用 API。无会话时仍能呈现入口；明确单视图限制或完成多视图支持。故障可退回内置视图，不重放 turn、不替换已运行引擎 |
| 联合验收 / P4 接入原 P6 | 在同一真实 Session 上完成插件页面发送、流、取消、历史、断线恢复和卸下故障视图；覆盖已支持的桌面/WebUI，不把源码解析测试当作真实交互验收。通过后只核销 Agent 页面范围，Shell/区域/结果呈现继续留在 P10 |

命令授权和事件契约明确后，前端选择/呈现可以与后端恢复工作并行；P6 必须等待它们共同闭环。这里新增的是对剩余任务的明确拆分，不是要求用户多走授权/组装步骤。对需求 2 的答复应随之分为“Surface 已有、命令桥接在途、完整页面待验收、Shell 后续交付”，不能合并成一个已完成标签。

§2.21 后续进展校正上述拆卡状态：命令/流接缝已有证据，显式页面发布与同目录局部选择已通过真实产品链，页面宿主的卸载/精确授权/撤销/回退已有 DOM 验证。因此不再从零安排这些入口；P4 剩余重点为完整可用页面的历史/流交错与断网恢复、无 Session 入口、默认/模板绑定和实际浏览器联合验收。当时选择不持久化；后续预设级默认及工作台入口见 §2.30～2.31，并非通用 UI Role/模板传播完成，不能被“点击后能出现 iframe”抵扣。

§2.22 继续交付基础参考页面及普通草稿入口，减少作者必须手写 Session 接线的负担。这份参考页并不建立临时 token 合并器：查询当前页的持久化记录并整体替换，运行中记录可变，未持久化内容不承诺恢复；支持 SDK 的文本发送/取消，失败不自动重试副作用。它可作为可修改的真实插件源码，而不是另一套内置页面实现框架。**剩余工作不能一笔勾销**：完整消息呈现与真实浏览器故障验收、默认/模板绑定、无 Session 入口仍在 P4；此次“源码草稿模板”不等于 Preset 的 UI 绑定模板。跨重载的未发送输入/不确定请求恢复尚未交付，参考页明确提示保存文本与核对历史，不能宣称无损恢复。

R2 拆卡也按最小依赖安排，而不是先做完庞大的 P7 才启动 P8：资源/事件/取消接缝稳定后，可分别推进记忆、模型流、完整 Prompt 管线与中间件；初始/动态 Context 分发、消费与用户贡献排序保留回归，不从零重做。生产子任务接口由 Skill fork 和协作策略共用。P9 先完成 §27.9 的 Runtime 选择合同演进及 Session 外层协议，再接真实 JS 引擎；不等待每个 Nomi 内部策略全部开放，也不能用这个引擎替代那些策略的验收。P10 的 Shell 在 P4 公共应用 API 和恢复链稳定后推进，不等待模型/规划全部完工。以上是任务依赖，不是重新承诺未拆卡的工期。

为使后续工作能进入开发安排，下面给出 **P7～P11 的拆卡底稿**。每行是原包内部的一组任务，不是新增平台或重复预算；“责任”是主责模块，跨层合同仍由同一负责人收口。它补充 §27.2 的逐项验收，不改变全部 29 行范围，也不表示每组只有一张卡或相同工作量。

| 原包 / 对应环节 | 主责与必须交付的用户结果 | 最小前置与不能替代的验收 |
|---|---|---|
| P7：02/03/09/11 | contracts、Kernel、产品资源/领域消费者：用户契约跨包消费、自定义 kind/命名 binding、真实 Browser/Computer Provider 替换 | 沿已有绑定/依赖图继续；每个领域验证实际动作与资源释放，通用 echo 用例不抵扣领域交付 |
| P7：07/08/10 | Skill/MCP/Service 消费者经既有后端接缝接入：支持所声明的 Skill 执行模式、MCP mapping 和带资源 Service | 复用精确包锁及受管资源；fork 依赖生产委派接口；三条链分别验证，不以只读 Skill 或无资源 Service 代替 |
| P7/P8：12/13/14/24 | Host 生命周期、事件与 Nomi 阶段消费者：可选 middleware、事件源/订阅、后台服务与观测/评估实现 | 共用取消/有界流，但生命周期和观察者不成为第二个 Session owner；阶段修改与只读观察分别授权/验收 |
| P8：01/05/17 | Nomi 发现与模型接缝：可选工具搜索/暴露策略、模型 Provider 与模型路由 | 先验证真实 JS 模型流、工具调用和取消；固定候选范围不等于固定路由决定，模型成功也不抵扣工具发现策略 |
| P8：06/18 | Nomi Prompt/历史消费者：可选完整 Prompt 构造、上下文选择和压缩器 | 捕获实际模型请求、验证预算及恢复；已有 Context 追加仅计已完成子项，隐藏内置拼接必须退出 |
| P7/P8：19 | 记忆/知识资源与 Nomi 消费者：记忆读写、检索、embedding 分别可选实现 | 使用受管索引/资源及来源信息；证明影响实际 Agent 记忆/检索，不是只增加一个搜索 Tool |
| P8：20/21/22 | Nomi 引擎、既有执行/委派服务：规划/停止、工具编排/并行/结果转换、协作策略分别可换 | 共用现有任务与效果所有权；每个策略验证真实决策、取消和副作用边界；不新建 Skill 专用子 Agent 平台 |
| P9：23，联调 20/22 | `nomifun-ai-agent` Runtime 接缝：真实 JS 引擎通过同一会话 API 替代 Nomi | 先演进 Runtime 选择合同和完整 descriptor；Mock 不算第二引擎，替换整引擎不抵扣前述 Nomi 内部组件 |
| P10：25 | UI/公共应用 API：可选 Shell、独立页面区域、主题/结果呈现及独立客户端验证 | 复用 P4 会话 API/恢复；逐类发布支持范围，不因完整页面可替换便宣称所有区域/桌面容器已开放 |
| P2/P8/P11：27，贯穿全部行 | Control Plane/Compiler：可选组装策略、唯一冻结计划、退出运行期重复求解；部署时可选目录存储 | 新策略提出候选仍经原最终校验；状态检查保留，不把目录存储替换变成第二个 Catalog |
| P11/P12：15/16/26/28/29 及桌面服务 | 产品组合根与各部署服务：Channel、调度、存储、认证/Secret、监督/隔离逐项验证第二实现 | 先拆启动依赖和状态切换；业务服务与信任根分开。另写 Kernel、强沙箱产品化不混入现有粗估 |

每组拆卡时都写出“一个内置实现 + 一个可安装的 JS/UI 实现 + 真实消费者 + 故障路径 + 退出的旧分支”；部署级组改为第二个可配置服务实现，并标明生效窗口。不能把所有接口声明先做完、真实消费者留到最后。普通插件发布安装、同目录选择与统一组装是这些卡的共用交付条件，不再各自建一条发布/运行主链。

**安排投入的判断：P7/P8 是剩余主体，不是几个 hook 的小修；P9、P10 和 P11 也分别是完整引擎、界面及部署工程。** P5 暂缓只去掉另一执行后端的专属工作，不会使上述组自动完成。建议先完成 R1 的真实部件与页面交付，同时在 P8 内提前验证一条深层模型/Prompt 链，再据实际改造和测试成本滚动估算各组；在拆卡前不把 §27.6 的旧总量重新包装成当前剩余报价。

从投入取舍看，**可以暂缓新增 Rust 执行后端，但不能用它解释其余工作量消失**：异构 Provider 涉及合同/编译/授权/消费者的联动；UI 涉及公共应用 API 和恢复；Skill 涉及精确装载与执行语义；模型/Loop/整个 Runtime 涉及流、取消、持久化与单一执行所有权。按上述顺序交付可逐批获得价值；并行应建立在各批最小接口稳定后，不要求先做完全部 P2/P7 才允许 Skill 或 UI 启动，也不把旧 R1 预算当剩余全栈工期。

工作量仍沿用 §27.6 的口径：**14～24 人周是原 R1-JS 基础预算，不是五项完整目标的剩余工期；45～79 人周只是旧全路线的算术参考。** 本次补清的实现差异、深层消费者和部署边界必须在对应包拆卡时计入，不能靠取消 P5 或使用风险余量隐去。暂缓 Rust 插件避免的是另一种执行后端及其分发/SDK 工作，不会消除公共契约、宿主拆分、真实消费与稳定性验证的主要成本。

具体安排开发时，建议以“可演示的用户结果”而非 crate 数量开任务卡：

1. **下一批 P2/P7 从通用图推进到真实领域**：按 §27.8/台账 §2.15 核销已通过的递归图、用途隔离和产品接缝验证；保留对外契约负例与生成检查。接下来交付实际 Browser/Computer 等消费者、产品资源 kind/multi-binding 和剩余调用类型；旧格式拒绝不能抵扣历史兼容/迁移工作。其他领域不必等待整个 P7。
2. **P3/P7 与 P4 独立推进**：Skill 核销冷启动与附图/装饰输入已验证切片，继续完整权限提示、来源与执行模式；执行模式复用生产子任务接缝。前端以完整 Agent 页及公共应用 API 为交付单位，UI 不必等待记忆/规划插件化，fork 不另建执行平台。
3. **R2 按业务域做纵向替换**：模型/Prompt/历史、记忆、规划/编排/协作分别接入默认 Nomi；整个 JS Runtime 和 Shell 按各自外层协议推进。每个域同时交付声明、选择、真实消费、失败处理和旧旁路退出，不先批量建一堆无人调用的接口。
4. **R3 分开处理部署与信任边界**：存储、认证、Secret、监督后端逐项拆解引导依赖、切换与数据责任，再估算；未支持的项留在清单中，不作为“都能插件化”的宣传承诺。

一个后端负责人维护公共合同与唯一编译/授权链，运行集成负责人接真实消费者，前端负责人接 UI/选择与恢复；这是职责建议，不是要求增加审批层。人员少时沿同一顺序串行，人员多时只在接口稳定后并行。每批评审只问三件事：用户能否选用、生产是否真的消费、失败能否稳定处理。只有接口或测试替身的任务不能独立记作用户需求完成。

### 27.12 JS-only 能否兑现目标：可行性、待证假设与投入决策

**架构上有实现路径，不等于剩余工作已具备准确工期或全产品验证。** 既有切片能证明 JS 可沿原 Kernel 契约替代部分内置贡献：动态 Context 已经过实际包安装、保存、Session 和 Nomi 模型请求消费，异构依赖图与公开/内部消费也已有独立回归和合同同步。仍需接通全部领域与资源消费者；生产 `AgentRuntimeHandle` 仍只有 Nomi，系统 Router 仍直接装配内置页面。这三处分别是需求 1、3/5、2 的主要剩余改造点，不会随取消 Rust 插件而消失。

本次五问追问复核再次核对了 [Provider 兼容检查](../../crates/backend/nomifun-agent-kernel/src/materialize.rs)、[Compiler](../../crates/backend/nomifun-agent-kernel/src/compiler.rs)、[公开调用入口](../../crates/backend/nomifun-agent-kernel/src/registry.rs)、[Nomi 消费者](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs)、[生产 Runtime handle](../../crates/backend/nomifun-ai-agent/src/runtime_handle.rs)、[系统页面 Router](../../ui/src/renderer/components/layout/Router.tsx) 和 [包内 Skill 消费者](../../crates/agent/nomi-agent/src/host_skills.rs)。确认：递归图按所选 Provider 解析，消费者/公开入口使用用途字段，保存复用比较图；Runtime 的另一个变体仅是测试 Mock；页面 lazy import 仍是固定页面选择而非插件绑定；包内只读 Skill 明确拒绝 shell/fork/hooks 等模式。[产品资源解析](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs) 仍有固定 kind 与每 kind 一个选择的限制；[产品 Session](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs) 仍重编译并比较已存 Snapshot。正式 05 §15.3.4 仍禁止 Revision runtime selector，逐 Agent 引擎替换须先演进这项合同；依赖图的规范与 schema 则已同步，不混为同一个未完成项。本轮定向检查及留存结果分别见 §27.8，不作工作区整体通过承诺。

不新增工作包，在原包内部优先验证下列风险；结果决定该包细化估算，不另建一套试验平台：

| 待证明事项 / 原包 | 最小有价值的验证 | 若未达到预期，如何决策 |
|---|---|---|
| 通用异构依赖切片可推广到真实领域（P2/P7） | 保留已验证的真实 JS 多跳、用途隔离、正常保存/打开与生成合同；在实际 Browser/Computer 等领域验证资源/动作落点，并补历史计划兼容 | 对具体领域边界补实现/测试；不重建已有图与 SDK，也不恢复依赖完全相等限制或靠增加 Rust 后端绕过问题 |
| UI 真的脱离内置页面（P4/P10） | 插件 Agent 页独立完成发送、流、取消、历史和恢复；卸下故障视图不重放 turn，Shell 后续沿同一应用 API 接入 | 补齐公共应用 API 与状态所有权；禁止复制后端业务逻辑或调用私有内部接口来伪装替换 |
| 深层策略适合跨进程消费（P8） | 真实 JS 流式模型/Prompt 策略在连续工具调用中工作，记录 IPC 负载、首响应和取消；验证慢插件不持有引擎大锁 | 优先调整批处理、流和有界队列；无法接受的原生依赖/计算瓶颈有测量证据后，再单独评估 Rust 后端，不预设 JS 与内置零开销等价 |
| 第二个引擎不会成为第二套平台（P9） | 演进正式 Runtime 选择合同后，JS 引擎消费同一冻结会话计划；发送/取消/事件归属只有一个 owner | 修正会话外层协议和 descriptor；不能让 JS 引擎被 Nomi 再调度，也不能用测试 Mock 代替真实替代引擎 |
| 部署接口无引导循环（P11） | 分别列出存储、认证、Secret、监督服务启动依赖，验证替代实现的装载/重启/数据保留；标明哪些可普通安装 | 对需要 bootstrap 的服务单列部署交付；未证明普通插件可替换的项保持未完成，不以公开 Rust trait 或重编译宿主抵账 |

Skill 的最小装载链不再列为上述未知假设：包内只读正文/资源和显式命令已有定向证据。剩余 shell/fork/hook 属于实际执行能力建设，按 §18.4 接生产权限、效果和取消链路；不把只读成功算成全部模式成功。组装简单性则贯穿所有行，按 §27.5 检查重复解析与旧旁路是否退出，而不是最后再做一次全平台重构。

另有一个与语言无关的发布前提：**当前普通 Node 进程不是强安全沙箱。** 类型化 DTO、Kernel 授权与协作取消能约束受管调用，但不能据此保证任意第三方代码无法绕过 SDK 使用其 OS 权限。可信本机插件与受限不可信插件必须明确区分；没有对应平台隔离证据时，不承诺后者。增加 Rust 进程插件不会自动修复这个问题，详见 §17.4。

本节原建议为暂缓 Rust 插件后端、继续完整 JS/UI 路线；现按 §27.13 的用户决定调整：长期目标不收缩为 Tool 平台，但本期不为 JS 不适合的实现建设临时版本。批准下一批开发应依据上述真实消费验证和原有 29 行台账，已验证子项只保留回归，未拆分的 P7～P11 不给出伪精确剩余总工期。普通可安装插件、部署级替换、最小信任根三种边界应在发布支持矩阵分别列明，不能用同一个“全栈已完成”标签混在一起。

后续恢复收尾（实施台账 §2.23）更新上述 P4 状态：参考页已用原插件 KV 版本读取/CAS 保存草稿与不确定发送意图，发送前确认保存，重开后仅允许用户显式同键重试；未确认编辑仍可能丢失。没有新增发送执行器或消息账本，也未完成真实浏览器故障联合验收，不能宣称无损恢复。用户指出过度打磨后，停止继续扩展参考页，下一批回到 ToolSearch 等真实部件替换；完整 UI 剩余项继续追踪，不因此核销或归入 Rust 延期。

回到部件开放主线的首个改动见实施台账 §2.24：ToolSearch 内置实现已拆开只读排序与原子激活，保留原行为并通过 Registry/ToolSearch/Agent 激活回归。发现/排序适合粗粒度 JS 决策，不需新增 Rust 插件后端；但同目录策略合同、用户选择和真实 JS 调用尚未接通，因此不算行 05 或需求 1/3/5 的完整交付。

后续 §2.25 已继续接线：`system.tool-discovery` Role 将内置与用户独立 capability 的纯排序实现纳入同一选择/冻结链，实际 ToolSearch 沿 Kernel/JS adapter 调用隐藏 action，再由宿主校验并激活。具体测试状态以实施台账为准；已安装包的此切片不等于发布型 Product 策略、schema 暴露控制、大目录性能或整行 05 已完成。没有恢复 UI 打磨，也没有新增 Rust 插件系统。

### 27.13 用户确认的本期取舍：JS 不合适的实现延期，不制造临时版本

**本期目标修正为：完成适合现有 JS/UI 执行形态的开放化与必要宿主拆分；明显更适合未来 Rust 插件的具体实现延期。** 这是用户新确认的工程取舍，不再沿用“除了 Rust 后端以外的所有行必须本期完成”的强制口径。29 行保留长期追踪；下一批任务只选择能自然实现并产生真实用户价值的部分。

判断单位是具体实现，不是语言标签或整个能力类别。例如，远程模型 HTTP 调用适合 JS，但本地推理计算内核未必适合；检索策略适合 JS，但本地向量索引引擎可以延期。Rust 宿主仍须修改真实消费接口，这不属于新增 Rust 插件系统，也不因为宿主用 Rust 就全部延期。

| 原 29 行 | 本期可继续的自然 JS/UI 边界 | 不做临时 JS 版本的部分 / 进入条件 |
|---|---|---|
| 01/02/04/05 | 同目录声明、Provider 选择、模板、工具及发现/排序策略 | 不另写 JS Catalog/依赖编译器来复制 Rust 权威；选择建议通过同一最终校验 |
| 03/09/10 | 现有资源句柄、命名绑定与 Service 消费的公共接缝 | 必须直连原生对象/设备且无合理公开服务的资源实现延期；不以任意二进制启动绕过延期的 native 后端 |
| 06/07/08 | Prompt/Context 构造、精确 Skill 装载、MCP 映射；执行模式复用现有受管执行/委派接口 | 没有现成安全接缝时不新增 Skill 专用 shell/fork 执行器；依赖原生库的 Skill 实现另列，不用复制装载器补洞 |
| 11 | 基于已有浏览器协议、受控宿主服务的 Browser/Computer 实现 | 新设备驱动、OS 原生控制内核或只能密集跨进程调用的实现延期；不能新增一个绕过权限的原生代理 |
| 12/13/14 | 明确阶段的 middleware、授权事件、Node 生命周期内的后台业务服务 | 高频同步引擎 hook、同步可变内部对象与必须持有引擎锁的回调不跨到 JS；需此类接口的具体功能延期 |
| 15/16 | 网络 Channel、业务触发与调度策略接入既有任务所有权 | 新建系统守护进程、启动期调度权威或 OS 级实时机制先不做 JS 替代；普通异步业务不因此整类延期 |
| 17/18/19 | 远程流式模型、模型路由、Prompt/历史选择、模型辅助压缩、检索/记忆策略 | 本地模型计算、embedding 内核、原生分词/向量索引等无成熟 JS 路径时延期；避免反复传输大块历史/向量，仅为绕语言边界建立专用服务 |
| 20/21/22/23 | 粗粒度规划/停止/工具编排/协作决策；能自然使用现有异步服务的真实 JS Runtime | 若完整引擎必须依赖高频内部回调、共享原生状态或再造 Session 调度器，先延期该实现，不交付套着 Nomi 的伪替代引擎 |
| 24/25 | 异步日志/评估出口、完整 Agent 页面、Shell、区域/结果呈现及公共应用 API | 原生 tracing 热路径、桌面容器/设备服务不伪装成 iframe 能力；UI 本身不应因为延期 Rust 而停工 |
| 26/27/28/29 | 已有消费者确实需要的组装建议、公开服务适配和职责拆分 | 新的底层存储后端、OS Secret/native 认证后端、supervisor/sandbox 实现先延期到部署/Rust 方案评估；不做 JS 数据库代理、原生进程启动后门或用于占位的空 port。最小信任根仍不是插件自我替换能力 |

这是基于当前边界的初步分类，不是假装已做完性能测量。进入具体任务前重读消费者，并记录：实际输入输出/状态 owner、是否有成熟 JS 实现或公开协议、跨进程调用频率/负载、取消与清理能否复用。只有真实风险点需要小规模验证，不为每个普通 Tool 增设一套评审门禁。

出现以下情况时，停止该具体实现并登记延期：需要专门的原生辅助程序才能掩盖语言限制；需要复制业务状态或执行 owner；正常负载测试表明通信/计算成本不可接受且无法通过合理粗粒度接口解决；平台所需 native 库或设备能力没有成熟支持。登记原因、依赖和未来验证条件即可，不保留半成品生产分支。相反，仅仅“Rust 可能更快”或“接口还没拆”不足以将自然 JS 业务全部延期。

本期不为未来 Rust 后端预建 SDK、loader、ABI、制品目标管理、空 trait 或平行协议。只有本期内置实现和可安装 JS/UI 实现都实际消费的边界才进入产品。未来 Rust 应复用届时成立的契约，而不是现在为未知需求铺一套框架。

**延期不是“先用 JS 兜一下，再换 Rust”。** 任务拆卡时只给出三种处置：自然适合 JS/UI 的实现进入本期；边界有疑问的实现先做限定范围的验证，未证明前不承诺上线；明确需要原生能力的实现直接延期并保持现有内置路径。验证若发现需要新增专用 native 代理、第二份状态或第二个执行 owner，就停止该实现，不把试验代码挂进生产，再用“后续 Rust 替换”解释债务。延期记录只需保留具体功能、原因和重新启动条件，不要求先为它拆空接口或放置占位插件。

**Rust 插件也不是所有延期项的自动解法。** 进程外 Rust 同样有 IPC/复制成本；进程内 native 装载还涉及 ABI、崩溃隔离和权限信任。换语言不会消除多重 Session owner、目录双轨或授权问题。只有原生库复用、计算/内存路径或设备接口的收益明确时，才值得重开 Rust 后端工作包；其工作量按一种执行后端评估，不等于再建一套 Catalog、组装器、UI 或 Session 平台。

排期随之调整：先收口已经实现的 UI 命令/合同，再继续真正的页面选择及事件恢复；Provider/Skill/资源按上述自然边界推进。深层模型/策略/Runtime 先验证粗粒度调用与所有权再扩大范围；原生密集与部署底层实现登记延期。§27.6 的旧全量数字不能直接减去几个比例形成新报价，需按筛选后的具体任务重新估算。完成审计区分本期已验收、仍在途、用户允许的延期和信任根例外，不能把延期计成全栈目标已完成。

安排人力时不把“研究全部 29 行”变成“本期同时开发 29 行”：先交付 UI/Skill/真实 Provider 的剩余自然切片，再挑一条远程模型或粗粒度策略验证，最后根据结果排后续域。每批只估算可说明输入输出、实际消费者和故障退出条件的任务；未经验证的深层引擎和已延期 native 实现不混入本期交付承诺。需求 1/5 在每个已开放域验收，需求 2 继续 JS/UI 实施，需求 4 按支持模式闭环，需求 3 则明确保留原生密集实现的缺口；因此本期依然不能宣称五问全部解决。

### 27.14 进度纠偏与下一批停止条件

当前不是全栈开放完成态。统一选择/冻结、部分 Context/Skill、Agent 页面及安装型和 Product 发现策略已有切片证据；Product `before_model` 已按台账 §2.28～2.29 限定范围收口。最近完成的功能卡为 §2.32 消息来源与基础呈现，§2.33 审批前置仅研究、未实施；29 行中的其余领域继续保留。不能用子卡或测试数量换算整体完成百分比。

**已出现的过度工作是参考 UI 连续打磨过深，以及批次完成后顺势扩展相邻需求。** 停止继续增加参考页功能；必要的身份、取消、失败验证保留，但不以“还有完善空间”为理由无限延长一批。这里调整执行顺序，不删除 UI 或深层部件的长期目标，也不把它们全部推给 Rust。

以下至“当前交付基线与唯一接续顺序”前保留的是 **§2.27 发现策略批次的历史交付记录**，其中“本批”不表示当前仍在开发。该批已验证：**用户通过已有产品发布入口发布发现策略，并在 Agent 中直接选择使用，来源差异不再迫使用户改走安装型包。** 它补齐需求 1 中发现策略的实际产品路径，不是另做一个排序算法或插件平台，也不代表需求 1 的全部领域消费者已经完成。

| 任务 | 实际落点与验收 |
|---|---|
| 接通发布型隐藏策略 | 复用 `NomiPluginProductToolInvoker` / `PluginRuntimeApplicationService` 的精确 release 校验与执行，不将 Product 伪装成已挂载包，不另建 Registry；仍消费同一发现输入/输出合同并由原 Session 激活 |
| 保存时发现冲突选择 | 同一选择校验用于 Session 装配与宿主编译检查；多策略保存拒绝，无改动保存也校验并复用原快照。不新增领域编译器；当前没有独立 preview API/UI，不把编译校验冒充预览交付 |
| 真实产品纵向验证 | 源码创建/构建/发布/启用 → Catalog → 保存/冲突拒绝/复用 → Nomi 工具循环 → 真实 Node Service 已通过；非法输出、5 秒超时、停用后既有 Session 明确失败且不增加 schema。上游模型受控；不冒充浏览器、非空大目录排序或 release 升级重开联合验收 |

当前实现：`materialize_plugin_product_actions` 识别精确隐藏合同并核对 schema 摘要，装配后不暴露为模型工具；所选 Product 缺执行适配器则拒绝注册。沿用原 owner、release、epoch 与 catalog digest 检查，不增加 Rust 插件后端。发布声明使用现有 `contributions.capabilities` + `schemas`；普通 `actions` 简写仍只生成可见工具。Product 必须声明 Agent + PluginService 消费者，当前不支持资源绑定，也仍禁止 release 声明 Role，故只承诺直接 capability 选择。

本批前置检查与修复见实施台账 §2.26：产品 Service 原调用 future 被放弃时会遗留 Host 在途登记，取消轮询也随之消失；已在原 Host/Node Actor 内修复，没有新执行器或协议。其真实 Node 取消/回收证据与 §2.27 的产品调用证据分别记录，不把前置测试冒充完整纵向测试。当前消费者 31 项、编译器 17 项、App discovery 3 项及最终真实 Product 集成 1 项通过，详细命令和边界只在台账记录，不以测试数量换算全局进度。

**本批明确不做：** 新排序算法、远程模型排序、可配置 schema 暴露预算、大目录性能平台、UI 参考页扩展、通用 middleware 或第二个 Runtime。这些保留各自任务，不被偷偷并入本批。若真实负载或调用生命周期暴露上线阻断问题，只修满足本批语义所必需的部分；需要 native 代理、重复状态/执行 owner 的具体实现按 §27.13 停止并登记，不交付临时版本。

本次来源接入/保存冲突切片与文档同步后停止扩展；行 05 仍有 schema 暴露策略/预算、大目录等剩余项，跨领域升级与生命周期的联合验收也不因此关单。随后再单独确定下一个实际部件消费链。每批只报告用户新增能做什么、尚不能做什么及验证范围，不重跑没有相关变化的全仓测试。当前剩余工期仍需分批估算，旧人周数字不作为这些未拆任务的承诺。

#### 当前交付基线与唯一接续顺序

前轮进度核对只更新安排；A 卡来源/消费者证据见台账 §2.28，用户排序与冻结收口见 §2.29。已有工作区改动混有历史/其他改动，不能把 `git status` 的全部文件当成本批成果；未提交也不等于未经验证，更不等于可直接发布。合并时按功能与依赖审查，不自动提交、回退或清理其他工作。

| 当前基线 | 已有证据 | 排期中不能再次从零建设 / 不能据此关单 |
|---|---|---|
| capability 与组装 | 台账 §2.2～2.7、§2.12～2.15：选择、冻结 Provider、依赖与受管调用切片 | 复用目录/编译器/执行链；尚非所有领域可替换 |
| Context 与 Skill | §2.8～2.11、§2.16～2.18：只读装载、命令、动态 Context 与排序等 | 保留相关回归；完整 Prompt 和其他 Skill 模式仍待做 |
| Agent 页面 | §2.19～2.23、§2.30～2.32：命令/事件接缝、发布选择、参考页七类消息展示、预设默认与工作台入口 | 不重建参考页框架；权限/附件等交互、完整浏览器联合验收和 Shell 未完成 |
| 工具发现策略 | §2.24～2.27：安装型和 Product 的真实消费及失败路径 | 已结束的历史批次；不顺带追加暴露预算/大目录工程 |
| 模型请求中间件 | §2.28～2.29：Product `before_model` 的真实请求变换、编辑器排序、保存复用与会话冻结 | 当前卡收口；不据此关闭全部 Middleware、N1 来源或其他模型调用 |

以下是剩余工作的依赖分组，不新增另一套工作包，也不替代台账 29 行；不是必须依次清空 A～F 的串行排期。**同一执行者一次只推进一张纵向实现卡；一个阶段可以包含多张卡，完成一张不自动完成整阶段。** 下表的“本期候选”只表示符合 JS/UI 方向，尚未验证的边界不算交付承诺。

| 顺序 / 处置 | 对应原环节与用户结果 | 前置、验收和停止边界 |
|---|---|---|
| A：当前部件卡，限定范围已验收 | 12，关联 06/21：用户发布、选择并排序真正影响模型请求的 Product `before_model` middleware；详见下卡 | 来源、消费者及用户指定冻结顺序已验证；本卡到此停止，不自动扩展其他阶段、模型或 UI |
| B：Agent 页面剩余范围，本期分卡；暂停自动追加 | 25：模板绑定、插件创建前入口、剩余消息交互和真实浏览器恢复验收；持久默认与工作台空会话入口已有证据 | 沿已有页面/API 继续；发送/取消/历史/故障切回不重放 turn。交互审批须先完成下述共享宿主前置，不能当作已有接口的小 UI 卡。B 不阻挡无此依赖的 C/D；Shell 单列，多视图未做须明确保留 |
| C：领域与资源闭环，本期逐卡 | 02/03/04/07/08/09/10/11/13/14：命名资源/Service、真实领域 Provider、Skill 剩余模式、MCP、事件与生命周期 | 共享资源接缝是相应消费者的前置，不要求做完全部资源才接其他域。每卡验证真实消费与回收；缺受管执行/委派入口的 Skill 模式先查证，不造专用执行器 |
| D：深层策略与业务服务，本期逐卡验证 | 01/05/06/12/15/16/17/18/19/20/21/22/24/27：发现/Prompt/模型/历史/记忆/编排/协作、网络 Channel、业务调度、观测和组装建议 | 优先远程流式模型或粗粒度 Prompt 的实际请求证据；后续按领域独立验收。只传必要视图/句柄，复用授权、效果与任务 owner，不要求整个 C 结束才开始 |
| E：独立的大交付，本期候选、单独估算 | 23/25：真正 JS Runtime，以及 Shell/区域 UI | Runtime 依赖会话契约与单 owner；Shell 依赖 B 的公共 API。两者互不依赖，不等待所有内部策略，但不能用替代引擎抵扣 Nomi 内部开放。发现需原生密集调用时延期该实现 |
| F：明确延期的具体实现 | 26/28/29 及其他行中的本地计算/设备/原生热路径：底层存储、native Secret/认证、监督/沙箱等 | 保留内置路径；记录原生依赖、重启条件与未来验证。不是所有安全/隔离问题都等 Rust 就能解决；不创建代理、空接口或第二执行后端 |

A 已在下述限定范围收口；B 的“预设级默认页面”“会话前工作台配置/打开”及“来源消息展示”已有子卡限定范围证据，见下节及台账 §2.30～2.32，不代表 B 整体完成。C/D/E 为后续队列，进入时只拆最近一张卡，不预建全部接口。具备独立负责人时可按具体依赖分工；此处是人员安排建议，不要求自动启动并行工作。F 只覆盖 §27.13 中明确不适合 JS 的具体实现，不能用它推迟普通业务、UI 或必要的 Rust 宿主修改。26～29 中仍有自然适合公开服务的接缝，按具体消费者纳入 C/D，而非整行关闭。

**本次收敛决定与下一批安排：**

1. **先交付已完成的限定范围，不重新建设或全仓重验。** 按上述五组基线审阅相关改动及已有证据；本次复读 §2.32 的产品测试、Session 单测和类型检查日志，与当前来源字段相符。它们仍只是该卡的历史验证，不是最新工作区全量发布验收，更不是浏览器端到端证据。
2. **停止自动追加 UI 功能。** 预设复制继承页面偏好目前仅阅读原 `create_preset` / `fork_from_revision` 路径，尚未实现。个人复制与跨包模板传播是不同范围，不为关闭“模板”一格合并成大卡；审批、附件、Shell 同样不自动附带。已交付行为的确定缺陷仍可定向修复。
3. **下一实现批次优先补真实部件替换缺口。** 从 C 的具体领域消费者或 D 的远程模型/粗粒度策略中选择一个有成熟 JS 路径的功能；不以做完全部 B 为前置，也不同时启动多个领域。进入时只读该调用链，明确用户选择、输入输出、原状态 owner 和端到端验收，再给该卡人日估计。边界未核清前不承诺剩余总工期，不将既有成果重复报价。
4. **停止条件是这张卡的用户结果成立，不是所有相邻问题消失。** 实现、相关失败/取消证据、文档对齐后交付；新发现的相邻需求归原 29 行，不自动追加。确实依赖原生库、密集共享状态或专用 native 代理的具体实现按 §27.13 延期，不以空接口或模拟执行占位。

以上是一次执行范围纠偏，不新增评审平台、工作流门禁或第二份任务系统，也不缩减本期适合 JS/UI 的完整目标。本轮仅校正主报告与台账；没有完成预设复制、审批或新的部件开放，亦不宣称剩余工作已完成估算。

#### B 已完成子卡：记住此 Agent 的默认页面（限定范围验收）

用户可临时切换页面，也可显式保存该预设默认采用的插件 capability/不可变 release；不是覆盖系统 ID，也不是把页面当作模型 Tool。保存复用原预设存储，独立 CAS 版本不进入执行 Revision/Snapshot；每次进入 Session 仍需取得原 Surface 的新作用域授权。正常保存规范化 Catalog 展示信息，精确目标不可用时拒绝；已有默认失效时保留并提示，临时显示内置页，不静默追随升级或选择其他插件。

这张卡只补持久选择，不扩写参考页、不新增 UI 注册表、Session owner 或 Rust 插件后端。记住当前页面不应重开 iframe；临时选择优先于迟到的默认读取，导航后的迟到响应不能把旧页装到新 Session。授权保存针对不可变 release：同版停用后重启用，下次进页可复用默认意愿，但不会复用旧 Surface grant；当前路由内已撤销的页面不因刷新自动恢复。

收口复核修正了同一 Session 重入时的缓存缺口：自动选择须等本次进入的默认读取完成，不能先采用旧缓存后忽略当前设置；读取失败不以旧默认自动打开插件。工作台或其他会话清空/改变默认后，下次进入按新值解析；已经明确临时选择的当前页面不被后台读取覆盖。此项只修复原读取链路，见台账 §2.31 收口复核，不增加配置同步服务或第二套授权。

此子卡已有前端交互及真实产品/存储证据：精确发布选择、独立 CAS、非法/跨 owner 拒绝、停用保留、显式清空、Session 观察和磁盘快照重开通过；执行 Revision/Snapshot 与既有 Session 保持不变，默认选择未发起模型请求。具体命令、测试范围及夹具修正见台账 §2.30。数据使用 fresh `060` schema 的新 JSON 列与原索引/关联审计，不补旧数据兼容迁移；未打开或重置真实用户数据集。

到此交付此卡审查，不自动进入相邻领域。模板传播、无 Session 入口、完整消息/浏览器恢复体验、多 Surface、Shell 均仍未完成；B 整体、行 25 和 29 行完整目标不因此关闭。磁盘数据库恢复证据不冒充整个应用跨进程重启/浏览器联合验收。后续仍在 B 内单独选择最近的产品缺口，不重新建设持久默认，也不预建整套 UI 框架。

#### B 后续子卡：会话前页面配置与空会话入口（限定范围验收）

Agent 工作台的个人预设编辑器增加“Agent 页面”页签，用户无需已有 Session 即可读取/保存/清空同一预设页面绑定；保存不改 Agent 草稿，也不额外保存 execution Revision。随后“打开已保存的 Agent 页面”调用原 Session create，仅提交 preset ID/title，再进入原 Session 路由；不先发消息、不覆写模型或资源，不新建执行器/授权桥。旧“使用 Agent”引导入口保留，用于模型、工作目录和资源选择；缺少这些条件时返回原错误，不临时绕过初始化要求。

独立页面保存仍使用原 CAS，冲突后显式刷新、重新决定；不可用精确选择保留，清空必须明确选择内置页。Agent 草稿未保存时可以独立保存页面偏好，但不能误用旧执行配置打开；页面选择未保存时也不将候选假装成已生效默认。重复点击被合并为一个在途操作；切换预设后迟到的保存/创建结果不会把旧页导航到新预设。创建结果不确定时提醒检查会话历史，不自动重试；已知创建成功而导航未完成时复用该 Session，不重复创建。

这是**无已有会话时的工作台入口**，不是插件接管 Session 创建前的欢迎页、资源选择器或整个引导流程。插件 Surface 仍在原 Session 创建和作用域授权后加载；创建前仅保存配置，不给插件 Session 权限。测试已覆盖编辑器→保存→创建→实际页面宿主消费，以及真实后端“先保存、后创建第一个空会话”；证据见台账 §2.31。模板传播、完整消息/浏览器恢复、多 Surface、创建前插件 UI 与 Shell 继续保留，不新增临时全局插件权限。

#### B 后续子卡：参考页按来源消息类型呈现（限定范围验收）

本次补齐 B 已列出的消息展示缺口，不新增页面框架。原 Nomi 观察响应只传正文和左右位置，丢失了持久化 `type/status`，插件无法可靠区分工具、思考和回复。现在原 `MessageProjection` 可携带来源 `message_type/message_status`，正文、摘要、游标及唯一 Session owner 不变；事件型投影仍用原合同，不增表或第二套消息协议。用户插件可据此编写自己的呈现逻辑，不需要模仿内置 React 组件。

可编辑参考页已支持文本、提示、思考、计划、工具调用、工具组和 Agent 状态，全部用文本 DOM 展示，保留同页重读替换与不重放 turn。未知类型、权限交互和不可展示的结构化输出明确提示；工具错误状态不误显示为成功，不把工具完成冒充回合/产物交付完成。没有产物 URI 打开、审批动作或新的授权范围，也没有 Rust 插件系统。已有 release 不被模板更新改写，作者修改后需重新发布。产品与模板/SDK证据见台账 §2.32；附件/产物交互、其他专用消息、真实浏览器、多视图及 Shell 仍未完成，不能据此关闭 B 或第 25 行。

#### B 审批交互前置核对：不是已有 API 的按钮接线（未实施）

当前 Nomi 主链尚无已闭合的交互审批生产链。`MessageType::Permission` 是枚举成员，但 [AgentStreamEvent](../../crates/backend/nomifun-ai-agent/src/protocol/events/mod.rs) 没有对应的请求/决定事件；现有 [插件 Session 接缝](../../crates/backend/nomifun-app/src/router/plugin_ui_sessions.rs) 只处理 `observe/turn/cancel`。[ThinAuthority](../../crates/backend/nomifun-agent-kernel/src/authority.rs) 判定快照、主体、action 与资源是否允许，[SkillTool](../../crates/agent/nomi-agent/src/skill_tool.rs) 执行配置 allow/deny，均不是“等待用户批准后恢复原调用”的服务。§2.32 的 permission 消息夹具只验证类型透传，不能作为已有审批能力的证据。

因此，审批应先作为第 21/28 行与第 25 行共用的**宿主能力前置卡**，之后再接插件呈现；不在参考页中先建 Promise 队列、KV 审批账本、伪造 permission 消息或自动重发工具来代替它。这不是 JS 性能/语言边界问题，增加 Rust 插件不会补出缺失的状态所有权；共享宿主实现可用 Rust，但不等于增加 Rust 插件执行后端，也不归入原生密集实现延期。

后续实施顺序与结束条件：

1. **先确定真实请求产生点。** 在原调用授权/执行接缝中确定哪些已获准操作需要进一步用户确认；批准不能扩大 Snapshot 的 capability/action/resource 上限。首张实现卡只接一个真实需要确认的调用，不预建全领域审批总线。
2. **由原执行 owner 持有等待与决定。** 同一请求关联原 Session/操作与确定的调用意图；决定幂等，取消/超时/撤销后迟到批准不能恢复失效调用。需要跨重启恢复时使用原持久事实/操作体系，不由页面重建执行。具体合同必须在实现时沿实际 owner 落定，本文不预建空接口。
3. **再开放可替换呈现。** 内置和插件页消费同一查询/决定服务；用户选择一个能读写会话的页面，不自动意味着允许该代码批准能力升级或代替人类作安全确认。纯呈现与可委托决定须区分；不可委托的确认保留可信宿主入口，允许委托的决定需独立、明确且可撤销的授权。
4. **以真实执行闭环验收。** 覆盖实际等待→确认→原调用只执行一次，以及拒绝、跨 Session、旧意图、并发决定、超时/取消/撤销和恢复。仅显示按钮、读到 permission 字段或 mock 返回成功不算完成。

本次只完成前置核对，未交付新审批能力。其余 B 项不必等待整套审批完成；审批仍保留在完整目标中，不能因缺前置或 Rust 插件延期而删去。证据与排期归属见台账 §2.33。

#### D 远程模型 Provider：流式前置已接线，模型替换尚未闭环

本次从 D 选择远程模型方向进行了一次限定范围代码预检，证据见台账 §2.34。**远程流式模型仍适合 JS，本期不因 Rust 插件延期而移除；但当前不能按“已有统一调用链，只补一个 adapter”安排开发。**

预检时 Nomi 的 `LlmProvider` 已可注入，产品 bootstrap 默认经既有配置构造 provider；平台另有 `AgentPlatform::open_model_stream` → Chat Broker。Broker 的协议集合和适配器键固定为六种协议，Product Service 当时只提供一次请求的最终 JSON 结果（后续流式前置见本节末尾与台账 §2.35）。只改 Broker、只加 `LlmProvider` 实现或只放宽协议枚举，都不能证明产品 Agent 使用了用户插件。两条入口的存在不等于同次请求双重执行；本次不据此宣称重复计费或既有重试故障。

后续按以下依赖实施，仍以一个真实用户结果作为首张功能卡：**用户发布并选择一个远程模型实现，原 Nomi 会话实际消费其增量输出与工具调用，取消后不再继续执行该请求。** 仅完成下列某个内部步骤不能关闭该卡。

| 实施顺序 | 交付内容与边界 |
|---|---|
| 1. 落实真实消费者与所有权 | 沿 `nomifun-ai-agent` 现有构造链确定所选模型实现如何进入 Nomi；保留原 Session/turn owner。明确这条请求的路由、凭据租约、重试和取消由谁持有；若复用 Broker，须通过原接缝传入其需要的调用事实，不能伪造 causality，也不能叠加两层自动重试。第一步同时确定旧分支何时退出，不先建一个没有消费者的模型注册表 |
| 2. 在原 Service 中承载真正的流 | 复用原进程、request ID、generation、deadline 与取消登记；定义增量事件、明确结束/错误、有限缓冲和慢消费者退出。消费端丢弃、停用与进程退出必须关闭同一请求。不得全量缓存回复后分段模拟流，不用轮询私有 KV 或后台任务另造流执行器；不为每个 token 新发一次普通 RPC |
| 3. 接回同一选择与冻结链 | 区分模型路由身份、协议格式和实现 capability 身份；同协议可以存在多个可选实现，插件不覆盖内置 ID。精确插件制品进入已有选择/冻结语义，资源与凭据沿现有授权边界解析。不能靠修改全局 URL 冒充逐 Agent 的插件选择，也不为此建立第二 Catalog |
| 4. 用生产路径验收并结束首卡 | 发布/启用→目录选择→保存/新会话→真实 JS 执行→Nomi 实际增量与工具消费；覆盖选择变化/旧会话冻结、缺失或停用、非法事件、半途错误、慢消费者和取消。首次增量在最终完成前可观测；不允许失败后自动重放已经有可见输出的请求。受控本地模型上游足以验证，不需要真实 API key 或收费调用 |

首卡不附带本地推理内核、全多模态协议、全部 failover 策略、所有旁路 one-shot/压缩调用或独立 JS Agent Runtime；这些继续保留在行 17 及各自范围。首卡也不要求先全面迁移所有现存模型调用方；但必须交代哪些调用使用所选插件、哪些仍用原路径，不能把局部模型替换宣传为所有模型调用已统一。

工作量上，这应作为**含流式基础与真实消费者接线的独立工作包**，不能套用普通 Tool/已完成 `before_model` 的接线成本；后者是请求前处理，不是模型 Provider。当前消费接缝和路由身份方案尚未落定，不给未经依据的精确剩余人日。先完成上表第 1 步的局部设计与可执行验证，再估算同一首卡余项；不把研究无限延伸为全系统重构。若发现某个具体实现需要专用 native 代理或密集共享引擎状态，按 §27.13 延期该实现，而不是用临时 JS 版本填表。

§2.34 预检当时只完成研究和实施依赖记录，未实施模型插件功能，不新增空 trait、协议占位或依赖；后续实际改动见以下进展。预检及流式前置均不算行 17 完成，也不使整个目标阻塞。

**后续前置进展（台账 §2.35）：** 原 Service Host/Node 请求已实现可选有界增量通道，单个待 ACK 事件和消费队列形成背压；原调用持有终态、取消、deadline 与代际，不新增执行 owner。真实 Node 已验证增量早于完成、慢消费者时其他请求仍可处理、丢弃/取消、版本失效、错误帧及部分失败；既有应用调用与存储 IPC 回归通过。显式 null 终态与缺字段也已区分。这只是上表流式基础的实际进展，未先行解决第 1 步的完整模型消费者/所有权问题，也未交付模型选择、模型语义或公开 SDK；不能独立算作“JS 模型插件完成”。下一步仍围绕同一 Nomi 模型消费链接线，不扩为通用 EventSource 平台。测试命令与初轮夹具问题只在台账记账。

**产品内部接线进展（台账 §2.36）：** 增量通道已从原 Agent capability 应用入口经生产 Runtime 接到同一 Host/Node，保留原发布身份、授权与资源限制；未支持流的 Runtime 明确拒绝，原 unary 行为保留。真实源码发布链已验证增量、背压、拒绝、关闭接收端取消、部分失败和禁用失效，相关应用回归通过。这里的“产品入口”是受信宿主内部调用，不是新增模型配置 UI 或 HTTP 流权限；传递的是普通 JSON 值，模型语义仍待消费者校验。

**本批收口与下一批边界：** 当前前置改动已完成定向验证及文档同步，停止追加通用流/事件基础设施或 UI。下一批集中落实上表第 1/3 步并接入第 4 步的真实 Nomi 消费者；模型合同复用、精确选择身份、凭据及重试所有权未落定前，不再把“再补一个内部 port”当作独立用户功能交付。原 Nomi 使用所选 JS 模型的真实增量与工具调用、失败/取消证据齐全后结束首卡；不能用仅通过 Service 测试替代验收。原 29 行及五问的未完成范围保持不变，暂不报无依据的总完成比例或剩余总人日。

**同一模型合同归位（台账 §2.37，当前批次结束）：** 已将 Broker 原有 `ChatModelInput/ChatModelEvent` 等纯数据定义及验证移动到 `nomifun-agent-contracts::chat_model`；Broker 原导出继续指向同一类型，没有复制协议或增加执行依赖，原 Broker 回归通过。该归位使发布合同和 Nomi 接缝可以复用同一数据层，不代表 Nomi 转换、插件选择或凭据传输已实现。Nomi 的工具结果消息角色、工具参数增量及 Service 最终成功与模型完成的区别仍须在消费者中正确处理。当前按“尽快收尾、不要过度发散”停止追加实现；下一批只围绕同一真实模型替换结果，不能靠裸传宿主密钥、虚构 Provider 配置或伪造调用事实做演示，也不将这次内部归位计为新增用户能力。完整目标未完成。

#### 最近完成卡：模型请求前的 Product JS middleware（限定范围验收）

实施增量见台账 §2.28～2.29：已接入 Nomi 主模型/工具循环的局部请求变换及原 Product Service 调用；工具权限快照在变换完成后生成，计划模式、资源通知与回合事实由宿主保留。用户在原 Agent 编辑器中选择多个 Product middleware 并上下排序；独立 `middleware_order` 进入同一 Revision、Snapshot 和运行配置摘要，不混用 `context_order`，不另建组装器。显式项先执行，其余已选项按 capability ID 接续；空顺序恢复默认。重复、未选中及非本阶段贡献不能借排序进入执行链。

真实产品测试已证明：无改动保存复用原 Revision/快照；仅变更顺序产生新版本/快照，新会话采用新顺序，已有会话保持原冻结顺序。两个真实 Node 插件对提示词做不同的嵌套变换，以实际模型请求验证顺序，不以字段或 mock 向量代替消费证据。排序不改变 capability 选择、工具权限或 Context 顺序。N1 来源、其他阶段及压缩等独立模型调用仍未接通；它们继续留在行 12，不归入 Rust 延期。

**用户结果：** 用户发布自己的 `TurnMiddleware` capability，选入 Agent 后，在每次模型请求前转换获准的提示词视图和工具候选；不是只能追加一段 Context，也不是向模型多注册一个 Tool。该卡仅验证一个阶段，完整行 12 的其余阶段仍保留。

实施前代码依据：[Engine](../../crates/agent/nomi-agent/src/engine/mod.rs) 的 Context 求值位于主模型循环内，而 `ProviderToolAuthority` 当时在 Context 之前构造；[Product 消费接缝](../../crates/backend/nomifun-ai-agent/src/plugin_tools.rs) 的隐藏绑定当时仅处理发现策略。§2.28 增加请求消费者、将工具权限快照移到变换之后，并沿原 invoker 绑定 Product middleware；§2.29 补齐排序和冻结验收。下表保留该卡的责任与验收边界，不再作为从零实施的待办。

| 工作与责任 | 必须交付 / 不顺带扩展 |
|---|---|
| 合同与选择：contracts / control-plane / ai-agent | 确认真实 `TurnMiddleware` 形态、阶段输入输出、失败语义和多贡献顺序，进入原冻结计划；顺序影响结果，应支持明确配置并给出稳定默认，不通过“只能选一个”掩盖组合问题。现有 Context 顺序不能未经验证直接当成 middleware 顺序 |
| 真实消费：nomi-agent，经 ai-agent 装配 | 使用本次请求的局部视图；校验后再形成模型可见工具集合及其调用权限快照。不得增加原本未授权的工具、恢复被前序移除的工具、修改持久化历史或持有跨 JS 等待的引擎锁；可信宿主指令的保留边界须显式设计 |
| 发布接线：plugin-platform / app 组合根 | 先复用 Product 源码发布、精确 release/epoch/schema 校验与原 Service 调用；动作不暴露给模型。不新建执行器，不新增 app→nomi 依赖。安装型 N1 的 kind/handler 接入单列后续来源卡，不伪装成 Tool，也不归入 Rust 延期 |
| 验证：上述模块对应测试 | 同一实际模型循环证明提示词改变、工具子集与调用权限一致、多贡献按冻结顺序执行；非法结果、超时、取消、停用/缺适配器不得继续发起该次模型请求或静默回退；真实 Product 发布→选择→Node→模型请求验证，原无 middleware 路径保留回归 |

允许插件改变的字段、执行顺序、大小/总等待上限必须在实现前写清，但本次不冻结未经验证的 DTO、常量或公共 SDK。只解决此阶段确实用到的输入输出，不预建 `after_tool` 等全阶段总线；中间状态只在当前请求存在，链路失败不提交部分请求变更。非空工具候选及恶意模型返回被隐藏工具调用需要实际测试，不能只用空候选或 mock 的正确返回证明授权成立。

**原始估算与人力（不是剩余工作量）：** 在上述边界成立、复用现有 Service 且不扩展 N1 的前提下，本卡原按 **8～15 人日**安排：合同/消费边界 1～2，Engine 与冻结组装 3～5，产品接线/选择顺序 2～4，故障集成和文档 2～4。这是原人工排期估计，不是实测工时；一名熟悉代码的开发者约需 2～3 个工作周，代码审查与等待另计，多人分工不保证线性缩短。现在限定范围已有交付证据，不能继续把这笔原估算作为待开发余额；N1 和其他阶段单独估算，不挪用旧 R1 总预算。

**结束条件：** 上述用户结果、来源和失败路径有匹配范围的证据后结束本卡；测试命令/结果只写入实施台账，报告只更新能力边界。发现必须借助新原生代理、复制状态/执行 owner 或不合理热路径传输时，停止该具体方案并记录原因，不留下无人消费的 SPI。正常功能失败由本卡修复；其他领域机会进入队列，不继续顺手开发。A 完成不等于五问或整行 12 完成。

此后的工期按 B/C/D/E 最近一张卡滚动估算；不能将这张卡的人日乘以剩余行数，也不能继续把旧 14～24 / 45～79 人周当成当前余额。每次进度报告固定给出：新增用户能力、未完成范围、实际验证、下一卡及停止条件，不重复附上所有历史测试。

### 27.15 远程更新后的实施基线（2026-09-14）

本次按用户要求，将工作分支从 `439bb385a` 快进到 `2ff029512`（20 个上游提交），再整合已备份的本地开放化改动。**以下事实更新覆盖前文的旧代码基线，不代表原 29 项需求全部交付。** 合并后的验证结果统一记录在实施台账 §2.38，旧日志不作为新基线的通过证据。

- 上游已加入编译期注册的 Engine Catalog、Engine SDK、共享 Session 生命周期和 Coding 引擎；Agent 配置增加 `runtime_engine`。这不是安装型 Rust 插件系统，也没有开放上传动态库/原生可执行文件，不改变本期“不新增 Rust 插件后端”的决定。后续引擎开放应接入这个边界，不再新建平行 Runtime Registry。
- Catalog 投影已迁至控制面；本地 Role/Provider 候选随之迁移。引擎选择、Provider 选择、Context/Middleware 顺序保留在同一 Agent 编辑/编译流程中；引擎兼容性检查和消费者校验各保留职责，不能以合并为由删掉执行权限限制或冻结绑定。
- 上游已有实际 MCP 目录/工具/资源与包 Skill 消费，不再按“完全没有生产通路”从零排期。插件恢复与 MCP 刷新必须使用同一发布锁、保留成功发布的另一类注册，避免后刷新者抹掉已有目录。完整用户 Provider 替换、所有来源与故障覆盖仍需分别验收。
- Nomi 的包 Skill 正文/资源采用上游共享验证装载；本地显式 `/skill:...` 命令复用已验证正文，不重复读取制品或向模型注册第二套正文加载器。命令仍检查当前精确来源和依赖；不支持的 shell/fork/hooks 等正文只作为上游的只读参考数据（不执行这些指令），不生成可执行命令。图片仍走原资源工具及模型图像准入，不通过十六进制文本冒充视觉输入。
- 模型纯数据合同归位保留上游新增的推理块/签名验证；路由绑定、凭据、重试和传输仍归 Broker。已有 `EngineModelPort` 是后续消费接线的复用入口，不是用户已经可以发布模型 Provider 的证明；下一张模型卡须基于新入口重新核对最短接线，不按旧转换器草案机械追加接口。

**本批停止条件已达到：** 完成安全同步、冲突及语义整合、相关编译/定向回归与文档记录；不顺带新增 Rust 后端、另一套模型执行器或完整 Runtime 插件系统。合并后 App 编译通过，插件消费者 39 项、Broker 32 项、插件恢复 3 项、冷启动 Skill 产品测试 1 项、UI 定向交互 14 项通过，UI 类型/桌面边界/i18n 与 canonical 生成物检查通过；详细命令和证据边界见实施台账 §2.38，不代表全仓或浏览器端到端验收。后续仍以 §27.14 的一张真实用户功能卡为单位推进。当前未交付用户 JS 模型替换、完整 UI Shell 或任意主机部件替换，不能宣布整个平台完工。

### 27.16 JS 模型功能卡：新基线下的接线决策（2026-09-15）

本次继续 §27.14 的模型卡，限定核对真实消费者、实现选择和连接授权。**本节是实施决策，不是模型插件已经可用，也不新增 Rust 插件后端。** 新代码基线改变了可复用的入口，但尚未消除以下三个实际缺口：

1. `EngineSessionHost::open_model_port` 已将 `EngineModelPort` 绑定到真实 turn receipt 和 `EngineTurnJournal`；Nomi 的生产构造仍调用 `AgentBootstrap::new`，未向它注入这个端口。不能将“新引擎已经走 Broker”外推到 Nomi，也不能仅传一份 `ChatModelInput` 就绕过调用事实校验。
2. Nomi 的 `resolve_provider_fields_at_revision` 为现有内置协议解析连接和认证，再构造原生 provider 配置。它不是适合直接发给 JS 的公开配置：其中含认证材料。`EngineModelPort` 本身也不是安全沙箱；安全来自宿主的真实调用准入、连接解析和执行实现。
3. Product 已有受管增量调用，但 `invoke_agent_capability_inner` 仍以 `CAPABILITY_RESOURCE_BINDING_UNAVAILABLE` 拒绝带资源需求的 capability。声明 credential slot、已有流式队列，均不能证明已经有模型连接授权或受管 HTTP 流。不能通过去掉这个检查、把 key 写进 payload/config，或额外起一个本地代理服务来补洞。

#### 接线原则

- **模型配置与实现选择分开。** 原模型路由继续说明调用哪个 provider/model/connection；沿现有 Role/Provider 体系表达“使用哪个模型实现”，并进入同一 Compiler 和精确 Snapshot。首卡只处理 Chat。内置和用户实现各有自己的 capability ID，不覆盖内置 ID，也不同时新增一个平行的 `model_plugin_id` 选择体系。Product 模型 Role 的发布、目录投影、冻结和消费仍需实现，不能假设已有通用 Role 就已覆盖此来源。
- **执行入口只保留一个。** 以现有 `EngineModelPort` 为宿主模型入口；Nomi 侧沿既有 `LlmProvider` 接缝适配，不增加新的模型注册表、Session owner 或重试器。绑定来自本次真实 turn receipt，而不是调用时查询“当前最新回合”。同一回合内每次模型采样分别登记和认领操作，不复用上一采样的 causality。选中插件后，原内置 provider 不得作为静默回退路径继续执行。
- **连接权限属于宿主。** JS 只取得获准的请求数据和绑定到本次调用的连接能力，密钥解析/认证头注入仍由原连接与传输层完成。固定连接的目标、认证方式及撤销规则不能由插件改写；插件不能把凭据带到任意 URL。沿既有 Service/资源调用机制承载双向流与取消，不另建临时 HTTP 代理、凭据数据库或通用网络平台。这里是待实现的授权边界，不是当前已存在的接口保证。
- **模型完成与调用成功都要成立。** 文本增量可以即时交付；可执行的完整工具调用与成功终态须经过完整调用结果校验。Service 失败、非法事件或缺失终态，不能因先收到 `Completed` 而变成成功。Nomi 已有工具授权、参数验证及回合结算继续是权威；部分输出后的失败不自动重放。

#### 后续只执行三个相依切片，不继续增加纯前置接口

| 切片 | 实际实施范围 | 停止条件 |
|---|---|---|
| M1：Nomi 接入已有宿主模型入口 | 在 `ai-agent`/App 原构造和回合入口绑定真实调用事实；复用既有模型数据合同和 `EngineModelPort`。处理 Nomi 混合 user/tool 消息顺序、工具参数增量与结构化预览、推理历史及终态；不支持的语义明确拒绝，不能静默丢弃 | 至少一条真实 Nomi 产品主循环经现有宿主入口完成增量输出、工具往返和取消；原内置默认行为有回归。不是只有一个可注入但无人使用的 adapter |
| M2：模型 Role 与受管连接接入原 Product Service | 在同一目录/Compiler 中选择并冻结实现；补模型连接所需的最小资源消费与流式传输，复用原连接、认证和单次传输实现。先落实一类真实远程连接，不扩成全协议或通用网络插件平台 | 宿主密钥不进入 JS；跨会话/旧回合/换目标/停用/连接变更均被拒绝；同一请求的背压、取消和释放形成闭环。资源检查不能用豁免替代 |
| M3：完成这一张用户功能卡 | 真实源码发布→模型 Role 候选→选择/保存→新 Nomi 会话→真实 Node→受控远程模型；复用原用户选择入口，不扩 UI Shell | 旧会话冻结、新选择生效、首次增量早于结束、实际工具执行、部分失败不重试和取消回收均有产品证据，随后结束本卡 |

M1/M2 是同一功能卡的中间状态，不分别宣传为“用户模型插件已交付”。首卡不强制迁移独立 one-shot、所有压缩/视觉辅助调用或其他引擎，但实现时必须列出实际覆盖的调用点；尤其不能因为向 bootstrap 注入一个 provider，就无意把辅助调用也纳入一个没有对应真实调用事实的执行域。相关路径若随注入一起受影响，须同步接线验收，或调整注入位置。

这三处缺口都可以通过 Rust 宿主与现有 JS runtime 的正常接缝解决，没有证据要求增加 Rust 插件系统。当前不报无依据的剩余总人日或完成比例；M1 的真实产品链通过后，再按 M2 的连接范围估算余项。本次只更新这一局部决策，验证/未实施范围见实施台账 §2.39，不扩展其他 29 项环节。

### 27.17 按最新授权加入 Rust 原生后端（2026-09-15，实际实施）

本节以用户最新的“更新代码，并把 Rust 插件一起开发”为准，覆盖 §27.13～§27.16 中“本期不增加 Rust 插件”的排期限制；历史结论保留以便追溯。它不把整个全栈插件平台的未完成项改记为完成。

#### 代码更新

已执行 fetch、工作区备份、`git pull --ff-only` 和本地改动恢复：分支 `rf/agent-capability-platform-v2` 从 `2ff029512` 更新到 `e617feb2b`，吸收 7 个上游提交、35 个文件变更。同步前已有大量用户/前序工作，全部保留；没有重置工作树、提交或推送。保留 `refs/backup/plugin-rust-before-pull-20260915`、`refs/backup/plugin-rust-worktree-20260915` 与对应 stash，恢复无冲突。

#### 已实施的架构，不增加第二套平台

Rust 与 JS 是**同一 Product Service 的并列执行后端**：JS 使用选定 Node 的运行时租约，Rust 使用 Release 内的目标平台可执行文件。Rust 后端没有安装/探测 Rust 编译器的 Runtime Manager，不在宿主执行第三方 Cargo/build.rs，不加载不稳定的 Rust 动态库 ABI。

| 已落地部分 | 实现与边界 |
|---|---|
| 制品与身份 | `PluginServiceExecution` 声明 Node/Native 和 native target；原生入口为 `service/plugin.exe` 或 `service/plugin`。沿用原 Release、manifest 摘要、文件目录、导入/分享和持久记录，不新建原生插件表 |
| 装载与生命周期 | 共用原 Service NDJSON actor、generation/release/run-key 握手、请求队列、超时、ACK 背压、取消及托管进程树。修复原生制品处于 Windows 长路径时 cwd 无法回退的问题 |
| Runtime 身份 | Native 指纹绑定 target + executable digest，不虚构 Node 版本；Native resolve/start/Test 不访问 Node authority，Node 候选切换验证不探测 Native Service |
| 向后兼容 | 旧 Node descriptor 的 `execution` 默认且不序列化，旧指纹 JSON 形状保留；同一合同校验器服务两种后端，删除平台侧重复的 Service descriptor 校验逻辑 |
| Rust SDK | 新增 `nomifun-plugin-sdk` 和真实 echo 可执行示例；支持请求、流式事件确认、调用内 KV/数据库 IPC、取消及 stop。有限写队列由单一写任务消费，避免取消造成半帧破坏 |
| 发布与 Agent 消费 | 原生制品通过同一 prebuilt import → Test receipt → Publish → Enable → Catalog → Agent capability 调用；保持原有 action、release 和目录身份检查。可执行示例声明独立 capability ID，不覆盖内置 ID |
| 开发者闭环 | 增加 `package_native_echo` 打包示例，使用已有 Release/Share APIs；开发指南提供编译、打包、导入、启用和显式真实进程测试命令 |

**安全语义必须直说：原生进程不是沙箱。** `NOMIFUN_ALLOW_NATIVE_PLUGINS=1` 必须由宿主显式启用，默认拒绝。子进程不默认继承宿主环境密钥，但拥有本机账户权限，仍能自行访问 OS；当前没有按发布者签名授信、每插件 OS 沙箱或宿主内部对象的跨进程共享。capability 授权保护的是宿主调用入口，不是恶意原生程序的全部行为。

Test Host 的原有含义没有改变：验证启动不等于验证所有动作。包含 contributions 时仍返回 `NeedsTestInput`，发布须显式确认；本批端到端验证另外执行真实 Agent action，不能把警告改成假 `Passed`。

#### 验收和交付口径

实际命令、通过数和未验证项统一见实施台账 §2.40，开发者操作见 [`docs/plugins/native-rust-service.zh.md`](../plugins/native-rust-service.zh.md)。本批 Windows x64 真实二进制覆盖进程协议和 Product 调用；Linux/macOS/arm64 的 target 枚举不等于对应平台已验收。

**这批交付 Rust Service 后端，不宣称“任意 Agent 主机部件均已替换”。** 尚缺真实消费者/受管连接的模型 Provider、会话资源和初始化、完整 Shell/整段主循环/调度等功能，仍按 §27.2 的 29 项台账与 §27.16 的功能卡推进。增加 Rust 不会自动补上组件合同或权限边界；以后原生密集实现可以直接用此后端，不必先包一层只为等待 Rust 的 JS 适配器。

后续顺序保持短链路：先完成现有模型功能卡的真实消费者与受管连接，再按领域逐项实现可替换的组件和 UI 接缝；每张卡均复用同一 Catalog/Compiler/Service，不新建语言专属选择数组、凭据桥或第二 executor。本批不据新增 SDK 代码量推算平台总完成百分比。

## 附录 A. 按目标要求完成统一的实施基线

本节是后续代码实施的固定边界。它不是兼容迁移方案，而是产品重构阶段的
“破而后立”方案。

### A.1 Canonical 领域实体

活动代码只允许使用以下产品实体：

- `PluginProductId`：用户看到和管理的插件产品身份；
- `PluginProjectId`：插件源码/工作区身份，安装型和发布型插件共用；
- `PluginReleaseId` / `PluginReleaseRef`：不可变发布物身份；
- `PluginMountId`：已安装 N1 package 的挂载身份，与产品身份明确区分；
- `PluginSurfaceSessionId`：插件 UI surface 会话身份；
- `PluginService`、`PluginSurface`、`PluginTool`：插件运行角色，而不是产品类别。

`PluginProduct` 可以同时拥有 Tool、Service、Surface、ResourceProvider 等贡献。
不能再用互斥的 `UiOnly` / `Service` 产品种类决定能力准入；产品能力应由发布
Manifest 的 contributions 和 role/profile 派生。

### A.2 Canonical Agent 模型

2026-09-14 按当前 canonical `ResolvedSnapshotContent` 修正本附录早期草案：能力记录已经统一为 `enabled_capabilities`，不要重新引入 `initial_capabilities` / `on_demand_capabilities` 两份能力数组。以下仅列能力与锁相关字段，不是完整 schema：

```text
enabled_capabilities
capability_allowlist
skill_locks
mcp_tool_locks
resolved_role_providers
```

所有能力（N1 package、已发布 PluginProduct、平台内置能力、MCP 映射）都进入
同一个 `ResolvedCapability` 类型。发布型插件的 active release、release digest、
publication epoch、catalog digest 作为统一 provenance/execution profile 的字段，
不能再以 `ResolvedMiniAppCapability` 另开一套数组。

`capability_allowlist` 是身份集合，不是第二份能力描述；运行期 active set 属于会话状态。后续实现依赖与消费用途的区分须按 §14.5 在同一 canonical 计划中演进，不以恢复旧数组或增加各语言专属数组实现。资源实例和租约仍属于目标/会话绑定，不写进 Preset 来简化表面结构。

### A.3 Canonical 数据根

运行时表、repository、外键和 identity 统一为 `plugin_*`。目标包括：

```text
plugin_products
plugin_projects
plugin_releases
plugin_catalog_publications
plugin_surface_sessions
plugin_service_test_receipts
plugin_source_mutation_*
```

开发阶段按破坏式切换处理：不保留 `/api/miniapps`、旧字段别名、旧 CLI 或旧
协议别名；已有开发数据库可以重建。若未来需要保留真实用户数据，必须另行设计
一次性数据迁移，不能把兼容字段重新带回活动领域模型。

### A.4 实施顺序和闸门

1. **Contracts**：先完成 ID、provenance、Snapshot 和 generated schema；闸门是
   `ResolvedMiniAppCapability`、`MiniAppActiveRelease`、`miniapp_id` 不再出现在
   活动 Agent contract。
2. **Plugin platform / DB**：重命名并合并 M1 release、service、surface、storage
   聚合；闸门是运行时只依赖 `PluginProduct`/`PluginRelease` 和 `plugin_*` 表。
3. **App / Session**：合并 catalog sink、runtime state、route state 和 tool
   materialization；闸门是 Session 只有一个 Plugin capability materialization 入口。
4. **UI / API**：统一 ID parser、resource kind、bridge 协议、DTO 和用户文案；
   闸门是用户可见入口及前端活动类型不再出现 MiniApp。
5. **Docs / tests / generated fixtures**：重生成合同、预置 Agent、验收脚本和
   架构文档；闸门是 residual scan 只允许历史记录和明确的兼容 allowlist。
6. **最终验证**：执行 Rust workspace check、相关集成测试、前端 typecheck/build，
   并对活动代码运行大小写不敏感的 `MiniApp|miniapp|MINIAPP` 扫描。

任何阶段如果重新引入 `MiniApp` 兼容 alias、第二套 capability 数组或独立
MiniApp resource kind，都视为没有完成统一，而不是“暂时过渡”。
