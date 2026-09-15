# 插件发布就绪台账：尽快上线，尽可能可替换

日期：2026-09-15。状态：**本轮收敛改造与 Windows 定向回归已完成；正式上线验收未完成。**

本文覆盖 [能力评估](2026-09-13-agent-plugin-capability-assessment.zh.md) 与 [历史实施台账](2026-09-14-agent-plugin-openness-implementation.zh.md) 中旧 JS-only、任意/全栈替换和“下一批”排期；历史事实、实现和测试记录保留。29 项是长期能力评估，不是上线欠账，不要求清零，也不承诺任意替换。

本轮已实施代码收敛并运行本机定向验证。下文仅记录当前工作区的实际证据；历史通过记录不能直接改写成当前通过，工作区测试也不能代替已安装候选包验收。

## 1. 本次范围

| 定位 | 范围与边界 |
|---|---|
| 正式默认 | 内置 Agent、内置模型链路、内置 UI；默认产品流程不能依赖实验功能 |
| 已支持、按证据验收 | Tool、Context、discovery、`before_model`、只读 Skill；限定已接线来源、阶段、权限、冻结身份及真实消费者，不扩大为全领域替换或可执行 Skill |
| 可信开发者实验 | 已实现 native Service / Rust SDK 保留，默认关闭，宿主管理员显式设置 `NOMIFUN_ALLOW_NATIVE_PLUGINS=1` 才启用；不得接纳不可信原生程序或宣称安全沙箱 |
| 页面实验 | 插件 Agent 页面默认关闭，宿主 `NOMIFUN_ALLOW_EXPERIMENTAL_AGENT_UI=1` 显式启用；正式默认仍为内置 UI，不宣称完整页面交互、恢复、多视图或 Shell 替换 |
| 延期 | 模型插件、整个 Agent runtime、Shell、深层基础设施替换；已有模型合同/流式前置和 native 后端不是这些能力的正式交付 |

native 进程拥有宿主 OS 账户权限；环境清理及进程隔离不等于文件、网络、凭据沙箱。本次范围收敛不是撤销现有 native 实现，不重做平台，不启动跨平台重构。长期项若发现影响正式范围的安全/可用性问题，应登记具体 P0 缺陷，不把整个长期项升级成上线前必做。

两项开关仅识别精确值 `1`，修改后重启宿主，不提供插件自授权或动态开关。关闭页面实验时，页面候选为空、模板创建和 Agent Session 访问拒绝；历史偏好仍可读取和显式清除，内置会话、普通无会话授权的 Surface 及关闭操作仍可用。不修改共享发布目录摘要，避免影响同一插件内的普通 Tool。

## 2. P0 六项发布台账

六项责任均为主代理。状态仅在获得本轮实际证据后更新；每项填写代码基线/工作区状态、日期、OS/架构、具体命令或操作、退出码/断言、日志位置和未覆盖边界。失败及重跑分别记录；未执行、跳过、缺环境不得记为通过。

| ID | 发布项 | 本次最小验收要求 | 状态 | 本轮结果 / 证据 |
|---|---|---|---|---|
| P0-1 | 范围开关 | 核对正式默认 Agent/模型/UI；native 默认拒绝、显式启用及可信开发者警示；页面实验边界清楚，延期能力不误导为正式支持 | 本机代码路径通过 | UI 产品 3 项、native 2 项及 Product 3 项；关闭实验不阻止内置 turn 或混合插件 Tool |
| P0-2 | 产品流程 | 在内置默认链路核对安装/导入、发布、选择、保存、重启后使用；Tool/Context/discovery/before_model/只读 Skill 各以对应真实消费者及已支持形态验收，不以单一示例替代全部证据 | 源码专项通过，制品流程待验 | 最终 Product 4 项、Agent 消费者 41 项、UI 产品 3 项通过；安装包重启后流程与真实模型调用尚未验证 |
| P0-3 | 故障安全 | 核对权限/冻结身份、撤下或停用、漂移、超时、取消、崩溃、非法输出和回收；禁止静默换实现或扩大授权；实验关闭不影响默认产品，显式开启后的故障边界须如实记录 | 本机专项通过，安装后待验 | UI/Native 准入与故障、Node Service 16 项和 storage IPC 3 项通过；不替代安装后进程回收验收 |
| P0-4 | 最低升级恢复 | 核对本次支持的旧版本到候选版本的升级、锁定制品/绑定恢复、不可用插件诊断，以及可操作的停用/显式切回内置与备份恢复路径；写明起止版本，不承诺任意历史迁移或无损回滚 | 迁移专项通过，实际制品恢复待验 | 上游前缀新增迁移 3 项、published-main 5 项、displaced Agent 3 项；未操作用户实际数据或演练已签名安装包回退 |
| P0-5 | 发布构建 | 明确本次实际发布 OS/架构及制品；对应正式构建、安装/启动/卸载和必要签名检查，核对路径与运行时依赖；其他平台未验证不得宣称通过 | 聚合检查与编译通过，发布产物待验 | UI production build、Windows desktop cargo check 通过；词汇守卫清理后 `bun run check` 全链通过，见 §2.4；没有生成或安装正式候选包 |
| P0-6 | 文档 | 用户入口、作者说明、正式/实验/延期边界、native 信任风险、已知限制、升级恢复说明与实际制品一致；交付跨 OS TODO / prompt；本次文档已整理不等于验收通过 | 本轮文档完成，制品差异待复核 | 两份历史报告顶部已覆盖旧排期；本文和 native 指南统一范围、信任边界与跨 OS prompt，证据已更新至最终定向回归 |

