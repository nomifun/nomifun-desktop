# 插件发布就绪台账：尽快上线，尽可能可替换

日期：2026-09-15。状态：**已有 Agent 页面开关移除及 Windows 定向回归、聚合检查、desktop check 已完成，见 §2.7；正式制品与双平台上线验收未完成。** 此前 Skill、Service/UI 前置清理及装配简化证据见 §2.6，原生删除及更早回归仅作为对应历史证据。

基线说明：原生删除和架构收敛已随 `70c28b5de86f820a535bdcca48191244196f2e2f` 提交并推送；本轮在其上筛选 hooks/UI 计划并移除已有 Agent 页面的实验开关，新 hooks 尚未实现。下文未提交工作区记录属于当时验证事实。首发为 **Windows x64 + macOS Apple Silicon（arm64）**；不包含 Intel Mac、Windows arm64、Linux 或 universal 制品。页面开放的最新验证单独记录，不以 §2.6 旧证据代替。

发布方式已确认：开源大重构直接正式上线，不设小范围候选分发、灰度或反馈等待阶段。下文候选制品只用于必要技术验收，不新增用户试用流程；复用已有发布方式。本轮尚未执行制品发布。

兼容策略已确认：优先最小工作量，不追加历史版本适配或升级/回滚矩阵；保留已有迁移与测试，不为移除兼容代码再做重构。P0-4 收敛为新安装、重启持久化与不兼容旧库的数据保护，不再要求最近正式版本必须无缝升级。下文旧迁移测试仅保留历史证据，不构成新的全量兼容承诺。

Rust native 删除决定不变：原生插件后端、Rust 作者 SDK、启用入口、示例及专属打包/测试路径已移除，不保留实验入口。本轮另外删除无生产消费者的 Service 流式分支；保留 Node 普通调用、存储 IPC、取消/生命周期、生产模型合同及正常产品路径。

当前范围入口为[架构收敛记录](2026-09-15-plugin-architecture-convergence.zh.md)：不再等待历史 A/B 选择。用户要求开发的能力开放可用，因此已有 Agent 页面移除实验开关，内置仍默认、插件由用户选择；专用事件协议和重型参考页不恢复。

最新产品取舍：外部插件模型 Provider、安装型/热加载 Agent Runtime、Shell 和其他 hook/UI 扩展候选移出计划。仅保留新增 before_tool/after_tool 及已有页面内业务输入/结果展示，详见[确定清单 §0.1](2026-09-15-agent-plugin-remaining-work-and-decisions.zh.md)。这些新增项尚未实现，不能写入本次支持声明；一旦开发完成，必须具有普通用户可发现和使用的完整链路，不设置长期默认关闭的实验开关。

本文覆盖 [能力评估](2026-09-13-agent-plugin-capability-assessment.zh.md) 与 [历史实施台账](2026-09-14-agent-plugin-openness-implementation.zh.md) 中旧 JS-only、任意/全栈替换和“下一批”排期；非 native 历史实现及测试记录保留，native 详细方案收缩为曾实现后按范围收敛删除的历史记录。29 项是长期能力评估，不是上线欠账，不要求清零，也不承诺任意替换。

此前代码收敛和本机定向验证的证据保留在 §2.1～2.4；它们不证明最新 native 删除后的候选通过。Windows 删除专项回归已通过，详见 §2.5，工作区测试也不能代替已安装候选包验收。

## 1. 本次范围

| 定位 | 范围与边界 |
|---|---|
| 正式默认 | 内置 Agent、内置模型链路、内置 UI；用户未选择时不自动加载第三方插件页面 |
| 已支持、按证据验收 | Tool、Context、discovery、`before_model`、只读 Skill；限定已接线来源、阶段、权限、冻结身份及真实消费者，不扩大为全领域替换或可执行 Skill |
| 删除项（代码已完成） | 原生插件后端、Rust 作者 SDK、启用入口及专属示例/打包/测试路径；不再列为可启用实验，Windows 删除专项回归已通过，详见 §2.5 |
| 本轮收敛删除 | 旧 Skill loader/resolver、Service 事件流/emit、UI 专用事件投影/订阅、参考页自动草稿恢复及宿主隐藏消费者重新绑定；结果见 §2.6 |
| 页面可选能力 | 移除实验开关，普通用户可创建/发布/选择/记住偏好/切回；内置 UI 仍默认，不宣称全部附件/审批交互、自动草稿恢复、多视图或 Shell 替换 |
| 取消开发 | 外部插件模型 Provider、安装型/热加载 Agent Runtime；保留内置模型与源码引擎注册/编译选择，不重复建设 |
| 待开发 | 新 hooks 仅 before_tool/after_tool；页面增强仅业务输入/结果展示；未完成不计为已支持 |
| 不做 | 应用 Shell、其他通用 hook/UI 扩展；保留现有内置正常功能，不按历史全量目标扩张 |

删除范围以上表及架构收敛记录为准，不删除 Rust 宿主、内置实现、Node 普通 Service 或必要执行保护，不重做平台。长期项若发现影响正式范围的安全/可用性问题，应登记具体 P0 缺陷，不把整个长期项升级成上线前必做。

页面不再依赖实验环境变量、systemInfo availability 字段或前端 context；删除开关不是返回恒 true 的兼容层。仍由原 owner、精确 release、Surface/Session 授权和停用状态决定访问资格，未选插件时内置页默认。普通 Surface 和同一插件的 Tool 不因页面选择改变权限；不用开放入口代替真实授权。

§2.1～2.6 中“默认关闭”“显式 opt-in”及其测试是历史阶段证据，已被当前公开可选决定替代；不再作为开发/交接指令。此前测试通过不自动证明本轮开关移除通过。

## 2. P0 六项发布台账

六项责任均为主代理。状态仅在获得本轮实际证据后更新；每项填写代码基线/工作区状态、日期、OS/架构、具体命令或操作、退出码/断言、日志位置和未覆盖边界。失败及重跑分别记录；未执行、跳过、缺环境不得记为通过。

| ID | 发布项 | 本次最小验收要求 | 状态 | 本轮结果 / 证据 |
|---|---|---|---|---|
| P0-1 | 范围与删除边界 | 核对正式默认 Agent/模型/UI；原生插件后端/SDK 与启用入口已移除；已有页面正常开放且有限能力清楚，未实现能力不误导为正式支持 | 代码与 Windows 专项通过，制品待验 | 原生删除证据见 §2.5；本轮页面开关移除、正常选择与授权保护见 §2.7 |
| P0-2 | 产品流程 | 在内置默认链路核对安装/导入、发布、选择、保存、重启后使用；Tool/Context/discovery/before_model/只读 Skill 各以对应真实消费者及已支持形态验收，不以单一示例替代全部证据 | 删除后 Product 回归通过，制品流程待验 | 此前 Product 4 项、Agent 消费者 41 项、UI 产品 3 项通过；删除后最终回归见 §2.5；安装包重启后流程与真实模型调用尚未验证 |
| P0-3 | 故障安全 | 核对权限/冻结身份、撤下或停用、漂移、超时、取消、崩溃、非法输出和回收；禁止静默换实现或扩大授权；native 删除不破坏 Node/共享 Service/正常产品路径 | 删除后故障回归通过，安装后待验 | 删除后 Node Service 16 项、storage IPC 3 项及 Product 4 项通过（§2.5）；不替代安装后进程回收验收 |
| P0-4 | 最小数据安全与兼容边界 | 新安装/显式新数据目录、保存重启和不可用插件诊断正常；不兼容旧库明确拒绝且保留数据，说明现有新目录配置方式；不要求历史升级/回滚矩阵 | 已有迁移专项证据保留，安装后最小路径待验 | 上游前缀新增迁移 3 项、published-main 5 项、displaced Agent 3 项仅为已有证据；不扩大为无缝升级承诺，未操作用户实际数据 |
| P0-5 | 发布构建 | 明确本次实际发布 OS/架构及制品；对应正式构建、安装/启动/卸载和必要签名检查，核对路径与运行时依赖；其他平台未验证不得宣称通过 | 聚合检查与 desktop check 通过，发布产物待验 | 最新 `bun run check` 与 Windows `cargo check -p nomifun-desktop` 已通过，见 §2.7；不替代正式制品构建和安装验收 |
| P0-6 | 文档 | 用户入口、作者说明、已支持/未实现/取消范围、删除决定、已知限制及数据恢复说明与实际制品一致；交付跨 OS TODO / prompt | 文档已整理，制品一致性待最终复核 | 同步页面正常开放及 hooks/UI 确定清单，保留历史验证事实；不将文档更新记为制品验收通过 |

放行规则：主代理在声明的发布目标上取得六项适用证据并处理实际阻塞后，才能写发布结论。未覆盖目标从发布声明中明确排除或继续待验证，不得用 Windows x64 结果替代其他平台。缺少非发布目标的环境不应自动阻塞无关范围。

历史证据索引（只供选择仍适用的定向验证）：实施台账 §2～2.1 Tool/Context、§2.8～2.11/2.16～2.18 Skill/Context、§2.27 discovery、§2.28～2.29 before_model、§2.19～2.22/2.30～2.32 页面实验、§2.35～2.37 共享流式与模型合同。§2.40 仅保留原生插件曾实现后按范围收敛删除的记录，不再提供执行指令。历史证据不自动证明当前工作区或发布制品已通过。

### 2.1 此前必要修复与升级边界（历史记录）

- 服务端以 `NOMIFUN_ALLOW_EXPERIMENTAL_AGENT_UI=1` 准入 Agent 页面候选、非空偏好保存、模板创建、Surface 会话授权、bridge 和事件转发；关闭和清除仍可执行。普通 Surface/Tool 保留原事实与目录摘要，开关不改会话执行配置。
- 前端在空候选时隐藏会话页的实验选择/模板入口，保留已保存不可用偏好的提示及明确清除；切回内置不发新 turn，不隐式重试副作用。
- 060/080 恢复为上游 `e617feb2b` 原始 SQL，字段与索引追加于 104；原未发布 097 顺延为 105，避免破坏上游到 103 的严格迁移前缀。
- 修复既有 published-main 059/060 认证把迁移编号当数组下标的问题：按真实公共版本集合（含 027 空缺）及精确 checksum 验证，仍只重定位已知 059→073、060→074。坏前缀/失败记录/未知 checksum 不改 schema 或 ledger，失败仍事务回滚。
- 不自动兼容曾运行修改版 060/080 或旧未发布 097 的本地开发库。遇到该来源保持拒绝，保留原数据；不得删除数据库、伪改 checksum 或悄悄新建空库。需要该库时另行基于备份和精确来源制定恢复方案。
- 追加迁移不承诺旧二进制可直接读取新库。正式升级前使用已有数据备份流程，回退须使用与旧版本匹配的完整备份，不只替换程序；本轮未操作用户实际数据目录。
- 补齐 `test:agent-view-template` 的脚本目录登记，不新建发布流水线。
- 分离 Product 调用准入与模型 FunctionTool 展示校验：精确 `before_model` / discovery Hidden action 可被既有消费者调用，仍检查冻结授权、allowlist、资源和回执，不向模型新增可见工具。
- 修复 Hidden 消费者先于 effect scope 装配时遗漏 retained-task invoker 的问题；两种安装顺序均使用同一任务保留、取消关闭派发与 unknown 关闭会话机制，不新增执行协议或调度器。
- `before_model` 输出仅以摘要、字节数记录回执，不保存或预览临时 system patch，避免通过恢复上下文重新注入下一轮。回执本身、pending 拒绝与重放保护仍保留。
- 超时产品用例按现有安全边界验收：未知结果禁止进入下一模型轮，不能期待继续输出成功文本；撤下测试使用超时前已冻结的另一会话，不能清除 pending 或强行复用未知效果会话来使测试通过。
- 生命周期限制：调用方超时不等于被调用任务已退出。挂起 Service 仍由宿主保留至现有 30 秒 watchdog / 有界清理；未知效果保留 pending，禁止自动重放，不承诺立即恢复原会话。本轮没有改短/放宽生产超时，没有清空历史效果账本。