放行规则：主代理在声明的发布目标上取得六项适用证据并处理实际阻塞后，才能写发布结论。未覆盖目标从发布声明中明确排除或继续待验证，不得用 Windows x64 结果替代其他平台。缺少非发布目标的环境不应自动阻塞无关范围。

历史证据索引（只供选择定向验证）：实施台账 §2～2.1 Tool/Context、§2.8～2.11/2.16～2.18 Skill/Context、§2.27 discovery、§2.28～2.29 before_model、§2.19～2.22/2.30～2.32 页面实验、§2.40 native。它们不自动证明当前工作区或发布制品已通过。

### 2.1 本轮必要修复与升级边界

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

### 2.2 本轮验证记录（Windows x64）

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
| Rust SDK / native Service | SDK echo 构建成功；显式 `NOMIFUN_NATIVE_TEST_EXECUTABLE` 并 `--include-ignored --test-threads=1` 执行 native 2 项和 application 3 项通过，`.tmp-release-service.*`；真实 Windows 原生进程，不外推其他平台 |
| Node Service / storage IPC | `cargo test -p nomifun-plugin-platform --test service_process --test service_storage_ipc -- --test-threads=1`，16＋3 通过，`.tmp-release-service-final.*`；首次流测试将 Node 冷启动包含在两秒事件断言内而失败，改为先完成宿主 warmup 再测流事件，协议超时和断言不放宽 |
| 前端及 Windows 宿主 | `bun run build:ui` 和最终修复后的 `cargo check -p nomifun-desktop` 通过，`.tmp-release-ui-build.*` / `.tmp-release-desktop-final.*`；有既有构建告警，不等于生成/安装正式包 |
| 发布静态检查 | 前轮 `bun run check` 在词汇守卫的 109 处检查失败（107 处文本在 HEAD 已存在），历史日志 `.tmp-release-vocabulary-final2.*` 保留。后续按用户授权清理现行命名、精确保护历史编码，最新 `bun run check` 全链通过，见 §2.4；不再是当前阻断 |
| 桌面 UI 边界 | 最后单独重跑 `bun run check:desktop-ui-boundary` 通过（1944 个 renderer source，最小 880×600）；未增加移动端目标 |

失败/重跑不隐去：Product 首轮 3 项失败；修正 Hidden 准入后仍有 2 项失败，进一步确定为临时 patch 经回执重入上下文，以及旧测试期待超时后继续模型调用。修复回执与任务保留后，正常连续轮次已通过，但挂起 Service 的 15/20 秒观察窗口不足以覆盖既有 30 秒 watchdog；只将这两项故障用例观察窗口改为 45 秒，生产超时未改，最终 4 项通过。原始及中间日志保留为 `.tmp-release-app-product2.*`、`.tmp-product-owner-fix.*`、`.tmp-release-product-diagnostic.*`、`.tmp-release-product-final.*`。

其他异常：首轮 App 编译的模块引用错误已修复；首次 DB `--lib` 启动 414 项并行用例引起资源争用，主动终止，不计通过，后续仅串行跑所需专项。尚缺 `NOMIFUN_LIVE_STEPFUN_API_KEY`，未执行真实付费模型测试、未读取用户凭据。当前安装包验收脚本要求干净 HEAD 与候选制品绑定；工作区仍含大量既有未提交改动，不绕过该检查，不自动提交、安装覆盖用户程序或上传发布。