### 2.2 删除决定前的验证记录（Windows x64，历史证据）

基线为 `e617feb2b39229e74d83fcb2f684fe76c66b5d25` 加保留的未提交工作区改动，不能写成该提交本身已包含本轮修复。日志为仓库根目录的 `.tmp-release-*` 本地文件，不纳入发布包。

| 检查 | 当前结果与证据边界 |
|---|---|
| `cargo test -p nomifun-agent-contracts --lib` | 100 通过，`.tmp-release-contracts.*`；既有生成器 `target/debug/agent-v2-contract.exe check` 也通过，本轮未改变 canonical 合同 |
| Agent 实际消费者 | `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=1`，41 通过，`.tmp-release-consumers-final.*`；含 Tool、Context、只读 Skill、发现策略、before_model，以及新增 Hidden 调用两种装配顺序下的取消保留和 unknown 拒绝 |
| Product → Node → Nomi | `cargo test -p nomifun-app --test plugin_product_discovery --no-default-features -- --test-threads=1`，4 通过，`.tmp-release-product-final2.*`；含正常连续轮次、回执不存 patch、排序冻结、发现策略、撤下、超时阻断下一模型轮与 Service stream；模型端为 wiremock，不是真实服务 |
| `cargo test -p nomifun-db --lib plugin_ui_upgrade_tests -- --test-threads=1` | 3 通过；published-main 修正后再次完整通过，最终日志 `.tmp-release-db-upgrade-final.*` |
| `cargo test -p nomifun-db --test published_main_migration_upgrade --test displaced_agent_preset_migration -- --test-threads=1` | 5＋3 通过，`.tmp-release-db-lineage.*`；含事务回滚、未知/缺失/失败 ledger 拒绝 |
| UI 交互、结构与流转发 | 5 个交互文件 44 项、3 个结构/转发文件 11 项通过；交互日志 `.tmp-release-ui-final.*`。新增测试首轮有等待顺序问题，改为等待内置内容实际显示再断言，不放宽业务断言 |
| 参考模板和 Product SDK | `node --test ui/src/renderer/pages/agentSession/AgentSessionTemplate.node.ts crates/backend/nomifun-plugin-platform/tests/product_sdk.test.mjs`，29 通过，`.tmp-release-template.*`；最初前台调用被工具时限终止，后台完整重跑通过 |
| UI 产品与启动 | 当前编译出的 `plugin_ui_sessions-5480a3751c535c1f.exe --test-threads=1` 3 项（内部隔离子进程不重复计数）、`startup_smoke-c4ffc00f3e08be29.exe --test-threads=1` 4 项通过；cwd 为 App crate，日志 `.tmp-release-ui-product.*` / `.tmp-release-startup.*`。不是已安装 WebView E2E |
| 测试入口注册 | `cargo test -p nomifun-app --test content_e2e_suite every_top_level_integration_test_is_registered --no-default-features -- --test-threads=1`，1 项通过，`.tmp-release-registration.*`；明确登记 4 个由父测试包含的插件子模块，并检查父目标与模块引用，不放过未注册文件 |
| 原生插件历史验证（已退出支持范围） | 删除决定前 Windows 原生进程 2 项和 application 3 项曾通过，日志 `.tmp-release-service.*`；仅保留历史事实，不代表现行支持，不作为删除后或跨平台验收证据 |
| Node Service / storage IPC | `cargo test -p nomifun-plugin-platform --test service_process --test service_storage_ipc -- --test-threads=1`，16＋3 通过，`.tmp-release-service-final.*`；首次流测试将 Node 冷启动包含在两秒事件断言内而失败，改为先完成宿主 warmup 再测流事件，协议超时和断言不放宽 |
| 前端及 Windows 宿主 | `bun run build:ui` 和最终修复后的 `cargo check -p nomifun-desktop` 通过，`.tmp-release-ui-build.*` / `.tmp-release-desktop-final.*`；有既有构建告警，不等于生成/安装正式包 |
| 发布静态检查 | 前轮 `bun run check` 在词汇守卫的 109 处检查失败（107 处文本在 HEAD 已存在），历史日志 `.tmp-release-vocabulary-final2.*` 保留。后续按用户授权清理现行命名、精确保护历史编码，最新 `bun run check` 全链通过，见 §2.4；不再是当前阻断 |
| 桌面 UI 边界 | 最后单独重跑 `bun run check:desktop-ui-boundary` 通过（1944 个 renderer source，最小 880×600）；未增加移动端目标 |

失败/重跑不隐去：Product 首轮 3 项失败；修正 Hidden 准入后仍有 2 项失败，进一步确定为临时 patch 经回执重入上下文，以及旧测试期待超时后继续模型调用。修复回执与任务保留后，正常连续轮次已通过，但挂起 Service 的 15/20 秒观察窗口不足以覆盖既有 30 秒 watchdog；只将这两项故障用例观察窗口改为 45 秒，生产超时未改，最终 4 项通过。原始及中间日志保留为 `.tmp-release-app-product2.*`、`.tmp-product-owner-fix.*`、`.tmp-release-product-diagnostic.*`、`.tmp-release-product-final.*`。

其他异常：首轮 App 编译的模块引用错误已修复；首次 DB `--lib` 启动 414 项并行用例引起资源争用，主动终止，不计通过，后续仅串行跑所需专项。尚缺 `NOMIFUN_LIVE_STEPFUN_API_KEY`，未执行真实付费模型测试、未读取用户凭据。当前安装包验收脚本要求干净 HEAD 与候选制品绑定；工作区仍含大量既有未提交改动，不绕过该检查，不自动提交、安装覆盖用户程序或上传发布。

### 2.3 收尾后的剩余上线 TODO（不再扩架构）

1. 首发范围已确认为 Windows x64 + macOS arm64，直接正式上线、不设灰度阶段；此前代码候选已提交并推送为 `48affddb7`。最新 native 后端/SDK 代码删除已完成，Windows 删除专项回归已通过，详见 §2.5；构建绑定删除后的实际候选，不把旧证据当作新改动或制品通过。
2. 在隔离测试数据目录构建/安装该候选，补验导入或安装、发布选择、保存重启、停用切回内置、新目录启动、不兼容旧库的明确拒绝和数据保留、卸载及适用签名；不覆盖真实用户数据，不另做历史升级/回滚矩阵。本轮未给出制品级通过结论。
3. 按 D4 复用现有专项证据，在最先具备授权模型配置的环境补一次默认 Agent 响应和无副作用插件 Tool 的实模检查，不新建测试平台或多厂商矩阵。本进程专用凭据未配置，当前优先交接已有配置的产品环境；实模仍待验，不能由模拟结果代替。步骤见[决策文档 §8.1](2026-09-15-agent-plugin-remaining-work-and-decisions.zh.md)。macOS arm64 按 §3～4 接续；首发之外架构继续延期。

不再排入本次剩余 TODO：其他新增插件领域、完整插件 UI、应用 Shell、原生沙箱或跨平台通用重构。外部模型插件与安装型/热加载引擎已取消，源码引擎机制保留；更多 hooks 仅做独立设计，不将设计当作 release 实现。

### 2.4 候选工作区整理与词汇检查收口（2026-09-15）

用户授权处理 109 处词汇失败并执行聚合检查后，已完成以下整理：

- **83 处现行命名清理**：Catalog 方法、宿主装配字段、类型、模块路径、测试夹具和说明统一为 Plugin Product。`engine_miniapp_tools.rs` 改名为 `engine_plugin_product_tools.rs`，调用点及用于构建身份的 `include_str!` 同步。没有恢复旧 API 别名，没有改执行权限或重新设计插件后端。
- **16 处历史删除清单引用**：守卫识别已归档的 `contracts/historical/agent-v2/deletion/*.json`，与原删除清单语义一致；不豁免其他 historical schema 或该目录中的运行时代码。
- **10 行精确持久化兼容**：保留回执域、调用 ID 前缀、持久工具名和拒绝编码、102/103 SQL 及迁移测试的旧字面值。守卫限定文件与完整行，验证每行恰好出现一次；追加新旧名称、复制到其他文件或新增现行类型均不受豁免。这些断言随每次词汇检查运行。102/103 历史 SQL 无改动，不伪改 checksum。
- `.gitignore` 仅补充仓库根目录 `.tmp-*.out` / `.err` / `.txt` 验证日志模式。日志保留本地，候选源码、SDK、测试和文档不因此被隐藏；没有执行删除、stash、重置、自动暂存或提交。

候选基线仍为分支 `rf/agent-capability-platform-v2`、HEAD `e617feb2b39229e74d83fcb2f684fe76c66b5d25` 加工作区。整理时有 180 条已跟踪变更、97 个未跟踪源码/文档路径，暂存区为空；571 个临时验证记录已由忽略规则排除。该统计包括先前各轮变更，不代表本次新增了这么多文件，也不等于已形成干净 release commit。提交时应保留模块改名对应的删除与新增两端，不能只提交已跟踪文件而漏掉 SDK 等新增源码。

本次 Windows x64 验证：

| 命令 | 结果 / 日志 |
|---|---|
| `bun run check` | 全链通过：typecheck、desktop UI、i18n、theme、icons、dead CSS、Windows installer contract、Creative Studio retirement、process/browser/automation 边界、Agent vocabulary 和 help；`.tmp-candidate-check.*` |
| `cargo check -p nomifun-desktop` | 通过；`.tmp-candidate-desktop.*`，保留既有 28 项 App 告警，不是安装包构建 |
| `cargo test -p nomifun-agent-control-plane --lib kernel_catalog -- --test-threads=1` | 2 项通过；`.tmp-candidate-catalog.*` |
| `cargo test -p nomifun-db --lib git_extension_preserves_the_plugin_product_persistent_owner_codec -- --test-threads=1` | 1 项通过，旧持久化域在升级后保留；`.tmp-candidate-codec.*` |
| `git diff --check` | 通过；暂存区仍为空 |