### 2.3 收尾后的剩余上线 TODO（不再扩架构）

1. 发布负责人确认本次 OS/架构子集，审阅已整理的候选工作区，形成可追溯的已提交、干净候选基线。词汇检查已通过；大量跨轮次未提交变更仍须审阅，不自动将其全部提交为发布版本。
2. 在隔离测试数据目录构建/安装该候选，补验导入或安装、发布选择、保存重启、停用切回内置、备份恢复、启动卸载及适用签名；不覆盖真实用户数据。本轮未给出制品级通过结论。
3. 提供专用测试凭据后补真实模型调用；其他拟发布 OS/架构按 §3～4 的 TODO / prompt 执行。未纳入本次发布的架构可继续延期，不要求六个平台全部通过才能发布一个已验收目标。

不再排入本次剩余 TODO：新增插件领域、模型替换、整 runtime/Shell、完整插件 UI、原生沙箱或跨平台通用重构。

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

## 3. 跨 OS TODO（只登记，不触发本轮重构）

本机为 Windows x64。本轮已重验 native 2 项、Service application 3 项及 Node Service / storage IPC 19 项，证据见 §2.2；尚未验证正式安装包。其余平台全部未验证。下表是待核对目标，不是正式支持声明；主代理须在 P0-5 明确本次实际发布子集。桌面仅限 Tauri / 桌面级 WebUI、最小 880×600，不加入手机/平板目标。

| OS / 架构 | native target | 制品内入口 | 当前证据 |
|---|---|---|---|
| Windows x64 MSVC | `x86_64-pc-windows-msvc` | `service/plugin.exe` | 本轮原生定向测试通过；安装包未验证 |
| Windows arm64 MSVC | `aarch64-pc-windows-msvc` | `service/plugin.exe` | 未验证 |
| Linux x64 GNU | `x86_64-unknown-linux-gnu` | `service/plugin` | 未验证 |
| Linux arm64 GNU | `aarch64-unknown-linux-gnu` | `service/plugin` | 未验证 |
| macOS x64 | `x86_64-apple-darwin` | `service/plugin` | 未验证 |
| macOS arm64 | `aarch64-apple-darwin` | `service/plugin` | 未验证 |

枚举存在不代表能运行；不隐含 Linux musl、Windows GNU、其他架构或 macOS universal 制品已支持。

- TODO-OS-1（target）：在真实目标机器核对宿主/插件精确 target 与架构，不匹配须拒绝。区分交叉编译、模拟运行和原生运行证据，确认 Linux libc/动态库及最低 OS 版本。
- TODO-OS-2（权限）：核对默认关闭和显式启用；Unix 可执行位、macOS quarantine/签名策略、Windows ACL/安全软件影响；确认清理子进程环境及最小运行所需变量，不把凭据输出到日志，不把进程隔离当沙箱。
- TODO-OS-3（进程清理）：分别验证 Windows 进程树与 Unix 进程组的取消、超时、崩溃、宿主退出回收；检查遗留子孙进程与句柄，按实际平台机制定位，不能只凭退出码推断清理完整。
- TODO-OS-4（打包路径）：核对 `.exe`/无扩展名入口、空格/Unicode/长路径、cwd、解包权限、符号链接/路径逃逸防护、安装后只读目录和临时目录；当前原生制品仅接受单个可执行入口及可选 UI，不擅自加入动态库目录格式。产物路径须取实际构建结果；macOS 签名、公证和 Windows 签名缺环境时明确待验证。
- TODO-OS-5（Node 隔离）：native 调用不访问 Node authority，Node 安装缺失/切换/停止不得误校验或停止 native；反向核对 Node 原有链路。保留各后端独立身份，不能靠本机恰好装有 Node 掩盖依赖。
- TODO-OS-6（定向命令与证据）：在目标机执行下节适用命令，记录工具链、版本、OS/架构、实际制品、退出码、断言/跳过数与日志；平台不可用或命令缺依赖时记录阻塞，不填通过。跨 OS 发现的必要修复先限定文件与缺陷，再实施，不借机重构平台。

## 4. 可复制跨 OS 补充开发、验证 prompt

以下是交接给后续目标机代理的任务模板，不表示已执行。将目标替换为上表的一行；Linux/macOS 两种架构分别运行，Windows 其他架构指 Windows arm64 MSVC。构建脚本仅为仓库已有入口，执行前检查其架构选择、依赖和副作用；不要运行自动发布/上传命令。