本次没有新增跨 OS 分支；共享 Rust 符号改名后的其他目标构建仍按 §3～4 接续验证。聚合静态检查通过不代替正式制品安装、真实模型调用或跨平台验收。

### 2.5 原生插件删除后的验证进展（2026-09-15）

本节仅记录原生删除阶段；其中“JS 流式保留”和测试入口以当时为准，本轮后续收敛覆盖它们，见 §2.6。

基线为 `48affddb7f1844f5da69573bd0dc061b6e14541c` 加本次未提交删除改动，环境为 Windows x64。原生执行分支、Rust SDK、打包入口、专属示例及测试已删除，发布合同恢复为 Node-only；不新增迁移或兼容转换，不删除用户数据。JS 流式、取消、存储 IPC、权限和 UI 开关策略保留。下表是删除后的实际检查，不与 §2.2/§2.4 旧候选证据混用。

| 检查 | 状态 / 证据边界 |
|---|---|
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write` / `check` | 均通过，生成 schema / 摘要同步；`.tmp-remove-native-contracts.*` |
| `cargo test -p nomifun-agent-contracts --lib` | 100 项通过、无忽略；同上日志。含旧原生格式拒绝及 Node JSON 格式保持测试 |
| `bun run check` | 全链通过，退出码 0；`.tmp-remove-native-check.*` |
| `cargo test -p nomifun-plugin-platform --lib --test service_process --test service_storage_ipc --test service_application --test m1_build_tests -- --test-threads=1` | 38 / 16 / 3 / 2 / 9 项全部通过、无忽略；含真实 Node、流式、取消、崩溃回收、长路径及存储 IPC；`.tmp-remove-native-regression.*` |
| `cargo test -p nomifun-db --test plugin_runtime_repository` | 23 项通过、无忽略；同上日志 |
| `cargo test -p nomifun-app --test plugin_product_discovery --no-default-features -- --test-threads=1` | 4 项通过、无忽略；真实 Node + 模拟模型服务，含 before_model、发现策略及流式权限/取消/停用边界；同上日志 |
| `cargo check -p nomifun-desktop` | 通过，退出码 0；保留已有 28 项 App 告警，不做无关清理；同上日志 |

上述 Rust 测试合计 195 项通过，合同生成检查、聚合静态检查和桌面编译检查均通过，`git diff --check` 通过。原生后端符号、启用开关、打包入口及已删指南的引用扫描无残留；JS 作者的 `nomifun-plugin-sdk.d.ts` 继续保留，与已删除 Rust SDK crate 无关。`Cargo.lock` 只移除该本地 SDK 包，没有升级依赖。未新增兼容层或操作用户实际数据；旧原生格式明确拒绝，不自动当作 JS 加载。本次未提交或推送。

首发仍为 Windows x64 + macOS arm64；正式制品、安装后流程、实模和 macOS 适用验收仍按 P0 / MAC 条目补证。历史原生插件测试不代表现行支持，不再列为必做验证。

### 2.6 架构收敛验证（2026-09-15，Windows x64）

本轮在 `48affddb7` 加既有原生插件删除工作区上实施；以下证据采集于提交、推送前，尚未发布。交接应记录包含本节及对应代码的实际提交。范围与真实支持矩阵见 [架构收敛记录](2026-09-15-plugin-architecture-convergence.zh.md)。第一/二/三批均已获用户批准，当前记录不再等待历史方案选择。

已改动：旧 Skill loader/resolver 删除；命令发现改用共享 loader 的 Commands 模式（仍校验整个制品完整性，不解码/装配无关资源）；Service 移除 emit/event ACK 分支，保留普通调用和取消；UI 撤掉专用会话事件链，默认关闭时从已有 systemInfo 引导信息直接选择内置 UI；参考页收缩为宿主 API 最小客户端；Session 宿主依赖单次绑定；具体 hook 验证留给 Nomi 消费者。

| 检查 | 当前结果 | 证据 / 未覆盖边界 |
|---|---|---|
| Plugin Service `--lib service` 与 process/application/storage 专项 | 29 项通过（16 + 8 + 2 + 3） | `%TEMP%/nomifun-service-removal-{lib,integration}.{out,err}.log`；包含取消、存储、普通返回及无 emit，不代替完整 Product 链 |
| Kernel `--lib middleware_order` 首次运行 | 3 通过、1 失败 | `.tmp-converge-kernel.{out,err}`；新测试把 Tool handler 留在 middleware 注册中，注册约束正确拒绝，修复测试夹具后重跑 |
| Kernel / 应用消费者边界重跑 | Kernel 专项 4 项通过，随后完整 54 项通过；应用专项 4 项通过 | `.tmp-converge-kernel-full.out`、`.tmp-compiler-boundary-app.{out,err}`；完整 Kernel 复用本轮 cargo 编译产物直接运行全部测试，无忽略 |
| Skill 共享 loader | 5 项通过 | `.tmp-converge-skill-loader.out`；复用本轮应用 lib 测试产物运行 `router::engine_skills::tests`，验证非法图片/资源限额与正文/制品身份独立处理 |
| Agent 消费者首次集成 | 39 通过、2 失败；修复测试后全部 41 项通过 | 首次 `.tmp-converge-consumer.{out,err}`，最终 `.tmp-converge-consumer-final.{out,err}`；旧测试把取消等同于立即丢弃调用，新断言验证保留、关闭派发、完成清理、不重试，未放宽生产保护；包含跨 Product/动态工具名字冲突和重复装配拒绝 |
| UI 交互与 wire | 55 项通过，371 条断言 | `%TEMP%/nomifun-ui-convergence-final.{out,err}`；页面关闭/启用/首个模板、缓存和普通 bridge |
| UI 启动与路由 | 6 项通过，22 条断言 | `bun test --cwd ui src/renderer/main.bootstrap.structure.test.ts src/renderer/pages/conversation/SessionList/SessionRouteSafety.structure.test.ts`；子任务工具输出，无独立日志文件 |
| 模板与 SDK | 12 项通过 | `bun run test:agent-view-template`；子任务工具输出，无独立日志文件 |
| `bun run check` | 全链通过 | `.tmp-converge-check.{out,err}`；包含 typecheck、desktop boundary、i18n、主题/图标/CSS、Windows installer、架构守卫、词汇守卫与 help |
| Product 真实 Node 调用链 | 4 项通过 | `.tmp-converge-product.{out,err}`；before_model、顺序冻结、发现策略、普通 Service 权限/取消/停用，无忽略 |
| 插件页面产品链 | 首次 2 通过、1 失败；修正后全部 3 项通过 | 首次 `.tmp-converge-product.{out,err}`，最终 `.tmp-converge-ui-product-final.{out,err}`；删除事件流和草稿恢复场景后 fixture 期望从精确 3 次同步为精确 1 次，重开不重复执行等断言保留 |
| `cargo check --locked --offline -p nomifun-desktop` | 通过 | `.tmp-converge-desktop.{out,err}`；已有 App 28 条告警保留，不做无关清理 |
| canonical 合同检查 | 通过 | `.tmp-converge-contracts.{out,err}`；`cargo run --locked --offline -p nomifun-agent-contracts --bin agent-v2-contract -- check`，并直接运行本轮产物核实退出码 0；没有新生成数据漂移 |

UI 55 项复现命令：`bun test --cwd ui src/renderer/pages/agentSession src/renderer/pages/agentSettings/AgentPageSettings.interaction.test.tsx src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.interaction.test.tsx src/renderer/pages/plugins/runtime/agentSessionBridge.test.ts src/common/adapter/ipcBridge.plugin-runtime-wire.test.ts`。

结果：本轮实施项已完成，独立计数的定向/集成测试共 213 项通过（不重复计入 Kernel 4 项专项重跑），没有忽略项；`bun run check`、Windows desktop check、合同检查与 `git diff --check` 通过。没有新增兼容迁移、依赖升级或执行发布；现有原生删除改动保留。首次失败与修正过程如上，不隐去失败记录。

正式制品安装、签名、真实模型与 macOS arm64 仍未验收。跨 OS 交接应同时阅读本轮架构收敛记录 §5：不恢复 Service 流式、UI 专用订阅或重型草稿恢复协议，不把旧历史测试名当作必须恢复的产品能力。

### 2.7 已有 Agent 页面正常开放（2026-09-15，Windows x64）

基线为 `70c28b5de86f820a535bdcca48191244196f2e2f` 加本轮未提交增量。删除后端实验准入模块、目录/模板/偏好/Session 入口开关、systemInfo availability 字段、前端 context 和 bootstrap 门控；不保留恒 true 兼容包装。普通用户可创建参考草稿、发布、选择、记住偏好及切回内置；不自动选择第三方，不改变执行配置。

保留并验证 owner、精确 release/capability、Surface/Session 授权、停用撤权、偏好版本冲突、取消和重开不重放。新增 `before_tool` / `after_tool` 与业务页面增强仅完成范围设计，未实现，不计入本节交付。

| 检查 | 本轮结果 | 证据 / 边界 |
|---|---|---|
| `cargo test -p nomifun-api-types --lib lifecycle::tests` | 11 通过 | `%TEMP%/nomifun-agent-ui-api-tests.{stdout,stderr}.log`；验证 systemInfo 不再输出实验字段 |
| `cargo test -p nomifun-app --test plugin_ui_sessions -- --test-threads=1` | 3 通过 | `%TEMP%/nomifun-agent-ui-sessions-tests.{stdout,stderr}.log`；包含 admission/binding 子模块，无 opt-in 的创建/发布/选择/持久化、真实 Node 发送/取消和越权/失效拒绝；模型使用 wiremock，不是实模 |
| UI 定向回归（命令如下） | 9 文件、58 测试、389 断言通过 | 本会话工具输出，无独立日志；覆盖页面、设置、启动、普通 bridge/wire 与 Surface，保留已保存选择解析先于内置页副作用 |
| `bun run test:agent-view-template` | 12 通过，无跳过 | 本会话工具输出；参考页与 SDK 的发送一次、取消、不明结果不重放、失效/销毁行为 |
| `bun run check` | 全链通过 | `.tmp-page-open-check.{out,err}`；包含 typecheck、桌面边界、i18n 和词汇守卫等 |
| `cargo check --locked --offline -p nomifun-desktop` | 通过 | `.tmp-page-open-desktop.{out,err}`；App 仍有 28 条非本次位置的编译警告，未扩大清理范围 |
| `git diff --check` 与开关引用检索 | 通过 | 生产代码不再存在实验开关/context/availability 字段；仅负向测试保留旧字段名以断言其不存在 |

UI 精确复现命令：

```text
bun test --cwd ui src/renderer/pages/agentSession src/renderer/pages/agentSettings/AgentPageSettings.interaction.test.tsx src/renderer/main.bootstrap.structure.test.ts src/common/adapter/ipcBridge.plugin-runtime-wire.test.ts src/renderer/pages/plugins/runtime/agentSessionBridge.test.ts src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.interaction.test.tsx
```

本轮独立定向测试合计 **84 项通过**，不重复计入重跑。UI 首轮 32 项和第一次扩展 58 项均通过，但有 React `act` 警告；仅修正测试初始化/保存的异步等待后重跑，最终 58 项无该警告，未放宽生产断言。前端单独 typecheck、desktop-ui-boundary、i18n 亦通过，聚合检查已覆盖，不另计测试数。

尚未执行本轮提交/推送、正式打包安装、真实模型或 macOS arm64 验收。跨 OS 应在包含本节代码的实际提交上**不设置实验变量**，复验用户入口与授权拒绝，继续执行下方 MAC/P0 项；源码通过不代表双平台发布放行。

## 3. 跨 OS TODO（正常产品路径，不触发通用重构）

首发范围保持 Windows x64 + macOS arm64；Intel Mac、Windows arm64、Linux 和 macOS universal 不进入本次排期。此前 Windows Node Service / storage IPC 证据见 §2.2，本次删除后的 Windows 专项回归已通过（§2.5），安装包及 macOS 仍待验。桌面仅限 Tauri / 桌面级 WebUI、最小 880×600，不加入手机/平板目标。

| OS / 架构 | 桌面宿主 Rust 构建目标 | 当前证据 |
|---|---|---|
| Windows x64 MSVC | `x86_64-pc-windows-msvc` | 删除后定向回归通过（§2.5）；安装包待验 |
| macOS arm64 | `aarch64-apple-darwin` | 首发目标；待目标机开发/验收 |

此表不是原生插件制品支持矩阵。native 后端/SDK 已移除，Windows 删除专项回归通过；不再要求 native 插件构建、打包、显式启用或运行验证。

- TODO-OS-1（目标）：在真实目标机核对桌面宿主、Node 及依赖架构、最低 OS 版本；区分交叉编译、模拟与实机运行证据。
- TODO-OS-2（权限）：核对正常产品与 Node 所需权限、macOS quarantine/签名策略、Windows ACL/安全软件影响；检查子进程环境及最小运行变量，不输出凭据，不把进程隔离当沙箱。
- TODO-OS-3（进程清理）：验证 Node/受管产品进程的取消、超时、崩溃和宿主退出回收，分别检查 Windows 进程树与 Unix 进程组、遗留子孙进程和句柄。
- TODO-OS-4（安装路径）：核对正常产品的空格/Unicode/长路径、bundle/cwd、解包权限、路径逃逸防护、安装后只读目录、临时目录和动态依赖；产物路径取实际构建结果，签名/公证缺环境时明确待验。
- TODO-OS-5（Node 生命周期）：核对 Node 安装缺失、候选切换、停止和恢复的现有产品行为；确认原生插件删除未破坏 Node 身份、调用、流式、存储 IPC 或共享回收机制，不恢复被删除的后端。
- TODO-OS-6（证据）：按目标机执行下节仍适用命令，记录实际基线、工具链、OS/架构、制品、退出码、断言/跳过数、日志及失败/重跑。缺环境不填通过；必要修复先限定文件与缺陷，不借机重构平台。


<a id="macos-launch-handoff"></a>

### 3.1 macOS 首发交接 TODO

分工：当前代理在 Windows 做本机工作；后续 macOS 目标机代理负责下表的实际开发和验证。发现共享代码缺陷只做必要修复，避免与 Windows 代理并发编辑同一文件。先核对分支、实际提交与未提交改动，不覆盖他人工作，也不自动拉取合并脏工作区。

| ID | macOS 待办 | 验收/交接输出 | 状态 |
|---|---|---|---|
| MAC-1 | 核对已选 arm64 目标、最低系统版本和工具链 | 在 Apple Silicon 机器记录宿主 `aarch64-apple-darwin`、OS、Rust/Node/Bun；Rosetta/交叉编译不算实机原生运行验证 | 架构已确认；待目标机执行 |
| MAC-2 | 编译与正式产品专项 | 按 §4 运行适用检查；验证默认内置 Agent/模型/UI，Tool/Context/discovery/before_model/只读 Skill 的真实消费 | 待执行 |
| MAC-3 | 安装后的桌面流程 | 候选 app/安装制品的安装、启动、发布选择、保存重启、停用切回内置、卸载；WebView 页面与最小桌面视口 | 待执行 |
| MAC-4 | Node/正常产品平台补充 | bundle/cwd、空格/Unicode/只读路径、运行权限、动态依赖、quarantine、宿主与 Node 架构；Node 缺失/切换/停止的正常行为；不验证已删除原生插件 | 待执行 |
| MAC-5 | 进程故障与清理 | Node/受管产品进程取消、超时、崩溃、宿主退出后检查 Unix 进程组及子孙进程；必要修复后定向回归 | 待执行 |
| MAC-6 | 最小数据安全 | 新安装/新目录、保存重启、不兼容旧库明确拒绝且原数据保留；记录新目录的现有配置方式，不追加历史版本适配或升级矩阵 | D3 已确认；待目标机验收 |
| MAC-7 | 签名、公证及分发验收 | 复用已有正式发布方式，核对签名、公证/Gatekeeper 要求；缺签名环境记待验，不绕过系统保护来宣称通过 | 直接上线已确认；签名环境待核 |
| MAC-8 | 真实模型与交接收口 | 使用 D4 授权测试环境补实模证据；回填精确提交、目标架构、命令、产物、日志、失败/重跑及未覆盖范围 | 待环境和前项证据 |

以上是待验证/待定位条目，不表示 macOS 已存在相应缺陷。修复以复现为依据；不重建插件平台、不新增模型/Shell/沙箱范围。Windows 先完成本机工作不等于双平台首发已经放行。

macOS 专用交接 prompt（配合 §4 的已有命令使用）：

```text
任务：补齐 Windows + macOS 首发中的 macOS 开发与验收，不是新增插件领域。
先读取 AGENTS.md、架构收敛记录、剩余工作文档和发布就绪台账。
此前候选为 48affddb7；最新改动含原生插件删除及 Skill/Service/UI/宿主装配收敛。接手时检查最新分支/提交、交接增量和工作区，记录实际基线，不把旧测试视为本轮通过。
已确认首发 macOS Apple Silicon（arm64），桌面宿主目标为 aarch64-apple-darwin；在相应目标机验证。
逐项执行发布台账 MAC-1～8 和 §4 的适用命令；Intel Mac、Windows arm64、Linux 和 universal 制品不在本次范围。
以默认内置 Agent/模型/UI、Node 插件与普通 Service 为验收路径；不设置实验变量，验证普通用户创建/发布/选择页面及保存/重启/切回。没有待回答 A/B 选择，不恢复原生插件、Service 流式、UI 专用订阅或草稿恢复协议。
仅针对复现的 macOS 缺陷做最小必要修复，先协调共享文件写入范围，不回滚 Windows 改动。
实际验收安装、保存重启、新目录使用/不兼容旧库保护、运行权限/路径、签名公证及 Node/产品进程组与子孙进程回收。
兼容遵循最小工作量：保留已有迁移，不新增历史适配/转换工具/升级回滚矩阵，不自动删除或覆盖旧数据。
不能用源码测试、交叉编译或 Rosetta 结果冒充另一架构的原生制品验收。
需要旧制品、测试密钥、签名环境或外部发布权限时记录依赖；不读取日常凭据、不覆盖用户数据、不上传发布。
回填 MAC 条目、具体修复文件与实际证据，交主代理最终汇总 P0；提供剩余 TODO/接续命令，未验项继续待验。
```

## 4. 可复制跨 OS 补充开发、验证 prompt

以下是目标机交接模板，不表示已执行。本次只对 Windows x64 / macOS arm64 运行；执行前核对当前源码仍有相应测试入口、工具链及命令副作用，不运行自动发布/上传命令。

```text
任务：在已批准的 Windows x64 或 macOS arm64 目标机补充正常插件产品的定向开发与验证。
先读取 AGENTS.md、架构收敛记录、剩余工作文档和发布台账，确认实际分支/提交、工作区改动、OS/架构及 Rust/Node/Bun。
原生插件后端/SDK 已移除，Windows 删除专项回归通过；不恢复旧后端/启用入口。
保留 Node 普通 Service、存储 IPC、取消/生命周期、生产模型合同及默认内置 Agent/模型/UI；插件页面普通用户可选，无实验开关。不要恢复被删除的 Service 流式和 UI 专用事件协议。
不回滚其他工作者改动；修复使用 apply_patch，先协调共享文件写入范围。
仅为 TODO-OS-1～6 的已证实缺陷做必要修复，不启动通用跨平台重构或新增插件领域。
核对宿主/Node 架构、权限与签名、进程树/进程组清理、安装路径/cwd/依赖及 Node 生命周期。
执行适用命令，检查实际断言和 ignored/skipped，不以编译成功替代运行、安装及实模验证。
按平台选择 build:mac 或 build:win，先确认实际架构、依赖和副作用。
缺签名凭据、付费服务或外部发布授权时报告依赖，不上传制品、不暴露凭据。
若修改 renderer/UI 规则，运行 bun run check:desktop-ui-boundary；保持桌面 880×600 合同。
回填基线、修改路径、命令、退出码、日志、平台、产物、失败/重跑及未覆盖项；主代理最终汇总。
其他平台继续未验证；不单凭定向测试宣称整体上线通过。
```

Node 与共享 Service 定向命令起点（先确认删除后源码仍提供这些入口；不运行旧原生插件测试）：

```text
cargo test -p nomifun-plugin-platform --test service_process --test service_storage_ipc -- --test-threads=1
cargo test -p nomifun-plugin-platform --lib service_runtime -- --test-threads=1
```

同一正式产品链路的适用专项；迁移命令仅复用已有专项，不新增历史版本矩阵：

```text
cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=1
cargo test -p nomifun-app --test plugin_product_discovery --test plugin_ui_sessions --test startup_smoke --no-default-features -- --test-threads=1
cargo test -p nomifun-db --lib plugin_ui_upgrade_tests -- --test-threads=1
cargo test -p nomifun-db --test published_main_migration_upgrade --test displaced_agent_preset_migration -- --test-threads=1
```

正式构建入口按首发平台选择：`bun run build:mac` 或 `bun run build:win`。构建不能替代安装后运行、权限、取消/退出清理及 Node 生命周期验证；具体产品流程仍按 P0-2～4 补证。完成后恢复该轮临时环境设置。