```text
任务：在目标机补充插件发布的跨 OS 定向开发与验证。
目标 OS/架构/native triple：<上表的一行，例如 Linux arm64 / aarch64-unknown-linux-gnu>。
先确认真实 OS、架构、Rust/Node/Bun 工具链及工作区改动；不要把交叉编译或模拟运行写成原生运行。
读取 AGENTS.md、2026-09-15-plugin-release-readiness.zh.md 和 native-rust-service.zh.md。
遵循“尽快上线，尽可能可替换而非任意替换”：正式默认内置 Agent/模型/UI；
native 默认关闭、仅可信开发者实验，插件 Agent 页面实验；模型插件/整 runtime/Shell/深基础延期。
现有 native 实现保留，不撤销。不回滚其他工作者改动，使用 apply_patch。
仅为 TODO-OS-1～6 的已证实目标平台缺陷做最小必要修复；不要启动通用跨平台重构。
核对精确 target、权限与可执行位/签名、进程树或进程组清理、打包路径/cwd/动态依赖、Node 隔离。
执行下列适用定向命令并检查实际断言、ignored/skipped，不以编译成功替代运行验证。
按目标 OS 审阅并选择 build:linux、build:mac 或 build:win；确认脚本实际架构与目标一致。
需要签名凭据、付费服务或外部发布授权时停下报告，不上传制品、不暴露凭据。
若修改 renderer/UI 规则，运行 bun run check:desktop-ui-boundary；保持桌面 880×600 合同。
将代码基线、修改路径、精确命令、退出码、日志、平台、产物、失败/重跑及未覆盖项写回发布台账。
仅更新实际验证的单元格；其他平台继续未验证。缺环境列 TODO 和可复制接续命令。
最终说明哪些 P0 已有证据、哪些仍待验证；不要单凭 native 定向测试宣称整体上线通过。
```

目标机定向命令起点（先确认当前源码仍提供这些测试；其他 OS/架构未在本轮运行）：

```text
cargo test -p nomifun-agent-contracts --lib native
cargo build -p nomifun-plugin-sdk --example echo
```

Windows PowerShell（x64 或 arm64 目标机，确认默认工具链与本机架构匹配）：

```powershell
$env:NOMIFUN_NATIVE_TEST_EXECUTABLE = (Resolve-Path target/debug/examples/echo.exe).Path
cargo run -p nomifun-plugin-platform --example package_native_echo -- "$env:NOMIFUN_NATIVE_TEST_EXECUTABLE" .tmp-native-release-validation
```

Linux/macOS shell（对应目标机；使用自定义 target-dir 或显式 `--target` 时须改为实际产物路径）：

```sh
export NOMIFUN_NATIVE_TEST_EXECUTABLE="$PWD/target/debug/examples/echo"
cargo run -p nomifun-plugin-platform --example package_native_echo -- "$NOMIFUN_NATIVE_TEST_EXECUTABLE" .tmp-native-release-validation
```

完成变量设置后执行以下共有命令；打包输出目录应为本次新目录，不覆盖现有制品：

```text
cargo test -p nomifun-plugin-platform --test native_service --test service_application -- --include-ignored --test-threads=1
cargo test -p nomifun-plugin-platform --test service_process --test service_storage_ipc -- --test-threads=1
cargo test -p nomifun-plugin-platform --lib service_runtime -- --test-threads=1
```

若该 OS/架构也在正式桌面发布范围，补做同一产品链路与升级专项；仅 native 实验验证不应自动扩成完整发布验收：

```text
cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=1
cargo test -p nomifun-app --test plugin_product_discovery --test plugin_ui_sessions --test startup_smoke --no-default-features -- --test-threads=1
cargo test -p nomifun-db --lib plugin_ui_upgrade_tests -- --test-threads=1
cargo test -p nomifun-db --test published_main_migration_upgrade --test displaced_agent_preset_migration -- --test-threads=1
```

正式构建入口按平台三选一：`bun run build:linux` / `bun run build:mac` / `bun run build:win`。构建不能替代安装后运行、权限、取消/退出清理及 Node 隔离验证；具体产品流程还须按 P0-2～4 补证。测试开关/环境仅用于该轮验证，不将 `NOMIFUN_ALLOW_NATIVE_PLUGINS=1` 写入默认安装环境。完成后恢复该轮临时环境设置。
