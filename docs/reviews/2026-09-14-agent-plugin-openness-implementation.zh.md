# Agent 全栈插件开放实施台账

## 当前发布口径（2026-09-15，优先于历史实施安排）

本轮批准实施的清理项、两个独立开放方向及来源支持矩阵以 [架构收敛记录](2026-09-15-plugin-architecture-convergence.zh.md) 为准。旧 Skill loader、Service 流式前置、UI 专用事件协议及重型参考客户端不再作为现行能力；本轮同时收正宿主装配和 hook 验证职责。具体测试结果见发布台账，不沿用历史通过数。

最新目标是**尽快上线，尽可能可替换，而非任意替换**。当前执行入口改为 [P0 发布台账](2026-09-15-plugin-release-readiness.zh.md)，不再以 §3 的 29 项全部完成、五问整体关单或历史 JS-only/全替换路线作为上线条件。

- 正式默认：内置 Agent、模型链路、UI。已有 Tool / Context / discovery / `before_model` / 只读 Skill 仅在已支持边界内按证据验收。
- native Service 后端、Rust 作者 SDK、启用入口及专属示例/打包/测试路径按最新决定删除，不保留默认关闭或显式启用实验。代码删除已完成；已通过检查及待补回归统一见发布台账。§2.40 仅保留历史记录，历史测试不代表现行支持。插件 Agent 页面决策独立，正式默认仍为内置 UI。
- 模型插件、整个 runtime、Shell、深层基础设施替换延期。已有前置实现不代表正式支持，也不触发继续开发这些延期项。
- §3 改为长期能力评估清单，不是上线欠账；仅影响本次正式范围的实际缺陷进入 P0。历史段落中的“当前”“本期”“在途”“下一步”均按记录当时理解，不是本轮指令。

此前收敛改造与 Windows 定向回归保留为历史证据。原生插件已移除，Windows 删除专项回归通过；最新证据统一见[发布台账 §2.5](2026-09-15-plugin-release-readiness.zh.md#25-原生插件删除后的验证进展2026-09-15)。正式制品与 macOS 验收未完成，不由历史通过数推断。

## 历史实施记录（以下日期、事实和编号保留）

日期：2026-09-14

启动代码基线：`439bb385a`

历史同步目标：`e617feb2b`（当时 7 个上游提交，同步记录见 §2.40）

状态：实施中，完整目标尚未完成

§2.40 记录曾同步并实现原生 Service 后端及作者 SDK，后按范围收敛删除的历史。其详细设计、开发与打包方案已移除；历史切片测试只代表当时证据，不代表现行原生插件支持，也不等于完整模型/UI/主循环替换交付。

阅读入口：当前开发顺序与实现卡统一见 [报告 §27.14](2026-09-13-agent-plugin-capability-assessment.zh.md#2714-进度纠偏与下一批停止条件)；本台账 §2 保留历史切片证据，§3 记录剩余范围。历史段落中的“下一步”和“在途”标题不另作当前排期或最新状态。Product `before_model` 已按 §2.28～2.29 限定范围收口；最近用户功能卡为 §2.32 消息来源与基础呈现。§2.33 审批与 §2.34 模型预检仅研究；§2.35～2.36 已实现原 Service 流式前置及产品内部受管调用接线，§2.37 将已有模型纯数据合同归位到合同层。模型选择/消费尚未接通。B 整体及完整目标均未完成。

**当前进度收敛：** 暂停自动追加参考页和相邻 UI 功能。预设复制继承页面偏好只完成代码阅读，尚无该功能实现或验收；不将其算作在途开发成果。B 的剩余项目仍保留，但不作为所有 C/D 工作的串行前置。模型流式前置按 §2.36 完成定向检查和文档收口；下一批仍围绕报告 §27.14 的真实 Nomi 模型消费结果，不追加通用事件系统，不把内部切片数量当作整体完成比例。

## 1. 范围与架构约束

历史实施范围来自 [评估报告 §27](2026-09-13-agent-plugin-capability-assessment.zh.md#271-直接结论与逐项对应)：用户 capability 可选替换、UI、Agent 全环节、Skill 装载、简单化组装。原 native Service 后端与 Rust 作者 SDK 按最新决定删除；Rust 宿主、内置实现、Node、共享 Service 流式和模型合同继续保留，不另建第二套平台。

**历史约束（2026-09-14，不构成恢复原生插件的计划）：** 具体实现若在 JS 支持下不合理，则延期，不做临时 JS 版本。按报告 §27.13 筛选；29 行继续跟踪长期目标，不是本期强制全量 JS 清单。不得新增原生代理、复制状态/执行器或预建 Rust 空框架满足名义覆盖；普通 UI、异步业务及成熟公开协议的实现不因宿主使用 Rust 而自动延期。

当前代码已合并 Plugin/MiniApp 统一工程，不重复建立身份、Catalog、Snapshot 或产品平台。保留唯一 canonical Compiler、冻结 Provider/制品身份、目标级资源绑定、单 Session 执行 owner；业务消费者通过 `nomifun-ai-agent` 接缝，不新增直接依赖 Nomi 内部模块的旁路。安装/发布属于 Plugin 平台，Agent 只消费已发布贡献。

读取依据：根 `AGENTS.md`、backend-crates 架构文档、05 的 Role/Provider/消费者合同、评估报告 §14～27，以及当前 Kernel、JS adapter/host、产品目录发布、Nomi Session/提示词投影代码。旧阶段文档中的 N1 功能限制是历史交付范围，后续按此次明确授权逐项扩展，不改变唯一目录/编译/授权的架构要求。

桌面 UI 最小视口遵循当前仓库的 880×600 合同，不建设手机/平板布局。部署级服务和最小信任根的例外仍按报告 §27.4 单独验收，不以重编译宿主冒充安装型插件替换。未经验证的功能与平台不记为完成。

## 2. 当前已实施切片：用户 Context 的真实消费

重新阅读发现的断点：JS Package 与 Kernel 已支持 ContextContributor，但产品可用性仅接纳用户 Tool，Nomi 初始上下文也仅装载显式获准的内置 Context。已有协议无需另建 Context registry 或再加一种包格式。

本轮改动：

- `nomifun-ai-agent::supports_nomi_plugin_capability` 统一描述 Nomi 实际支持的用户 Tool/Context 形态；产品 Catalog 使用同一判断。来源、运行环境与权限检查不由该形态判断替代。
- `KernelNomiPluginToolSession` 对已选中、精确锁定的 ManagedLocal/PluginMount Context 使用现有 `KernelRegistry::contribute_context`；内置 Context 保留原有显式 admission。两者进入同一提示词组装入口，不额外注册模型 Tool。
- 保留精确来源验证、当前 Kernel 授权、每个贡献调用 5 秒上限与 64 KiB 总上下文限制。5 秒不是所有贡献合计的 Session 启动上限。失败、撤下、制品漂移、超大结果均明确失败；未选中贡献不调用。
- 产品安装集成测试增加真实 JS Context 导出，覆盖导入/安装、Catalog 可用性、编译 Snapshot、Node 执行和 Nomi 系统提示词消费。测试不依赖外部模型或 API key，也不把捕获提示词等同于模型必然遵循。

验证记录：

| 检查 | 结果 / 边界 |
|---|---|
| 修改前 `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 8 项通过，作为该消费接缝的本轮基线 |
| 修改后同一命令 | 13 项通过，含新增用户 Context、未选择、漂移/撤下、失败/大小边界和形态检查 |
| `cargo test -p nomifun-app --lib router::plugin_platform::tests::nomi_core_plugin_flow_materializes_and_withdraws_exact_mount --no-default-features` | 1 项通过；真实 Node 插件导入/安装、目录可用性、锁定 Context 执行和提示词消费；原 Tool/Gateway、重启恢复与卸载检查仍通过 |
| `cargo test -p nomifun-app --lib router::nomi_core_agent_projection::tests --no-default-features` | 19 项通过；投影、精确绑定、动态能力、资源和模型路由等既有回归；文档复核时补录已有结果，未重新运行 |

这只是 Context 的初始装载切片，不代表整个 Prompt 管线、每 turn Context、用户 Provider 替换或 Skill 已全部完成。最初记录的 Tool/Context 放弃等待后取消通知缺口已在下节补齐；其他资源/服务/未来流式协议的取消与清理不能由此推断完成。

当前初始贡献结果按 capability ID 排序，还未实现用户指定的管线顺序；总启动预算也需另行覆盖。2026-09-14 文档复核补充的现状、合同冲突及五组用户验收见 [报告 §27.8～27.10](2026-09-13-agent-plugin-capability-assessment.zh.md#278-当前代码复核不能把已有切片扩大解释为需求完成)，不据此扩大该切片的完成范围。

### 2.1 已实施：Tool/Context 共享协作取消

当前 `nomifun-js-host` 的 supervisor 继续作为唯一进程/request owner，不新增取消注册中心、后台发送任务或另一套 Context supervisor：

- `HostRequestHandle` 或其 `wait` future 被丢弃后，Actor 根据原 oneshot 接收端是否关闭，向已发出的 Tool/Context 请求发送 request-scoped cancel；还未分发且已无人等待的调用不再执行。
- 复用原有一个预留取消槽和有界输出队列。多个请求放弃时逐个发送，取消确认或 Actor 其他进展后继续推进；没有流量时仍由现有 watchdog tick 唤醒。取消已发标记防止重复排队。
- Node Tool/Context 使用同一个 `runCancellableMountRequest`；两类导出都获得 `signal`，结束时移除 request-local Controller。不响应取消仍按原请求 deadline 失败并回收 generation；ACK 不等于原请求结束，也不延长原 deadline。
- 共享 MountLoad、ResourceAcquire/Release 等生命周期请求不因一个普通等待者退出而被上述逻辑误取消。取消不撤销已提交副作用，不自动重放工作，不代表 Node 获得强沙箱隔离。

新增真实 Node 测试覆盖：同 Mount 上多个已放弃 Tool 被取消而保留的请求继续等待；Context future 被 abort 后 JS 收到 signal 且原 Mount/Generation 仍可复用；工作槽满时使用预留取消槽；不响应取消仍被 watchdog 回收。原有资源、共享装载、服务、IPC 背压和进程回收检查继续保留。

| 检查 | 结果 |
|---|---|
| `node --check`：Host 入口和 `tests/fixtures/main.mjs` | 通过 |
| `cargo test -p nomifun-js-host --test extension_host --no-default-features` | 首次与取消槽续发调整后的最终复跑均为 73 项通过，含新增 4 项 |
| `cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter --lib --no-default-features` | Host 16 项、adapter 2 项通过 |
| `cargo test -p nomifun-js-host --test extension_host cancellation --no-default-features` | 调整取消槽续发时机后 7 项通过；不是再新增 7 项测试 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 本轮最终复跑 13 项通过，保留初始 Context 与 Tool 消费证据 |

此处没有实现用户 Provider、Skill 装载、每 turn Context 或整个 Runtime，不能关闭对应需求。资源获取方消失后的租约归属、SDK 子调用和后续流式插件的完整取消语义仍须在所属工作包验证。

### 2.2 已实施切片：JS Role/Provider 的精确映射与真实分发

本轮重新读取合同、Kernel/JS adapter、产品安装/发布和 Nomi 消费接缝后，在原有主链上完成：

- `RoleProviderMemberContribution.implementation` 表达契约 member 到用户自有 capability 的精确引用。无映射的内置 typed export 继续工作；JS Provider 必须显式映射。用户可以发布自己 Package 命名空间的 Role，并实现已有系统/其他插件契约，不抢占系统 capability ID。
- N1 validator 检查映射来源、精确版本、非 façade 目标、契约 namespace/member digest；Kernel Materializer 检查 Provider Mount/Package 归属、契约及实现兼容。Compiler 额外检查映射实现自身的平台、Surface 和 runtime feature，不只看 façade。
- JS adapter 使用既有四种 typed 接缝：Agent action、非 Agent Role Tool、Context factory、Resource factory。它们验证冻结 Provider/source/artifact 与 Mount state，然后复用原有 JS Host 调用；没有第二次 Kernel dispatch、独立 Role registry 或新的 RPC 方法。
- 契约 façade 不注册 generic handler；独立实现 capability 保留普通调用入口。映射不会把实现额外加入模型工具集，也不自动增加授权。资源获取/释放复用原有 handle 与租约路径。
- 05 §5.5 和 06 的后续开放说明同步演进。`None` 不序列化，原 bundled Provider 摘要不变；新增映射进入 contribution digest。更新 v1 生成 schema/digest ledger，不改变现有 Node wire method/version；旧宿主仍会明确拒绝含 Role 的用户包。

真实 Node 测试将一个 bundled typed Provider 与用户 JS Provider 放在同一个 Kernel 中，验证默认/override 调用落点、Tool/Context/Resource、一次资源释放、非 Agent 操作、独立 capability、老 Snapshot 不漂移、撤下不回退、实现平台限制和旧制品锁失效。另验证不兼容声明被拒绝，以及用户自己的 Role façade 不进入 generic handler。

产品安装测试在真实包中加入用户定义的 Context Role，分别编译独立 Context 和映射 façade，均经安装后的 Kernel/Nomi 接缝进入实际组装提示词；保留候选测试、导入/分享、重启恢复、Gateway 与卸载回归。这不是选择 UI 或 Browser/Computer 领域替换的验收。

| 检查 | 结果 / 边界 |
|---|---|
| `cargo test -p nomifun-agent-contracts -p nomifun-agent-kernel -p nomifun-js-kernel-adapter --no-default-features` | contracts 87、Kernel 31、adapter unit 2、真实 Node adapter integration 6 项通过；包含新增 3 个合同测试和 4 个 adapter 集成测试 |
| `cargo test -p nomifun-app --lib router::plugin_platform::tests::nomi_core_plugin_flow_materializes_and_withdraws_exact_mount --no-default-features` | 1 项通过，已扩展为独立 Context 与用户 Role Context 两条 Nomi 消费路径 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | Provider 改动及 schema 生成后复跑 13 项通过，原 Tool/Context 消费与来源漂移回归仍通过 |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write`；同次构建工具 `target/debug/agent-v2-contract.exe check` | 生成及检查通过；同步生成的 platform/runtime fixture digest 只是合同引用更新，不代表重新运行跨平台验证 |
| 新增文件的定向 rustfmt；`git diff --check` | 通过；未全仓格式化，未修改 UI，因此未运行 UI boundary 检查 |

未完成边界仍明确保留：依赖/资源目前采用严格相等兼容，不能据此关闭报告 §14.5 的异构实现要求；单 binding Resource 限制、完整跨插件子调用/授权和 Role 请求的领域级取消/清理仍需继续验证。本切片当时未提供 Catalog/Provider 选择 UI，该缺口的后续进展见 §2.3；默认绑定管理、升级影响分析和跨包契约的启动恢复仍需闭环。当前恢复按 Mount 顺序逐包发布，不能假定任意跨包契约依赖都能在重启后正确恢复。没有以本轮分发成功关闭 P2/P7 或 29 项总目标。

### 2.3 已实施切片：同一 Catalog 的 Provider 候选与 Agent 级选择保存

重读发现编辑器原先分别请求 capability、Skill、MCP 列表，且公开 Catalog DTO 未包含 Role/Provider。现已在同一主链完成：

- `CatalogSnapshot` 从同一 MaterializedRegistry 投影 Role 契约和 Provider；API 返回精确契约/选择结构、来源、包版本及支持的 member。仅展示 Agent 消费者可见的精确 member，过滤 TestFixture；不把候选列表冒充环境兼容或授权结论。
- 新增读取整个快照的 `GET /api/agent-catalog`，仍由原 Control Plane 和 owner 鉴权上下文提供。工作台改为一次请求该投影，不再把三次独立请求拼成目录。原子列表是可重建的读模型，不新增目录权威、持久化事实或求解器；既有分项读取端点保留给其他消费者。
- 个人 Agent 编辑器和官方模板自定义共用“部件实现”选择器。它提交现有 `system_role_provider_overrides`，不要求用户填写 digest/mount，不改写 capability ID，不自动加入内部工具。界面按选中能力/依赖/Skill 需求展示相关 Role；该遍历仅决定展示，实际计划仍由后端编译。
- 契约或 Provider 撤下后保留旧选择并提示；同 Mount 的契约 digest 改变也不会自动重新绑定。明确选择“使用已配置的默认实现”只删除该 Role 的 Agent override，不冒充已实现安装级默认管理。
- 真实 JS 安装测试从公开 Catalog 取候选，走正常控制面 save/read Revision、读取持久化 Snapshot，再复用生产 `compile_nomi_plugin_snapshot` 等价检查和 Nomi Context 消费。测试存储为既有 InMemoryControlPlaneStore；不把它称为跨进程数据库恢复或真实外部模型验收。生产 helper 仅扩大到 router 模块可见以复用，没有新增编译路径。

| 检查 | 结果 / 边界 |
|---|---|
| `cargo test -p nomifun-agent-control-plane -p nomifun-agent-platform --lib --no-default-features` | 最终 Control Plane 29 项、Platform 18 项通过；新增两项精确候选投影检查和一项完整 Catalog 路由/owner 上下文检查 |
| `cargo test -p nomifun-app --lib router::plugin_platform::tests::nomi_core_plugin_flow_materializes_and_withdraws_exact_mount --no-default-features` | save/read/生产消费接线后 1 项通过；独立 Context 与 Role Context 均进入实际组装提示词，保留安装/重启/卸载检查 |
| `cargo test -p nomifun-app --test nomi_core_route_gap default_nomi_core_router_answers_canonical_catalog_requests --no-default-features` | 1 项通过；真实默认应用路由经本机信任鉴权返回完整 Catalog 的四类列表，保留原分项读取路径 |
| `bun test --cwd ui src/renderer/pages/agentSettings` | 55 项通过，含新增 6 项选择器/个人编辑器/模板、保存数据、撤下/契约漂移和依赖展示检查；修复新测试把全局 toast 算入组件告警的问题后复跑通过。仍有现有 Arco/React ref 和 act 警告 |
| `bun run typecheck`、`bun run check:desktop-ui-boundary`、`bun run check:i18n` | 通过；桌面/WebUI 最小 880×600 边界未改变；未做真实浏览器/WebView 视觉验收 |

还不能关闭 P2/P7：安装级默认的管理/展示、不同 Provider 依赖、跨包恢复、平台候选诊断与升级影响仍需推进。本切片发现 `snapshot_matches_registry` 只比较 capability，不能从首次 save 成功推断升级后 clean-save 校验已正确；该缺口的后续修复见 §2.4。此处候选选择和首次保存已有证据，不代表所有 Browser/Computer 等领域替换完成。

### 2.4 已实施切片：Provider-aware 的无改动保存校验

原问题：用户未修改 Agent 文档时，控制面仅比较已锁定 capability 的 materialization。系统 façade 保持不变、但选中的 Provider 制品升级或撤下时，保存可能直接复用已经失效的旧 Snapshot。

本次复读 05 §5/Role binding 合同、Compiler、Registry 分发和 JS adapter 后，在原有链路补齐：

- Kernel `AgentPresetCompiler::role_providers_unchanged` 调用已有 `compile_role_provider_locks`，比较当前解析结果与保存的精确锁；控制面不复制 override/default 选择规则，也不增加 registry 或持久化字段。原 capability 检查继续负责闭包，新增检查负责 Provider 部分。
- 保存复用检查覆盖 Role 契约、Package/version、Mount、contribution digest、source（包含 JS 制品摘要）、支持 member 集合，以及原解析器的所选 member 平台/实现可用性规则。解析失败即退出复用路径，由正常编译产生诊断；不会换成其他 Provider。
- 当前默认绑定只在**新保存/编译**时参与选择。默认目标发生变化且无 Agent override 时，新保存生成新 Revision/Snapshot；有显式 override 时继续使用该选择。旧 Revision、旧 Snapshot 和已有 Session 不在此过程中改写。
- 未选中 Provider 的变化、无关 Registry generation、默认绑定的版本号/时间戳变化（选择未变）不制造新 Revision。仅当当前解析的实际锁变化或失效，才不再复用。
- 新增正常 `save_revision → get_revision/get_snapshot` 回归，验证升级后的新版本持久化、旧版本不变、撤下失败不追加版本，且即使内置 Provider 仍可用也不回退。使用 InMemoryControlPlaneStore，不冒充 SQLite 跨进程恢复或真实 Provider 发布升级测试。

验证：`cargo test -p nomifun-agent-control-plane -p nomifun-agent-kernel --lib --no-default-features`，Control Plane **35** 项、Kernel **31** 项通过；新增 **6** 个测试函数（含多种变更分支），不是新增 66 项。首次回归中两个断言错误地把 Kernel variant 名当成公开错误码，按现有 `canonical_code()` 映射修正后完整复跑通过，未改生产错误协议。Role 缺失/不匹配目前通过 `CAPABILITY_NOT_MATERIALIZED` 加具体 message 返回，平台不匹配通过 `CAPABILITY_UNAVAILABLE_ON_PLATFORM`；细粒度产品诊断仍需后续完善。

另复跑 `cargo test -p nomifun-app --lib router::plugin_platform::tests::nomi_core_plugin_flow_materializes_and_withdraws_exact_mount --no-default-features`，**1 项通过**，保留真实 JS 安装、公开 Catalog 选择、正常保存回读和 Nomi Context 消费链证据；不是新增的真实发布升级场景。新增测试文件定向 rustfmt、`git diff --check` 通过。本批未修改 UI、未重新运行 UI 或全仓检查，也未做跨平台/外部模型验收。

未完成边界：`CompilerEnvironment` 仍是控制面构造时传入的值，默认变更测试使用新编译环境，不等于已经有实时默认管理 API/UI。下一步管理功能必须从现有 installation binding 存储读取一致的当前值，不能再建第二份选择事实。Provider 升级影响清单、跨包恢复、异构依赖及完整 Skill 装载仍未闭环；本批不是通用 Snapshot 的所有输入漂移审计，也没有改变 Session 执行时的冻结规则。

### 2.5 已实施切片：跨包契约/Provider 的整批恢复与正常对账

复读安装服务、repository、发布器、Kernel 原子发布和资源释放路径后，确认两个相关缺口：启动先发布空目录、再按 Mount 排序逐包加入，会丢失先于契约包出现的 Provider；正常对账只修改一个 Mount，禁用契约时其依赖者会阻止整批发布，重新启用也不能自动恢复此前被排除的 Provider。

本次沿原有权威链收敛：

- 启动与正常对账共用 `recover_inventory`；持有原发布 mutex 后读取已提交 inventory，从真实不可变包重建候选，不使用传入旧 Mount 行拼接缓存。此工作在安装/配置/启停与恢复时执行，不进入每个 Tool/turn 的热路径；大规模安装的重建成本尚未基准测试。
- 每次将 base + 全部动态候选交给现有 Kernel `replace_all`。它验证失败不改变 generation，验证成功才交换整批；不再先发布空目录或发布一系列可观察的前缀。
- `plugin_registry_recovery.rs` 仅把 Kernel 已有的类型化错误主体映射回声明所属 Mount，不解析错误字符串、不计算依赖图或兼容性。可归属错误排除相应用户 Mount 后重试剩余整批；候选集合严格缩小。重复主体不按 Mount 顺序选择赢家，base 不参与移除。没有安全归属的错误立即返回，不猜测隔离对象。
- 已提交安装仍由 DB 持有；目标自身被拒绝时返回现有 reconcile-required 错误，不回滚 DB、不静默替换 Provider。无关坏包不会让有效目标的正常启停失败。契约禁用后依赖者从 live Registry 撤下，但不修改其 enabled/revision/制品；契约恢复后重新读取 inventory，自动恢复原精确 Provider。
- 删除单 Mount 发布/失败回退分支和动态 registration 缓存；只保留上次发布 Mount 集合用于清理。撤下后清理各相关 Mount 的已持有资源，尝试所有清理并刷新目录可用性，不清空独立插件的资源。目标清理失败也会继续尝试对账，不能因为清理失败就跳过已提交状态的发布。

真实 JS 集成覆盖 Provider 先安装但缺契约、补装契约后无需 retry、独立插件共存、不兼容 Provider 隔离、宿主重新组合只产生一个成功 generation、Catalog 精确选择与正常 save/read/Nomi Context，以及契约禁用/重新启用后原 Provider 的撤下/恢复。另有依赖环失败边界与类型化归属回归：无法隔离的全局错误保留原 `Arc`/generation；用户禁用坏包后可正常发布。测试重用同一 SQLite 内存数据库 pool 与磁盘制品根，不称为跨进程重开数据库。

最终验证：`cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features`，**6 项通过**，含新增 **2** 项恢复测试和既有真实安装/撤下回归；日志见 `.tmp-open-provider-recovery-verified.out/.err`。新增文件定向 rustfmt、`git diff --check` 通过。首次 fixture 删除 Tool 后误保留输入 schema，按实际 Context 引用裁剪 schema 后启动恢复测试通过；正常对账测试编译时修正了新断言对 DTO lifecycle 的读取及 Mount ID 格式化，没有放宽产品验证。本批未改 UI，未运行 UI 检查、全仓测试、跨平台或外部模型验收。

仍未闭环的边界：依赖环等无作用域错误不能自动局部恢复，保留上一代目录并报错；未新增持久化健康状态或面向用户的关联影响清单。撤下与已在途 acquire 的竞争、Host generation/取消和租约生命周期不由本切片完整证明，仍归资源工作包。安装级默认、异构实现依赖、真实领域替换、Skill 及其余环节仍待继续实施。

### 2.6 已实施切片：产品安装默认管理与实时保存、冻结执行

重新读取实际组合根发现：已有 `installation_role_bindings` 装载只在独立 Fresh-v4 平台路径，当前产品的 `state.rs` 给 Compiler 传空绑定；Nomi 启动又会重新编译持久化 Snapshot。直接刷新执行环境中的“当前默认”会让旧 Snapshot 校验受新默认干扰，因此本批同时补 authoring 与执行边界。

- 产品 DB 当时用新增 migration `097_installation_role_bindings.sql` 持久化同一个 canonical `InstallationRoleBinding`；2026-09-15 发布收敛已将其顺延为 `105_installation_role_bindings.sql`，避免插入新上游历史前缀。每个 Role 一行，技术主键/逻辑关联按 v3 registry 登记，没有双读写或打开另一条 Fresh-v4 Session 数据库。Provider Mount 包含内置不透明标识，不能伪造到 `plugin_mounts` 的物理外键；撤下后保留选择供修复。
- DB repository 提供读取与单语句版本 CAS；`0` 只允许首次创建，已有 binding 必须匹配版本后更新。失败不覆盖其他选择。实际产品适配器用安装 owner 鉴权，Control Plane 复用该存储提供 `GET /api/agent-role-defaults`、`PUT /api/agent-role-defaults/{role_id}`，没有第二份进程内绑定缓存。
- 设置默认先调用 Kernel 同一 Role resolver 校验精确契约、Mount、必需 member 与运行环境；不要求在安装设置时提供 Session 资源实例。可选 member、完整依赖/授权与资源绑定仍在具体 Agent Save/admission 校验；设为默认不是授权或“所有用途都可用”的证明。
- 三个现有 authoring 编译调用点统一先读取当前存储，再调用原 Compiler，包括初始创建、重新保存和模板相关编译；无需重建控制面。现有无改动保存校验决定是否新增 Revision；显式 override 仍优先。
- `compile_nomi_plugin_snapshot` 从持久化 Provider locks 派生本次校验使用的选择，忽略当前安装默认；仍校验完整编译结果与保存 Snapshot 完全相等。该派生不是新的持久化 binding，也不改写 Revision。旧制品/实现不可用时明确失败，不切到新默认。
- Agent 工作台新增默认实现管理弹窗，直接用现有 Catalog 候选，不要求用户填写 digest/Mount。每项显式保存、版本冲突不自动重试覆盖、失效选择保留；读取失败不显示可写的假空配置。界面说明“仅影响新建/重新保存的继承者，旧版本/会话不变”。支持桌面/WebUI 的 880×600 合同，不扩展移动布局。

| 验证 | 结果与范围 |
|---|---|
| `cargo test -p nomifun-db --lib installation_role_bindings --no-default-features` | 1 项通过，真实磁盘 SQLite 保存、版本冲突、schema/data registry 与关闭后重开；不是全量旧数据库升级验收 |
| `cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features` | 7 项通过，新增 1 项跨真实 JS 包/Catalog/HTTP PUT/SQLite Control Plane/生产 Nomi 消费的完整默认场景；同实例新保存采用新默认，override 优先，旧 Snapshot 不变且旧实现撤下不回退 |
| `cargo test -p nomifun-agent-control-plane -p nomifun-agent-kernel --lib --no-default-features` | Control Plane 35、Kernel 31 项通过，保留之前的 clean-save/来源漂移与 Kernel 回归；不是新增 66 项 |
| `cargo test -p nomifun-app --test nomi_core_route_gap default_nomi_core_router_answers_canonical_catalog_requests --no-default-features` | 1 项通过，实际产品路由经本机信任鉴权读到默认绑定 API，保留完整 Catalog 与分项路径 |
| `bun test --cwd ui src/renderer/pages/agentSettings` | 59 项通过，含新增 4 项默认管理交互：显式选择/CAS、冲突、失效保留和读取失败；仍有既有 Arco/React ref/act 警告 |
| `bun run typecheck`、`bun run check:desktop-ui-boundary`、`bun run check:i18n`、`git diff --check` | 通过；未运行全仓测试、真实 WebView 视觉验收或跨平台/外部模型验收 |

边界仍保留：未声明默认的用户 Role 不自动挑选“唯一/第一个”Provider；当前管理页从 Agent Catalog 展示候选，不冒充全部非 Agent/部署服务的设置中心。独立 Fresh-v4 host 的启动期默认装载与 seed 未在本批改为新的管理产品路径。全量内置角色初始化、每个领域消费者、异构实现依赖、升级影响清单和资源生命周期仍需继续验收。本次没有关闭完整 P2/P7、Skill、UI 替换或 29 环节总目标。

### 2.7 已实施切片：所选 Provider 的不同资源/特性需求与保存一致性

重新阅读 Role 契约、materialization、Compiler、准入和真实 JS 资源调用后，收口 §2.2 留下的一部分严格相等限制。此处是 **Tool/Context 的私有资源及运行特性差异**，不是任意不同 capability 依赖已经闭环。

- `role_implementation_matches` 不再要求 Tool/Context 的资源清单、runtime feature 与 façade 完全相等；映射 member 的资源需求必须精确匹配实现 manifest。action/schema/effect、host port、事件/Context schema 仍遵守共同契约；本切片当时 `requires`/`conflicts` 仍相等，后续冲突差异演进见 §2.12。
- ResourceProvider 的输出 kind 与 Role 的序列化目标资源是对外类型/身份约束，不按私有实现差异放宽；不允许更换资源输出类型或省略 façade 使用的序列化目标。
- Compiler 解析 Provider 后，通过 `apply_role_requirements` 更新同一 `ResolvedCapability` 的有效需求，再派生原有 authority policy、全局需求与 profile digest。私有资源取所选实现的需求，不与原默认实现取并集；制品、契约与映射身份仍由原有精确锁保护。
- 实际消费的资源 factory 也必须通过平台/Surface/feature 准入，其契约与映射实现的 feature 纳入有效需求；不要求同 Provider 的无关可选 member 特性。Compiler 与 Registry 调用复用 `role_resource_members`，删除分发处重复的选择逻辑；不另增公开 capability/模型 Tool 或授权。
- Tool、Context、non-Agent operation 继续通过既有 owner、binding、operation grant 与 acquire/release 路径。真实 Node fixture 返回获取后的资源参数，证明调用确实使用所选实现的资源，不只检查编译字段。
- 无改动保存通过原 Kernel resolver 比较 Provider 精确锁后，再用同一投影函数比较有效需求记录；锁不变但需求投影过时时重新保存新版本。新增控制面正常 save/read 回归验证旧 Revision/Snapshot 保持不变，后续 clean-save 重新稳定复用，不每次制造版本。

本批没有增加 wire 字段、RPC、JS 计划或第二个授权入口；05 §5.5 已同步语义，package 中新增解释为普通注释，不为了注释更改 schema。资源实例仍是 Session/target 状态，不放入 Preset。

| 验证 | 结果与范围 |
|---|---|
| `cargo test -p nomifun-agent-control-plane -p nomifun-agent-kernel -p nomifun-js-kernel-adapter --no-default-features` | 最终 Control Plane 36、Kernel 31、adapter unit 2、adapter integration 12 项通过；含资源需求相关 6 个 adapter 测试及 1 个控制面保存测试；doc tests 通过（0 项） |
| `cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features` | 最终 7 项通过，保留真实 JS 安装、默认变更、正常保存/回读、Nomi Context、撤下与跨包恢复；不是新增 Browser/Computer 或产品自定义资源解析测试 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 最终 13 项通过，保留 Tool/Context、来源漂移、准入与失败回归；日志 `.tmp-open-requirements-consumer-final.out/.err` |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`；新构建的 `target/debug/agent-v2-contract.exe check` | 通过，无 schema 内容变更；不是重新运行跨平台 release 验证 |
| 新增测试文件定向 rustfmt、`node --check` fixture | 通过；未对既有大文件全仓格式化 |

首次新增 adapter 保存复用测试错误地用带额外需求的实现生成了内置契约，导致契约摘要不一致；改为从原始内置基线构造契约后完整复跑通过，没有放宽生产契约校验。日志 `.tmp-open-requirements-closure-final.out/.err` 与 `.tmp-open-requirements-product-final.out/.err` 对应最终代码，替代报告此前“最新 helper/保存校验未复跑”的在途状态。过时投影用语法有效的合成历史 Snapshot 检查，不声称进行过真实旧版本数据库迁移。

最终 `git diff --check`、两份报告的本地文件引用/代码围栏与 29 项清单检查通过。本批未改 UI，未运行 UI 检查、全仓测试、真实 WebView、外部模型或跨平台验收；已有工作区 UI/合同生成改动保持原样，未提交或清理无关文件。

剩余边界：任意递归 capability 依赖及受管 SDK 子调用、内部依赖的用途/授权区分、自定义产品资源 kind、多命名 binding、资源获取与撤下竞争、升级影响诊断和全部 Browser/Computer 消费者仍未完成。不能将有效资源需求的局部闭环记为整个 P2/P7、02/09/11 或原始需求 1 完成。后续仍按单一计划、实际消费者和逐操作授权推进。

### 2.8 已验证切片：包内 Skill 精确装载与标准工具消费

本批接续已有来源锁/只读装载器改动，并重读 Kernel、制品存储、产品 Session 和 Nomi Bootstrap：

- `ResolvedSkillLock` 增加必填 contribution/source/Mount/artifact 锁；Kernel Compiler 生成同一锁，Control Plane 无改动保存复用时检查它。Snapshot 验证拒绝重复 Skill、错误 Mount、非法摘要和未选依赖；编译检查声明的宿主 Surface 与 Agent consumer。
- 现有制品存储增加声明文件的有界读取，先验证 inventory，再核对实际字节摘要。单文件上限 1 MiB、每 Session 合计 8 MiB/128 个文件；不向运行时交付裸 package path。产品 artifact resolver 用阻塞 IO 线程读取，不阻塞异步工作线程。
- 已鉴权产品 Session 调用 `with_package_skills`，检查与会话相同的 Snapshot、当前 Registry 精确来源和 active dependencies；装载等待后再次检查，正文/资源每次读取仍有来源守卫。正文/资源缓存不绕过撤下；升级不会使老 Session 静默改读新正文，当前采用来源变化明确失败的语义。
- Nomi manager 把 descriptor 交给原 Bootstrap，仍使用标准 Skill 索引/工具。显式包 ID 优先于同名目录，新增 Skill 工具仅用于所选 Skill；不会把依赖 capability 自动加入计划或暴露隐藏 action。app 的包内 ID 目录投影退出，非包内目录 Skill 保留。
- 只读入口支持参数替换和声明资源读取（UTF-8 或 hex），严格解析 frontmatter；shell/fork/hooks/model/tool override 等尚未接入受管执行的模式明确拒绝，参数不能引入 shell 插值。该切片不是所有 Skill 模式完成，也不把资源文件含脚本当作获得执行授权。

新增验证覆盖真实包导入/安装与正常保存/回读、精确正文/资源、标准 Skill 工具、Bootstrap 索引、同名目录优先级、制品变化/撤下、IO 等待期间撤下、文件缺失/篡改、路径/数量/大小边界，以及同正文制品更新触发重新保存。安装测试使用真实 Node/受管制品及生产编译接缝，authoring 存储为现有 InMemoryControlPlaneStore；Bootstrap 使用捕获请求的测试模型，不依赖外部服务，也不将其称为用户真实模型验收。

| 验证 | 结果与边界 |
|---|---|
| `cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features` | 最终 7 项通过；现有真实安装测试扩展 Skill 包内正文/资源、正常 save/read、策略注册及撤下；`.tmp-open-skills-product-verified.out/.err` |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 最终 17 项通过，含新增 4 项 Skill 来源/制品漂移、读取失败、Session 身份和合同/consumer/依赖检查；`.tmp-open-skills-consumer-verified.out/.err` |
| `cargo test -p nomi-agent --lib skill --no-default-features` | 72 项通过，含新增标准 Skill 工具读取/同名优先/逐次来源守卫，以及严格 frontmatter/参数 shell 拒绝；原目录 inline/fork/副作用相关回归保留 |
| `cargo test -p nomi-agent --test bootstrap_test --no-default-features` | 17 项通过，含新增真实 Bootstrap 索引不使用同名目录的场景 |
| `cargo test -p nomifun-plugin-platform --test artifact_store --no-default-features` | 11 项通过，新增声明文件读取、路径/重复/大小/总量/数量/摘要与已发布文件篡改检查 |
| `cargo test -p nomifun-agent-control-plane -p nomifun-agent-kernel -p nomifun-agent-contracts --lib --no-default-features` | 最终 contracts 87、Control Plane 37、Kernel 31 项通过；含同正文制品变化使 clean-save 重新编译、撤下拒绝及旧 Snapshot 不变 |
| `cargo test -p nomifun-app --lib router::nomi_core_agent_projection::tests --no-default-features` | 19 项通过，保留非包内 Skill 和既有投影回归 |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write`；新构建工具 `target/debug/agent-v2-contract.exe check` | 生成与检查通过；schema/digest 及 runtime/platform fixture 引用同步，不表示重新完成 release 跨平台验证 |

首次新包测试缺少 `consumer:agent` 声明，按原 N1 合同修正 fixture 后复跑通过，没有放宽准入。新文件定向 rustfmt 与最终差异检查通过；本批未修改 UI，因此未运行 UI boundary/视觉测试。未运行全仓测试、真实外部模型、跨平台、旧数据库迁移或完整 Skill 执行模式验收。

05 §3.3 已同步合同演进：旧格式非空 Skill Snapshot 没有来源默认值，不自动迁移或清库；若需保留历史数据，仍要补迁移/制品保留验证。目录 Skill 全来源统一、slash 命令完整体验、脚本/fork/hook 权限与取消、跨版本制品保留仍是后续工作，不能因此关闭编号 07 或 P3/P7 的完整范围。

### 2.9 已实施切片：包内只读 Skill 的显式命令与输入框补全

重读发现：§2.8 的描述已进入 Bootstrap/Skill 工具，但 Engine 命令注册表只有内置命令；产品 manager 缓存的 slash 列表因此不含包内 Skill。前端查询正则又不接受点分 ID。该切片沿现有命令列表 API 与同一 descriptor 接通，不创建 Skill 命令目录或第二个执行 owner。

- 命令统一为 `/skill:<精确ID> [参数]`，原 Tool 调用 ID 不变。即使插件 Skill 名叫 `help`、`exit`、`copy` 或 `open`，也不抢占引擎/UI 命令；frontmatter 名称不成为执行别名。
- `SkillTool` 与命令共享 `Arc<HostSkill>` 和 deny checker。用户不可调用/被 deny 的条目不显示，但显式输入仍失败；包内禁用模型调用的条目不会因为猜中 ID 就能被模型工具执行。未知 `skill:` 名称明确失败，不让 LLM 猜测使用方式。
- Engine 检查真实注册工具、请求级 allowlist 与 plan-mode 范围，然后调用同一来源/依赖守卫。正文只附加到原用户 turn，不覆盖系统提示、不递归启动引擎，原 source-message 与 rewind 身份保留；读取等待取消、撤下和参数引入 shell 均不产生后续模型请求。
- 前端沿已有 `useSlashCommands → SendBox → useSlashCommandController` 消费列表，支持命名空间/点分 ID 查询与精确插入，选中只填入命令、不自动执行 UI builtin。没有新增页面、API 或移动布局。

验证已覆盖 Bootstrap 完整调用/元数据、只对用户开放的 Skill、deny/user-invocable、受限 turn、同名目录/内置/UI 名称、缺失命令、来源撤下、等待取消、参数 shell 拒绝，以及真实 Kernel 锁 → Session 装载 → Bootstrap → 捕获模型请求 → Kernel 撤下拒绝。后者使用受控制品 resolver fixture，不冒充磁盘安装/外部模型/真实 WebView 全链路验收；实际包安装的既有回归另行复跑。

| 验证 | 本批结果与证据范围 |
|---|---|
| `cargo test -p nomi-agent --test bootstrap_test --no-default-features` | 最终 23 项通过，新增 6 项命令集成测试；`.tmp-open-skill-commands-bootstrap-final.out/.err` |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 18 项通过，新增 1 项真实 Kernel/Session 描述进入 Bootstrap 命令并在撤下后拒绝；`.tmp-open-skill-commands-consumer.out/.err` |
| `cargo test -p nomi-agent --lib skill --no-default-features` | 72 项通过，保留目录/包内 Skill 与原脚本/fork 回归；不能据此声称包内脚本/fork 已开放 |
| `cargo test -p nomi-agent --lib command --no-default-features` | 最终 21 项通过，含新注册冲突检查和旧 help/clear/quit/未知命令行为；`.tmp-open-skill-commands-command-final.out/.err` |
| `cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features` | 7 项通过；复跑真实 Node 安装、保存/消费及默认/跨包恢复接缝，不是新增真实模型命令验收；`.tmp-open-skill-commands-app.out/.err` |
| `bun test --cwd ui src/renderer/hooks/chat/useSlashCommandController.interaction.test.tsx` | 新增 2 项通过，验证点分命名空间查询、参数结束补全、精确插入且不执行 UI builtin |
| `bun run typecheck`、`bun run check:desktop-ui-boundary`、`git diff --check` | 通过；保持 880×600 桌面合同，未运行全仓、真实 WebView、外部模型、跨平台或数据库迁移验收 |

前一轮 9 项命令测试及首次 Bootstrap 23 项是中间结果；上表最终日志包含统一 `skill:` 命名空间与未知 Skill 明确失败。新增 Rust 文件定向格式化，既有大文件不做全仓格式重写；未提交或清理既有无关工作区改动。

本批没有改 Snapshot wire schema、新增安装后端或开放受管 shell/fork/hook。本批交付时命令元数据仍在 manager 启动时捕获；后续冷启动与附件输入改动见 §2.16～2.17，不以旧状态描述最新代码。来源统一、剩余执行模式和历史制品保留/迁移仍未关闭。普通非 `skill:` 未知命令及传统目录 Skill 保持原路径。07 与完整 P3/P7 仍为部分完成。

### 2.10 实施切片：显式阶段的动态 Context 与每轮真实消费

当前重读发现：原 `context_contribute` 只传输出 schema；Nomi 虽有动态 Context 接缝，用户包只在 Session materialize 时被调用。该批沿原合同和执行链补齐，不创建新的插件平台、Context Registry 或 Agent owner。

- `CapabilityContributions.context_phase` 显式声明 `session_start` / `before_turn`，默认值序列化时省略，旧 manifest 摘要不变。N1 与 Kernel 拒绝 Tool/非 Agent 能力声明 `before_turn`；Role façade 和 implementation 的阶段必须相同，不能让 Provider 私自改变消费时机。
- `ContextContributionInput` 从 Kernel 传到 direct/Role factory，再经原 Host 请求到 JS `contributeContext({ schemaRef, contribution, input, signal })`。每轮输入只有当前 source message ID、文本、图片 MIME 列表，无图片字节或任意 host context/授权字段；严格拒绝额外字段和超过 256 KiB 的输入。
- Nomi Session 为已选的动态 Context 建立同一 Kernel 消费适配器，启动时不执行、不作为 Tool 公开；每次主模型推理前重新获取贡献，同一用户轮次的工具后续推理也会执行。结果只用于当前请求的 system prompt，不写进历史消息或累加上轮贡献；不是一次性用户消息 hook，也不是完整 Prompt/压缩管线替换。
- 每次调用重查冻结 Snapshot、精确实现、当前权限和来源。失败、超时、撤下不跳过贡献或回退内置。初始 Context 批次与动态 Context 批次分别共享 5 秒 deadline、64 KiB 结果限制；这更新 §2 当时“每个初始贡献各 5 秒”的实现，但不是整个 Session 启动或所有 middleware 的总预算。
- 取消沿既有 Host request ledger / JS AbortSignal。真实 Node fixture 验证放弃动态请求后 JS 收到取消，仍不承诺强制终止不协作的 Node 运算。产品 app 仍只经 `nomifun-ai-agent` 接缝消费，无新增 `nomi-*` 直接依赖。

最终定向验证：

| 检查 | 结果与证据 |
|---|---|
| `cargo test -p nomifun-js-kernel-adapter -p nomifun-agent-kernel -p nomifun-agent-contracts --no-default-features` | contracts 89、Kernel 31、adapter unit 2、真实 Node integration 14 项通过；`.tmp-open-turn-context-verified.out/.err`。覆盖 legacy 默认序列化、非法阶段、Role 阶段不匹配、direct/Role 动态输入及 JS 协作取消 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 最终 22 项通过；`.tmp-open-turn-context-consumer-verified.out/.err`。新增 4 项验证实时输入、输入/结果上限与错误、5 秒 deadline，以及真实 Nomi Engine 连续两轮请求/不累积/撤下拒绝 |
| `cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features` | 最终 7 项通过；`.tmp-open-turn-context-app-final.out/.err`。真实包导入/安装 → Catalog → Agent 正常 save/read → 生产 Snapshot 校验 → Session 动态 Context，在两次输入中消费已安装 JS；原 Provider/Skill/恢复回归保留 |
| `cargo test -p nomifun-js-host --lib --test extension_host --no-default-features` | Host unit 16、integration 73 项通过；`.tmp-open-turn-context-host.out/.err`，覆盖已有请求取消、背压、资源/服务与进程回收 |
| canonical contract `write` 后 `check`、两处 JS 的 `node --check`、`git diff --check` | 通过；生成 schema/摘要和 05 的 Context 阶段合同已同步，无新增依赖或第二套运行协议 |

首轮 consumer 21 项及中间编译日志不代替上述最终结果；Host 首次前台调用因工具 60 秒限制中断，已改为隐藏后台完整复跑并通过。没有执行外部模型、真实 WebView、跨平台或全仓发布验收；安装/save/read 使用受控产品测试存储，不冒充重启磁盘数据库的迁移验证。未改 UI renderer，因此不运行与本批无关的 UI 构建。

本批不关闭 06 或完整需求 3/5：用户自定义排序、跨全部贡献的预算、可替换完整 Prompt 管线、任意 middleware 仍未完成。只改阶段和类型化输入不等于模型/记忆/规划/Runtime 已开放；其他 29 行继续按原范围实施。

### 2.11 已验证切片：用户 Context 顺序从工作台到冻结执行

重读发现：已有初始贡献在消费后按 capability ID 排序，动态贡献继承 Compiler 的 ID 顺序；仅调整 UI 列表无法改变执行。该批在原 Agent document / Revision / Snapshot 中加入可省略的 `context_order`，不建立第二套 Prompt 配置、排序存储或执行 owner。

- 公共 DTO 与 canonical Revision 使用同一字段。顺序只能引用本 Agent 直接选中的 Agent ContextContributor，重复/未选择/Tool 等条目在保存编译时拒绝，不自动添加能力、依赖或授权。Role 使用 façade 的公开 ID，实际实现仍由同一冻结 Provider lock 选择。
- Compiler 冻结顺序并覆盖 Revision、Snapshot 和 runtime profile 摘要；空值省略，旧 payload/Snapshot/profile 的 canonical 空顺序不增加字段。无改动保存检查顺序投影，变更顺序生成新快照，不改变旧会话、所选 Provider 或权限集合。
- Nomi 按显式列表优先、其余 ID 顺序调用并排列 Context。启动与动态阶段分别消费同一顺序，不能把启动贡献移动到推理阶段；显式排序但未获该 Runtime 准入的贡献报错，不静默忽略。原来源/撤销检查、5 秒批次预算、64 KiB 结果上限和协作取消保留。
- 工作台 Agent 设置新增上下移动与恢复默认顺序，经原保存入口提交。Catalog 缺项保留显式选择，主动移除 capability 时清理对应顺序项；不修改 Tool 集排序，不创建新的配置页面链路。中英文说明明确该顺序不控制完整 Prompt 或权限。

| 检查 | 最终结果与范围 |
|---|---|
| `cargo test -p nomifun-agent-contracts -p nomifun-agent-kernel -p nomifun-agent-control-plane --lib --no-default-features` | contracts 90、Kernel 31、Control Plane 38 项通过；`.tmp-open-context-order-core-final.out/.err`。包含可省略/唯一/已选择约束、摘要覆盖、顺序保存与复用、冻结 Provider/权限不变 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 最终 25 项通过；`.tmp-open-context-order-consumer-verified.out/.err`。初始/动态调用与结果顺序、默认稳定顺序、旧快照、撤下和显式未准入拒绝；实际 Nomi Engine 连续两次模型请求中验证两个阶段顺序及动态内容不进入历史消息 |
| `cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features` | 7 项通过；`.tmp-open-context-order-app.out/.err`。真实安装的双 JS 动态 Context 经正常 save/read、持久化 Snapshot 和生产 Session 校验后按指定顺序消费；直接/Role 单贡献和原 Skill/恢复回归保留 |
| `bun test --cwd ui` 的 ContextOrder、RoleProviderPicker 交互及 capabilityChanges 三个文件 | 15 项通过，其中新增顺序 4 项；`.tmp-open-context-order-ui-verified.out/.err`。包含个人编辑器实际保存回调、缺项保留、禁用编辑及能力移除。SWR fixture 隔离无关模型目录网络请求，无真实服务依赖 |
| `bun run typecheck`、`bun run check:desktop-ui-boundary`、`bun run check:i18n` | 通过；类型检查最终退出 0，维持 880×600 桌面合同；i18n 生成类型同步并通过一致性检查 |
| canonical `agent-v2-contract write` 后 `check`、`git diff --check` | 通过；生成 schema/摘要及引用 fixture 同步，05 正式顺序合同已更新 |

第一次补充未准入测试使用了错误 helper 名称，修正后最终 25 项通过；首轮 24 项不代替最终结果。UI 测试存在 React/Arco 的 ref/测量警告，不将 DOM 测试记为真实 WebView 或视觉验收。安装测试复用既有受控产品存储，不代表磁盘数据库跨进程迁移/恢复验收；未运行外部模型、全仓发布或跨平台检查。其他 crates 的新增空顺序字段仅用于既有 struct literal 兼容，不表示开放了对应业务域。

本批关闭的是 06 的 Context 贡献排序子项，不关闭完整 Prompt 管线：persona、内置 lifecycle、历史选择/压缩、其他 middleware 的相对位置和跨全部贡献预算仍待开放。完整 29 行目标保持不变，也没有新增 Rust/native 插件后端。

### 2.12 已验证切片：Provider 独立冲突声明与实际组合检查

重读 `role_implementation_matches`、Compiler 的闭包/选择顺序和 JS 调用后确认：不同内部 `requires` 的开放仍需要消费用途与父调用授权/受管子调用，不能直接删除检查。`conflicts` 则不授予执行权限，可以先让用户实现独立声明，并通过真实组装检查闭环。该子项属于 P2/P7，不替代递归依赖和其余 29 行。

- Materializer 不再要求实现和 façade 的冲突清单相等，callable schema/effect 等合同仍保持兼容。Compiler 在所选 Provider 确定后检查公开贡献、所选实现及实际使用的隐式资源工厂；反向指向内部实现的冲突、跨 Role/普通贡献冲突也在同一次检查中处理。
- façade 的冲突保留为共同契约约束；实现可以有自己的私有冲突，不把未选中默认实现或未消费成员的限制带入新选择。仅属于默认实现的限制应放在实现中，不冒充所有实现必须遵守的契约。
- 临时消费集合不生成 enabled capability、allowlist 或 policy；没有新增授权、内部 Tool 暴露、RPC、快照字段或注册中心。安装默认只验证候选可用，不强制成员同时使用；Agent 编译/非 Agent operation 准入按真实选择检查，拒绝发生在副作用之前。
- Control Plane 的无改动保存通过 Kernel 同一检查拒绝早期漏检的组合，不更改旧 Revision/Snapshot。实际产品安装 fixture 中，普通 Context 与其 façade 的冲突声明不同；冲突组合正常保存失败，不冲突的原保存/读取/生产 Nomi 消费仍通过。

最终验证记录：

| 检查 | 结果 / 覆盖 |
|---|---|
| `cargo test -p nomifun-agent-kernel -p nomifun-agent-control-plane --lib --no-default-features` | Kernel 31、Control Plane 41 项通过；`.tmp-open-provider-conflicts-core-complete.out/.err`。新增跨普通贡献、跨两个 Role 和正常 save/store 不改旧版本的检查；旧漏检计划是刻意构造的负例，不是已完成历史数据迁移 |
| `cargo test -p nomifun-js-kernel-adapter --test kernel_adapter --no-default-features` | 19 项通过；`.tmp-open-provider-conflicts-adapter-verified.out/.err`。新增 5 个测试覆盖差异准入/真实 Node 调用、选中/未选中、反向内部目标、隐式资源、非 Agent 准入、共同契约冲突保留；同时验证设置默认不等于全部成员同时消费，以及内部直接调用/撤下拒绝 |
| `cargo test -p nomifun-app --lib router::plugin_platform:: --no-default-features` | 最终 7 项通过；`.tmp-open-provider-conflicts-app-final.out/.err`。实际安装的独立冲突声明通过 materialize，正常保存拒绝冲突组合且诊断指明双方；合法组合继续 save/read/生产 Nomi 消费，并保留跨包恢复和默认管理回归 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features` | 25 项通过；`.tmp-open-provider-conflicts-consumer.out/.err`，保留 Context/Skill/Tool/撤下等原有消费行为 |
| `agent-v2-contract check`、`git diff --check`、文档链接/围栏/29 行检查 | 通过；本批不改 schema 或 wire 字段，未重写生成合同；两份 29 行清单完整保留 |

初次 Control Plane 测试遗漏局部 import，补齐后上述 41 项通过。产品测试最初只断言失败，加强诊断断言后发现跨包恢复夹具没有安装冲突对端；现由完整安装场景显式传入冲突验收，要求对端已 materialize 并核对双方诊断，恢复夹具只验证其最小包集合。最终 7 项通过；首轮绿灯及中间 `.tmp-open-provider-conflicts-app-complete` 的失败均不代替最终证据。没有新增 UI 或 native 插件后端；未进行真实 WebView、外部模型、跨平台或磁盘数据库迁移验收。冲突开放不解决任意 capability 递归依赖、受管 JS 子调用、产品自定义 kind、多 binding、资源竞态和全部领域消费者；这些继续按原范围实施。

### 2.13 Kernel 父作用域依赖调用：已有定向证据的前置工作

本条及 §2.14 最初由五问报告复核补记工作区既有代码和日志；随后实施继续完成夹具修复与定向验证，最新结果单列在 §2.14 末尾，不将历史文档轮次算作执行证据。它们推进 02 的依赖执行基础，不关闭异构 Provider 或完整需求 1/3/5。

- `CapabilityDependencyCaller` 绑定父调用身份、同一冻结 Snapshot 和祖先链；Nomi Tool 通过 `invoke_shared` 复用该 Snapshot，不复制第二套调用计划。
- 只允许所选实际实现声明的直接依赖，子调用继续走原 Kernel 的来源、动作、资源和授权检查；祖先撤下/来源变化也要重新检查。插件不能自报 owner 或以依赖声明提升权限。
- 父调用结束后，保留的 caller 失效；Registry 弱引用避免保留 callback 形成所有权环。调用 key 非空且最多 128 UTF-8 字节，每个父调用最多接受 1024 个 key，重复 key 和祖先循环被拒绝。
- 派生 operation/effect 标识复用父身份命名空间，不提供持久化 exactly-once 保证；取消不会回滚已提交副作用，也不默认重试。

代码位置：[Kernel 接缝](../../crates/backend/nomifun-agent-kernel/src/dependency_call.rs)、[Kernel 测试](../../crates/backend/nomifun-agent-kernel/src/dependency_call_tests.rs)、[Nomi 消费测试](../../crates/backend/nomifun-ai-agent/tests/plugin_tool_consumer/dependencies.rs)。既有 `.tmp-open-js-dependencies-core.out/.err` 记录 contracts 91、Control Plane 41、Kernel 44 项通过；`.tmp-open-js-dependencies-consumers.out/.err` 记录 Nomi consumer 26 项通过。它们不是整个 JS 集成的绿灯，该日志中的 adapter 失败见下条。

当前仍限于 Agent Tool 调用，`requires` 仍要求 façade 与实现相等。编译计划尚未区分仅供内部依赖与向模型公开的贡献，也未解析任意不同实现的递归依赖。因此不能把“父作用域调用能执行”当作“异构依赖已可组装”。

### 2.14 JS 受管依赖调用与父租约复用：实现及验证收口

工作区已有以下接线，不应继续表述为“JS Host/SDK 完全未接入”，也不能以早期局部绿灯记为完成：

- canonical `PluginDependencyCall` / `DependencyInvoke` 通过原 Host wire 传递；Tool 的 `dependencies.invoke({ capabilityId, actionId, callKey, input })` 由闭包绑定父请求与 Mount，不允许 caller 自报身份/授权。
- Host 在原 pending request 与 service task 生命周期内验证父请求、精确 Mount/generation、存活状态和 deadline；结束、放弃或取消时关闭 callback。没有增加第二套请求账本或让 Mount 激活 SDK 自动获得依赖调用权限。
- direct Tool 与 Role Tool 都通过 `NodeToolHandler::invoke_scoped` 回到原 Kernel。Context、非 Agent operation、任意后台任务并未因此开放同等 API；错误映射不把其他插件内部诊断直接暴露给调用者。
- 产品 `RuntimeBoundExtensionHost` 的受管 callback 显式携带同一 Host 的 `RuntimeUseLease` 和 supervisor，避免切换写锁已排队时嵌套重取读锁的等待环。其他 Host、脱离调用的任务不能借用该租约；没有新增 runtime manager。
- authoring scaffold 已增加调用 DTO 与 Tool invocation 类型；正式 05 §5.10、06 的增量说明已有对应描述。新声明、生成合同和最终集成验证仍需收口，不能只依据代码存在核销。

五问复核时的既有日志状态（历史失败保留；后续修复/复跑见下方）：

| 检查 / 日志 | 结果与证据边界 |
|---|---|
| Host/adapter lib：`.tmp-open-js-dependencies-lib.out/.err` | 16 / 2 项通过，是较早代码时点；不覆盖后续新增 SDK/测试修改 |
| Host dependency 定向：`.tmp-open-js-dependencies-host.out/.err` | 当时新增 4 项通过；后续扩充为 6 项，不能把旧结果套用于新版本 |
| adapter dependency 定向：`.tmp-open-js-dependencies-adapter.out/.err` | 早期同包 direct/Role 用例通过；跨包配置是后加的，不在该次证据内 |
| 最后一次 Host 集成：`.tmp-open-js-dependencies-host-final.out/.err` | 76 项通过、3 项超时：`abandoning_parent_drops_its_child_and_keeps_the_generation_usable`、`dependency_callback_cannot_outlive_the_parent_watchdog_deadline`、`pending_activation_blocks_only_its_mount_commit_fence`；原因尚未经复跑确认，不能直接归为机器争用 |
| 最后一次 consumer/adapter：`.tmp-open-js-dependencies-consumers.out/.err` | Nomi 26 项通过；adapter 19 项通过、1 项失败。跨包 fixture 删去 contribution 后仍保留无引用 schema，制品构造被严格校验拒绝，未进入该跨包调用。不是已证实的运行期跨包失败，也不是可忽略的绿灯 |
| 产品 runtime Host：`.tmp-open-js-dependencies-app.out/.err` | 5 项通过，含真实 Node、预先排队的切换写锁、正常调用与取消后租约释放；另验证其他 Host/脱离任务不能复用。不是完整真实安装或整个产品链验收 |
| contract generator：`.tmp-open-js-dependencies-schema.err` | 记录运行了 `target/debug/agent-v2-contract.exe write`，不是 `build.noindex` 下二进制；不能据此认定最终生成物已通过 `check`，后续应核对实际构建输出与合同一致性 |

后续实施已修正跨包 fixture：子包只保留自身 action 引用的 schema，不放宽生产制品校验；同包/跨包 × direct/Role 四种配置都执行真实 Node 多跳调用。另加“父包不变、仅子包 source digest 改变”的场景，要求错误由依赖调用返回且不执行 builtin，区分根准入拒绝与真正的子调用来源检查。

两个新生命周期测试先完成 Host/Mount 装载，再测父调用放弃与 watchdog；保留原 500 ms watchdog、调用计时上界和回收断言。首次受控并发复跑在调整前即通过 79 项，不能把该调整说成修复了已证明的产品死锁。没有修改生产 timeout、旧 startup/activation 测试或全仓测试并发配置。

本次实施的验证记录：

| 检查 | 结果与范围 |
|---|---|
| `cargo test -p nomifun-js-host --lib --test extension_host --no-default-features -- --test-threads=2` | 最终 unit 16、integration 79 项通过；`.tmp-open-js-dependencies-host-final-bounded.out/.err`，包含计时调整后的六项依赖用例和原取消/背压/资源/进程回收；不等于默认高并发运行也无启动敏感性 |
| `cargo test -p nomifun-js-kernel-adapter --lib --test kernel_adapter --no-default-features -- --test-threads=2` | unit 2、integration 20 项通过；`.tmp-open-js-dependencies-adapter-verified.out/.err`。包含上述四配置和子包单独来源漂移；不代表不同 `requires` 已可组装 |
| Nomi consumer：`cargo test -p nomifun-js-kernel-adapter -p nomifun-ai-agent --test kernel_adapter --test plugin_tool_consumer --no-default-features -- --test-threads=2` | consumer 26 项通过，`.tmp-open-js-dependencies-consumers-fixed.out/.err`；同次 adapter 20 项是加子包单独漂移断言前的中间结果，以上一行最终版本为准 |
| `cargo test -p nomifun-app --lib router::plugin_ --no-default-features -- --test-threads=2` | 29 项通过；`.tmp-open-js-dependencies-product-final.out/.err`，含原安装/保存/恢复 7 项、runtime Host 5 项及产品/路由回归。受控本地测试不是外部模型、跨平台发布或磁盘数据库迁移验收 |
| contracts / Control Plane / Kernel `--lib --no-default-features` | 91 / 41 / 44 项通过；`.tmp-open-js-dependencies-core-verified.out/.err` |
| authoring `--test authoring_foundation --no-default-features` | 15 项通过；`.tmp-open-js-dependencies-authoring.out/.err`，包含新 JS/TS 脚手架中的 Tool invocation 与 Mount SDK 隔离断言 |
| TypeScript compiler 内存类型检查、五处 JS `node --check` | 通过；合法 `dependencies.invoke` 可编译，额外 owner 字段和 Mount SDK 的 `dependencies` 不可用；不把静态类型当运行期授权 |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`，随后直接复核同一产物 | 通过；实际二进制为 `target/debug/agent-v2-contract.exe`，`build.noindex` 是中间构建目录，已按 `.cargo/config.toml` 核对，无需猜测另一份二进制 |

Host 并发证据须单独说明：初次两线程全套 79 项通过（`.tmp-open-js-dependencies-host-bounded`）；修正测试计时后，默认高并发且与其他集成同时运行的一次为 77 通过 / 2 超时，分别是既有 `pending_activation_blocks_only_its_mount_commit_fence` 和 `stderr_is_drained_before_waiting_for_hello`（`.tmp-open-js-dependencies-host-verified`），新增六个依赖用例均通过。最终两线程全套按上表复跑，16/79 项通过；这个默认并发的启动敏感性仍保留为测试风险，不宣称零 flaky，也不根据两线程绿灯证明任意压力下都稳定。

本切片结束时的下一步是递归依赖/消费用途，后续进展见 §2.15，不再以此历史描述判断当前代码。`dependency_path` 仍只是诊断路径；Role 选择可能引入原始 manifest 图不存在的环，必须检查实际选中图。不新建 JS 专用快照，也不把未选中的所有 Provider 加入执行计划。本切片不关闭 02 或其余 29 行。

### 2.15 递归选中图与公开/内部消费：同一 Snapshot 的跨层演进

在 §2.14 的受管调用基础上，允许同一 Role 的不同实现声明不同 `requires`，不再要求
复制内置实现的内部依赖。所有变更沿原 Catalog/Compiler/Snapshot/Session owner：

- `compiler_dependencies.rs` 在 canonical Compiler 内按所选 Provider 递归展开，包含
  façade、映射实现与实际使用的隐式资源工厂需求。依赖本身是 Role 时复用原选择器；
  精确版本缺失与选择引入的循环明确失败。使用迭代遍历，不递归增长 Rust 调用栈。
- 原 `ResolvedCapability` 记录冻结 `consumption` 与 `dependency_refs`。显式选择为
  Contribution，其余为 Dependency；同一能力兼有两种用途时仍只有一条公开记录。
  `contributions()` 是原快照的投影，不是第二份计划、授权或激活注册表。
- 图结构校验覆盖精确边、重复边、循环、allowlist 一致性与公开根可达性。内部依赖
  不自动进入 Context 顺序或公开 MCP 映射。PluginProduct 投影不得自报内部图字段。
- Nomi Tool、初始/动态 Context 和 lifecycle 的六处消费改用公开贡献；公开 Tool、
  direct/Role Context 在任何执行/资源副作用前检查用途。隐式资源工厂继续由原租约
  准入管理，不要求它为了内部执行而成为公共能力。
- 受管子调用同时核对精确父实现的声明与冻结父记录直接边；还保留来源、祖先有效性、
  操作和资源授权、父调用生命周期。图中有记录不授予任意平级/祖先调用权。
- clean-save 重新计算同一个选中图并比较完整记录/Role 锁/资源需求，不能只因 façade
  或 Provider lock 相同就复用旧图。既有应用/测试构造点已补字段，不新增手写 JS 图副本。
- `Contribution` 默认值和空边不序列化，保留旧记录摘要；这是读取兼容，不是历史图
  迁移。旧 Session 仍使用冻结绑定，当前产品 open 在重新编译结果不同时明确拒绝。
  不自动改写旧数据，不把这个拒绝行为称为所有旧会话已可继续使用。

重新执行的验证（2026-09-14；不是沿用 §2.14 绿灯）：

| 命令 / 范围 | 结果与边界 |
|---|---|
| `cargo test -p nomifun-agent-contracts -p nomifun-agent-kernel -p nomifun-agent-control-plane --lib --no-default-features -- --test-threads=2` | contracts 93、Control Plane 41、Kernel 47 项通过；含选中图循环、内部调用/公开拒绝、过时图复用及递归 Role 默认/override；日志 `.tmp-open-graph-core-final` |
| `cargo test -p nomifun-js-kernel-adapter -p nomifun-ai-agent --test kernel_adapter --test plugin_tool_consumer --no-default-features -- --test-threads=2` | adapter 21、Nomi 27 项通过；含同包/跨包 × direct/Role 的真实 JS 多跳调用，内部 Context 在公开入口被拒绝且没有 Node 激活，实际 Nomi 不消费私有 Context/Tool；日志 `.tmp-open-graph-consumers-final` |
| 原 adapter 失败的处理 | 删除“dependency 差异必须拒绝”的过时负例；schema/effect/resource 不兼容负例仍保留。不是恢复相等限制或放宽对外行为契约。此前 19/1 失败记录留在 `.tmp-open-graph-consumers` |
| canonical 生成及检查 | `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write` 完成生成；命令等待窗口在生成器启动后超时，随后直接执行同一 `target/debug/agent-v2-contract.exe check` 成功。生成 schema/摘要与源夹具同步，05 §5.5 同步；未增加 Node wire 方法 |

后续五问复核轮次（仅更新文档与验证记录，不新增功能代码）取得新增用例结果：

| 命令 / 范围 | 结果与边界 |
|---|---|
| `cargo test -p nomifun-app --lib router::plugin_ --no-default-features -- --test-threads=2` | 30 项通过，日志 `.tmp-open-graph-app-fixed`；含 `installed_heterogeneous_providers_freeze_private_graphs_across_save_open_and_withdrawal`。此前首次编译的错误导入已由既有代码修正，本次结果来自修正后的代码 |
| `cargo test -p nomifun-js-kernel-adapter --test kernel_adapter --no-default-features -- --test-threads=2` | 22 项通过，日志 `.tmp-open-graph-adapter-final`；新增隐式资源工厂依赖用例验证缺 runtime feature 拒绝、精确内部依赖入图但不成为公开贡献、真实 JS 资源消费与释放 |
| `target/debug/agent-v2-contract.exe check` | 再次执行退出码 0；不是仅沿用生成时的结果 |

产品用例经真实包导入/测试/安装、SQLite 控制面保存、生产 Nomi Snapshot 准入与
materialize、Kernel 真实 Node 子调用，验证 A/B 实现分别锁定自己的私有依赖，只有
一个公开 Tool，私有 Context 不进入消费；clean-save 复用，切换后旧计划仍调用 A，
撤下 A 后旧计划打开/调用失败而 B 可用，存储中的旧 Snapshot 不变。
旧格式 envelope 的验证仅证明结构合法但语义不符时明确拒绝，不证明历史图已迁移。
这些是产品接缝与受控包的测试，不是外部 LLM、真实 Browser/Computer、UI 或全产品进程验收。

剩余范围：非 Agent operation/Context/资源工厂发起受管子调用、发布型 Service 的依赖
扩展、产品任意 resource kind/multi-binding、租约竞态、全部领域替换与历史计划迁移。
当前 Node 仍不是强沙箱，SDK 授权只约束受管调用。平台 owner-scoped capability catalog
保留完整执行/active 事实，不把它误改成模型工具清单；生产 Nomi 的公开消费由上述用途
投影与入口校验负责。本节不关闭需求 1/3/5 或 29 行中的完整范围。

### 2.16 已验证切片：包内 Skill 冷启动命令发现

前次文档复核将本项记为在途；本轮重新阅读代码后修复产品夹具和 UI 测试类型，并取得实际产品链路证据。已有代码沿原查询链推进：

- `useSlashCommands` 不再等待非空 Agent status，切换会话时清除旧建议，空结果/错误移除缓存；状态变化后重新查询。
- Conversation 先验证会话归属；原 `AgentRuntimeRegistry` 有 live runtime 时以其结果为准（包括空列表），无 runtime 时查询原 Session Provider 的 `discover_skill_commands`，不创建运行时。
- 产品 Provider 提取 `compile_request` 复用已保存绑定与原 Compiler 校验；发现不调用有 Context/资源物化行为的 `resolve()`。共享 `load_package_skills` 继续校验精确来源/正文/资源，过滤 `user_invocable` 并返回命令描述，不新增 Skill Registry、配置组装器或活动能力集合。
- 发现 descriptor 没有 active state，`authorize()` 明确拒绝执行。命令建议不等于 grant；实际 Runtime 的配置/deny、活动能力与来源检查仍决定能否使用。冷列表不是所有来源的完整命令列表，也不保证与 live 列表相同。

| 检查 / 留存证据 | 当前可确认的结果 |
|---|---|
| `cargo test -p nomifun-ai-agent --lib --test plugin_tool_consumer cold_ --no-default-features -- --test-threads=2`，`.tmp-open-skill-cold-core.out` | Registry 1 项、consumer 1 项通过；覆盖发现不建 runtime/物化 Session，以及精确制品、隐藏命令、不支持模式、摘要错误和 IO 中撤下 |
| 产品夹具的历史失败 | 起初使用不存在的 `CapabilityKind::Context`；修正后 HTTP 请求又因缺 `persona` 被拒绝。现已改用 `ContextContributor` 和正式请求/响应 DTO；未放宽生产校验 |
| `cargo test -p nomifun-app --test official_preset_catalog_integrity --no-default-features -- --test-threads=2`，`.tmp-open-skill-cold-product-final.out` | 2 项通过：冷启动用例及官方模板目录安装/刷新回归 |
| `cargo test -p nomifun-app --lib router::plugin_ --no-default-features -- --test-threads=2`，`.tmp-open-skill-cold-app-regression.out` | 30 项通过，含原安装、保存、恢复与运行接缝；该记录在后续 §2.17 的引擎输入改动前取得 |
| `bun test --cwd ui src/renderer/hooks/chat/useSlashCommands.interaction.test.tsx src/renderer/hooks/chat/useSlashCommandController.interaction.test.tsx` | 6 项通过，25 条断言；冷查询/状态刷新、会话切换、旧响应隔离、空缓存与命令插入 |
| `bun run typecheck`；`bun run check:desktop-ui-boundary` | 均通过。初次 typecheck 暴露测试 `initialProps.status` 推断为仅 `null`，修为 `string \| null` 后通过；桌面边界 1923 源文件、880×600 |

产品用例验证真实安装与持久绑定，连续两次 HTTP 查询均不激活会写标记并报错的 JS 插件/Context、不创建 runtime、不改 Snapshot；错误 owner 被拒绝，撤下已安装插件后查询返回 Conflict，不使用旧缓存或目录回退。这是实际产品组合与本地 HTTP 路由证据，不是外部模型或 WebView 验收。当前检查仍复用重编译并读取包文件，不能算作“退出重复编译”或零成本发现。

本节只涉及包内只读命令建议。附件/宿主装饰输入的后续切片见 §2.17；其余来源、完整权限提示、shell/fork/hook、历史制品保留与迁移仍按报告 §18.4 分开交付，07 和完整需求 4 保持部分完成。

### 2.17 实施切片：附图与宿主装饰输入中的显式 Skill

重新阅读实际消费发现：引擎原先仅拦截单个文本块，图片会使 `/skill:<ID>` 退化为普通模型输入；生产 manager 的知识库前缀还可能遮住命令。本轮沿原 Engine turn 入口修复，而不是另建命令执行器：

- 对带图/附加文本的输入，只从首个用户文本识别显式包内 Skill；附件与后续上下文不被扫描为命令，也不混入 Skill 参数。原始内容保留，解析后的正文仍追加为用户级内容。
- 现有宿主 turn 接缝增加可选的本次用户原文参数。生产 manager 在首个 pass 传 `data.content`，检索装饰只供模型阅读；steering race-tail 传空串禁用命令解释，不能重放根 Skill。CLI/直接调用仍默认使用首文本，不引入新的会话 owner 或另一套完成判定。
- 原 Skill descriptor、配置 deny、`user_invocable`、当前工具上限与来源撤销检查继续生效；控制命令仍保持纯文本语义，附图 `/clear` 不清空会话。这里仅开放只读包内 Skill，不执行 shell/fork/hooks。

验证：`cargo test -p nomi-agent --lib --test bootstrap_test --no-default-features -- --test-threads=2` 在 `.tmp-open-skill-input-nomi-final.out` 中为 unit 633 / Bootstrap 26 项通过。新增 3 项测试覆盖图片/附加文本与精确参数、5 类拒绝场景、原文与伪造检索命令区分、续跑不重放和附图控制命令兼容；原完成证据、运行授权、取消和历史恢复回归也在同次 unit 检查中通过。首轮新测试漏算了既有 `Selected Skill ...` 正文来源标签，已修正精确预期，未删掉来源标签或放宽权限。

后端与产品最终回归：

| 命令 / 日志 | 结果与范围 |
|---|---|
| `cargo test -p nomifun-ai-agent --lib --test plugin_tool_consumer --no-default-features -- --test-threads=2`，`.tmp-open-skill-input-ai-final.out` | unit 515 / consumer 28 项通过；生产 manager 新参数已编译，原 turn 生命周期、Registry 冷/热查询、精确 Skill 与 Context/依赖消费回归通过 |
| `cargo test -p nomifun-app --test official_preset_catalog_integrity --no-default-features -- --test-threads=2`，`.tmp-open-skill-input-product-final.out` | 最终代码上 2 项通过，重新编译实际产品组合并复跑冷启动和官方目录测试；不是沿用引擎输入改动前的二进制 |

这是原引擎真实 Bootstrap/模型请求捕获、生产 manager 编译/单元回归及产品冷启动接缝的证据，不等于真实 WebView 上传、远程模型响应、所有文件附件适配器或所有命令来源已验收。剩余完整权限提示、其他来源和受管执行模式仍分别跟踪。

### 2.18 实施切片：Agent Context 发起同图受管依赖调用

本轮先重读 backend-crates 架构、正式 05 §5.5/§5.10、Kernel Context/Tool 分发、Host pending/callback 与产品 runtime lease，再扩展原机制；没有新增 Rust 插件后端、Registry、JS 计划或 Context supervisor。

- `DependencyAncestor` 区分 Tool invocation 与 Context access。Context 不伪造 action/idempotency envelope；父链逐项复查原权限、精确来源与 Role Provider，子调用仍检查所选实现的直接依赖和冻结边，经原 `invoke_scoped` 执行。
- direct/Role 的 Agent Context 请求携带 caller，JS `contributeContext` 提供原 `dependencies.invoke`；非 Agent Role Context 为 `None`，没有伪造 Session 或自动授权。子目标仍是 action，不代表 Context/Resource 方法作为依赖目标均已开放。
- 原 Host pending request 持有 Context callback，复用 deadline、取消、服务任务与 generation 清理；SDK 闭包及绕过 SDK 的 wire 均校验父请求/精确 Mount。产品 wrapper 复用父 runtime lease，避免子调用在已排队写锁后再次等待读租约。
- Context 子效果使用独立身份域；宿主 operation ID 为这一次实际求值命名。Nomi 初始与动态 Context 使用唯一编号，替换重建后会归零的局部计数或可重复启动编号，避免不同求值的依赖效果身份冲突。明确重试同一次求值才复用身份；没有新增持久效果账本、自动重试或回滚保证。
- 作者 scaffold 增加 `PluginContextContribution` 类型。没有新增 wire method/字段；正式 05 同步受管父类型、身份与生命周期语义。普通 Node 的 OS 权限仍不等于强沙箱。

本切片定向证据（2026-09-14 文档核查同步留存结果，非重新运行；不自动覆盖之后改动）：

| 检查 | 结果与范围 |
|---|---|
| Kernel `dependency_call_tests`，`.tmp-open-context-dependencies-kernel.out` | 19 项通过，其中 3 项新增 Context 测试验证真实 factory 无 Tool action、子资源 policy/owner/active/action 拒绝、祖先撤下和父 drop 释放后代；既有 Tool 身份/回调边界继续通过 |
| JS adapter `dependencies`，`.tmp-open-context-dependencies-adapter.out` | 4 项通过；新增真实 Node 的 direct/Role × 同包/跨包 Context→Tool→Tool，未声明目标、重入、错误 action 和来源漂移拒绝；不是只有 Mock |
| JS Host `dependencies`，`.tmp-open-context-dependencies-host.out` | 9 项通过；新增 Context 成功/失败/丢弃、未 await 子任务、无 caller、超时，以及绕过 SDK 的错误父身份/Mount/generation 验证 |
| 产品 `router::plugin_runtime_host`，`.tmp-open-context-dependencies-app.out` | 6 项通过；真实 Node Context 子调用在 runtime 切换写锁已排队时仍完成，取消释放租约；没有第二个 Host 或 Session |
| `cargo test -p nomifun-ai-agent --lib --test plugin_tool_consumer --no-default-features -- --test-threads=2`，`.tmp-open-context-dependencies-consumer.out` | unit 515 / consumer 29 项通过；新增用例验证初始/动态 Context 消费私有依赖、同 Session 重建后效果身份不冲突和不暴露内部 Tool；Nomi 此用例为进程内 handler，真实 Node 路径由上列 adapter/Host 单独验证 |

验证尚未收口的范围：本切片新增作者 SDK 类型后的 authoring foundation 测试，以及最新 Context 改动后的 Kernel/adapter/Host 更广回归，尚无此次通过记录；先前全套通过不能自动覆盖新增改动。上述定向结果不等于产品安装/UI/远程模型的完整端到端验证，也未消除 Host 默认高并发启动超时风险。

这修复的是 Context **发起**受管依赖 action 的缺口。非 Agent operation、资源工厂/后台任务发起调用、任意资源 kind/multi-binding、完整 Prompt/middleware、领域替换、UI/Runtime 与部署服务仍待后续交付。06/02/11 和完整需求 1/3/5 不因该切片关闭。

### 2.19 在途切片：Plugin UI → 既有 AgentSession 命令

2026-09-14 五问复核补记工作区既有实现及验证状态。本次只修改评估/台账并执行定向诊断，不修改运行时代码，不把桥接接通等同于系统页面替换。

当前实现范围：

- 现有 Surface open 可由可信宿主显式授予一个已存在 AgentSession 的访问，绑定精确 UI release；普通打开/重开不继承会话授权。Bridge 不接受 iframe 自报 owner 或会话，身份从授权记录取得。
- `PluginAgentSessionPort` 是应用适配接口，不是第二个 Session manager。产品 adapter 经 `nomifun-ai-agent` 既有接缝，共用会话身份校验、查询和幂等发送；`observe/turn/cancel` 分别处理持久化历史、开始 turn 和取消。错误保持原 Session 状态码/业务码。
- 发送以插件 ID 和意图键分域，视图关闭不撤销已接纳 turn，也不自动重发。异步结果交付前再次校验 Surface，撤销后成功结果及私有错误均不返回旧视图。
- DB 在原 Surface 行保存可空会话引用，检查归属，普通 reopen 清空授权，会话删除撤销 Surface；正式 05 已记录 fresh baseline 选择。本次未操作用户数据集或启动产品迁移。
- 合同将 `cancel` 定义为空对象变体，拒绝多余身份字段；SDK 校验输入及分页，超时报结果未知，结构化克隆失败清理 pending。SDK 随新 release 构建，旧不可变 release 不会被隐式修改。

证据按执行时点分开记录。以下留存结果已读取核对，但不是本次重新执行，也不是整个工作区认证：

| 检查 / 留存记录 | 结果和范围 |
|---|---|
| `cargo test -p nomifun-agent-contracts -p nomifun-api-types --lib plugin_ --no-default-features`；`.tmp-open-ui-session-contract-final.out` | contracts 41、API 类型 28 项通过，含命令额外字段拒绝与 release 授权输入；不等于生成制品同步 |
| `cargo test -p nomifun-db --test plugin_runtime_repository surface_sessions_are_exact_revocable_and_disable_enable_aba_safe --no-default-features`；`.tmp-open-ui-session-db.out` | 1 项通过，含跨 owner/缺失会话拒绝、重开清空、删除撤销与逻辑关联审计 |
| `cargo test -p nomifun-app --lib router::plugin_ --test plugin_ui_sessions --no-default-features -- --test-threads=2`；`.tmp-open-ui-session-app-final.out` | app unit 32 项通过，含测试 port 暂停查询期间撤销后隐去成功与私有错误；**integration 被同一过滤器排除，运行 0 项**，不算产品命令验收 |
| 早期产品命令测试；`.tmp-open-ui-session-product.out` | 当时 1 项通过，真实安装/发布、HTTP Session、生产 Nomi 与模拟模型上游；覆盖发送、历史、重开不重放等。其后新增普通 Conversation 拒绝和运行中取消，旧结果不覆盖新版本 |
| UI 定向测试、TypeScript 和桌面边界（此前执行） | 解析器/Surface/桥接 3 项、57 断言通过，typecheck 通过，桌面边界检查通过；不是实际 WebView 完整页面验收 |

本次重新执行：

| 命令 | 结果及判断 |
|---|---|
| `node --test crates/backend/nomifun-plugin-platform/tests/product_sdk.test.mjs` | **3 项通过**。实际 SDK 与 Node 原生 MessagePort，覆盖命令、错误码、超时不自动重试及 DataCloneError 清理；不冒充浏览器页面运行 |
| `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1` | **1 项失败**。Wiremock 校验 Mock #0 命中 2 次而期望 1，Mock #1 命中 0 次而期望 1。两者都是同 POST/路径条件，延迟响应未命中；当前测试不能证明真实运行中取消。修复需区分响应匹配并确认取消时仍在运行，不能仅删除请求次数断言使其通过。此失败不直接证明生产取消有 bug |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check` | **失败**。首个漂移为 `contracts/runtime/runtime-release-fixture.json`；待实施时按源合同生成、审查差异并再 check，本次不改写生成制品。此前图合同通过记录不覆盖新增 UI 合同 |

剩余任务归入原 P4：命令测试/合同收口、实时事件与历史对账/恢复、同 Catalog 的 UI contribution 选择、实际 Agent 路由接替及内置视图同 API 消费。`observe` 不是 token 重放；公开 Session events 仍不支持；当前一个插件只有一个活动 Surface，多会话/多窗口不得默认已支持。最后须在真实页面验证发送/流/取消/历史/恢复，不能把 HTTP + 模拟上游测试当作完整页面验收。

本切片不关闭 25 或需求 2/5，也不引入 Rust/native/Wasm 插件后端；Shell、独立区域/结果呈现、深层策略、完整 JS Runtime 与部署服务仍按原 29 行推进。下一步修复测试夹具和生成同步是验证收口，不是重建 SDK、Session owner 或另造 UI 业务后端。

**后续实施修复与验证（同日，在上述诊断之后）：**

- `plugin_ui_sessions.rs` 为两个模型响应分别匹配本轮最后一条用户消息，而不是只匹配 POST/路径或扫描包含首轮输入的全部历史；保留两个响应各一次的次数断言。
- 等第二轮真实到达模型后，额外读取公共桥接观察并断言 `head.status == running`，再发送取消，避免把空闲取消当成运行中取消。
- 随后将模拟响应延迟设为 30 秒，严格长于取消后的 5 秒验收期限；不能靠模型自然结束通过取消断言。此最终版本用同一无过滤命令再次通过，1 项、4.23 秒（本轮终端结果）；下列 4.29 秒日志属于增加该强断言前的版本。
- `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1`：**1 项通过**，4.29 秒，见 `.tmp-open-ui-session-product-fixed.out`；真实产品安装/Session/Nomi 消费，模型上游为受控 SSE 响应，不是外部服务或浏览器页面测试。
- `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write` 后，再执行同一命令的 `check`：**通过**。生成物沿当前源合同重建，包含 AgentSession 命令 schema 和关联摘要；不改动其他用户源代码、不添加第二套协议或数据迁移。

上方失败记录保留为历史，产品测试及 canonical 漂移这两项现已核销；后续 SDK/模型事件与真正页面替换仍需各自验证。用户新增 JS 适配性约束不影响本命令接缝的价值：它复用现有业务 owner，未通过原生临时代理实现功能。下一步事件设计沿既有 owner-scoped WebSocket 总线推进，不能为了尽快有流式效果再造日志或无界消息队列。

### 2.20 Plugin UI 实时事件：复用现有总线，不新增执行后端

本切片按用户的 JS 适配性约束继续 UI 公共接缝；不启动 Rust 插件系统，不用临时原生代理绕开语言限制。实现基于 §2.19 的同一 Surface grant 和 Session owner：

- 后端只读观察既有用户事件总线，将 `message.stream` 投影到同一 `/ws` 的 `plugin.agent-session.stream`；批量上限 32 条，不另建事件存储或订阅授权表。独立观察任务使异步授权查询不阻塞原普通会话转发。
- 只投影同 owner/会话、enabled 且 release ID/digest/epoch 匹配的 Surface；经 canonical Session admission 后重查 ID/generation，关闭后重新打开的视图不继承旧批次。已经交付的网络数据不可撤回，不承诺原子撤销在途帧。
- 父页面按 plugin/Surface ID/generation 精确路由，去掉路由信封再送 MessagePort。关闭/替换视图时清理监听。SDK 支持异步 `agentSession.subscribe`，订阅代数防止旧帧混入新监听；重复注册同一函数可独立退订。
- 最多 8 条未确认交付，每条事件至多 64 KiB；回调串行完成后 ACK，错误回调不阻塞后续处理。慢消费者不无限堆积碎片，消费恢复后明确通知重新同步。首次订阅/重连/总线丢失也触发重新同步。
- 恢复调用 `observe` 读取持久化消息，不重发 turn、不伪造 token 游标。不保证恢复运行中尚未持久化的半条回答；页面需要按消息身份对账，并展示恢复中/等待最终消息。实际恢复 UI、Catalog 页面绑定和 Shell 尚未完成。

定向验证（本切片）：

| 检查 | 结果与范围 |
|---|---|
| `node --test crates/backend/nomifun-plugin-platform/tests/product_sdk.test.mjs` | 6 项通过；原生 MessageChannel 验证命令/错误、异步历史恢复后 ACK、串行回调与异常、独立退订及新旧订阅隔离 |
| `bun test --cwd ui src/common/utils/pluginAgentSessionStream.test.ts` | 4 项、18 断言通过；精确路由、1000 次事件突发仍受 8 条额度限制、无效 ACK、超大事件与重连、投递失败关闭 |
| Surface DOM interaction test | 2 项、10 断言通过；受控握手及关闭后卸载三个事件监听，保留失败重试用例。此测试使用空白 iframe 和受控端口，不冒充真实浏览器 SDK 产品验收 |
| `cargo test -p nomifun-app --lib router::plugin_product::tests::agent_ui --no-default-features -- --test-threads=1` | 3 项通过，见 `.tmp-open-ui-stream-app.out`；真实 SQLite/产品授权，暂停 Session port 制造关闭/重开竞态。包含原命令撤销用例，不是实际模型测试 |
| `cargo test -p nomifun-app --lib router::plugin_ --no-default-features -- --test-threads=2` | 34 项通过，63.53 秒，见 `.tmp-open-ui-stream-regression.out`。包含上述 3 项，不累计为 37 个不同测试；首次前台调用达到工具时限，随后后台完整运行通过 |
| `bun run --cwd ui typecheck` | 通过；包括新增 relay、事件类型、Surface 接线和测试 |
| UI 四文件联合回归、`bun run check:desktop-ui-boundary` | 8 项、81 断言通过；包含 relay/Surface/命令解析/不可变 release 握手合同，桌面边界检查通过（最低 880×600） |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`、`git diff --check` | 均通过；本切片没有新增 canonical 执行后端或修改数据 schema |
| `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1` | 1 项、5.24 秒通过，见 `.tmp-open-ui-stream-product.out`；真实安装/HTTP Session/生产 Nomi/现有 WS manager 出站队列，模型上游受控。新增第三轮验证 scoped stream，首个模型响应对应两轮、取消响应对应一轮；仍保留原第一轮立即关视图及第二轮运行中取消断言。不是网络 WS 握手或浏览器端到端测试 |

当前尚未完成整个 13/25 或需求 2/5。公共 Session events 的持久化重放仍不支持；本增量是 live WS 投影，不是该端点补齐。无全平台压力、真实浏览器断网恢复或完整页面替换验收，不宣称已达到对应发布条件。

### 2.21 Agent 页面贡献发布与显式选择

按 §27.13 的 JS 适配原则完成自然 UI 接缝的收口，不推进 native 密集实现或另一个执行后端：

- 作者通过源 manifest 的 `agent_view: { name, description }` 显式发布 UI-only capability，可与 actions 共存，不要求另建 Node service。普通 HTML 不自动变成 Agent 页面。canonical `ui_slot = agent_session` 标记真实消费槽；不把 UI 贡献暴露成 Agent Tool。
- UI 候选来自既有 Catalog 的只读投影，不新建 registry。选择精确绑定 plugin/capability ID/version/release digest；Surface open 重新校验同发布记录及现有 owner/Session/活动指针，不仅相信前端列表。
- `/agent-sessions/:agentSessionId` 真实路由加入可信选择/恢复区域；选用插件后卸载内置内容、挂载原隔离 Surface。原内置页面移至 `BuiltinAgentSessionPage.tsx`，保持其公共 AgentSession API 消费，不重写查询/发送/取消业务。
- 仅改变下拉候选不授权；用户点击使用才打开视图。候选撤下/升级后关闭旧视图，即使原候选重新出现也不自动重新授权；语言变化不触发 reopen。路由切换后迟到的 open 结果关闭，不挂载到新 Session。
- 返回内置/重载只管理 Surface，不重发或取消 turn。close 失败保留明确警告，不将本地断开宣传为已确认服务端撤权。此选择只作用于页面，没有增加 UI 默认数据库或把它冒充 Preset/Role 绑定。

本轮定向验证：

| 检查 | 结果与范围 |
|---|---|
| `bun test --cwd ui src/renderer/pages/agentSession src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.interaction.test.tsx src/renderer/pages/plugins/runtime/agentSessionBridge.test.ts src/common/utils/pluginAgentSessionStream.test.ts` | 7 文件、23 项、149 断言通过；其中新增页面选择 5 项。包含原页面/模型投影/导航/Surface/命令解析/流 relay 回归，导航断言同时检查新宿主和移出的内置页面。使用独立 SWR 缓存、受控 API 和空白 iframe，不冒充真实页面业务联调 |
| `bun run --cwd ui typecheck` | 通过；修复测试 close 返回值类型后重跑，不引用较早失败输出作为成功证据 |
| `bun run check:desktop-ui-boundary`、`bun run check:i18n` | 通过；最低 880×600；中英文文案与生成 key 一致 |
| `node --test crates/backend/nomifun-plugin-platform/tests/product_sdk.test.mjs` | 6 项通过；沿用原 SDK/MessagePort 回归，无另建命令协议 |
| `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1` | 最终 1 项通过，5.30 秒，见 `.tmp-open-ui-choice-product-final.out`。通过真实 HTTP 编辑源码/构建/发布，验证普通 HTML 无候选、UI 进入同目录但不进 Agent Tool、精确选择成功、错误 ID/version/digest 被拒、禁用撤下候选。保留生产 Session 发送/历史/实时投影/运行中取消与重开不重放验证；模型为模拟上游，不是浏览器端到端 |
| `cargo test -p nomifun-agent-contracts --lib plugin_runtime::tests -- --test-threads=2` | 最终 23 项通过，见 `.tmp-open-ui-choice-contract-verified.out`。新增纯 UI 无 Service 与 UI+Tool 共存正例，以及缺 HTML、错误 kind/consumer、无 slot、未消费依赖拒绝；原发布/权限/制品合同回归保持通过 |
| 来源 manifest `agent_view` 单元测试 | 1 项通过，见 `.tmp-open-ui-choice-contract-final.out` 中 plugin-platform 部分，验证显式声明、精确包身份、UI-only 和 actions 共存。该记录早于最后 release 校验修复；最终产品测试再次实际经过相同 materialize 路径 |
| `cargo test -p nomifun-app --lib router::plugin_ --no-default-features -- --test-threads=2` | 35 项通过，98.99 秒，见 `.tmp-open-ui-choice-regression.out`；包含新增 authoring HTML/Service 准入与原授权/撤销/依赖回归。执行在最终 release 无 Service 准入修复之前，不能冒充最后修复后的全量重跑；最后修复由上述 23 项合同和真实发布链覆盖 |
| canonical `write`，随后从最新源码执行 `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check` | 通过；同步 `ui_slot` schema 和摘要引用。不代表重新执行跨平台制品验证 |
| `git diff --check` | 通过；未运行整个 workspace 测试或真实桌面/WebUI 浏览器联调 |

修复过程如实记录：初次编译暴露新增可选 `ui_slot` 后若干完整 Rust struct literal 漏字段，已在对应构造点补 `None`；产品测试扩展后 `grant` 重用须 clone；UI 测试的同步 asset resolver 曾误 mock 为 Promise，修复后联合回归无网络请求错误。合同负例夹具由 Tool 改 UI 时遗留 action schema，严格 registry 校验正确拒绝，清理无引用 schema 后重跑；没有绕过 schema registry 校验。

真实产品测试另发现旧 release 校验把所有 capability 一律认作需要 Service，导致显式 UI 发布被拒（`.tmp-open-ui-choice-product.out`）。最终仅允许无 Service 产品包含已经支持的 UI contribution，后续 slot/consumer/HTML/依赖校验仍执行；无 Service 的 Tool/MCP/资源/迁移/凭据仍被拒。正例改为实际无 Service 制品并增加 UI+Tool 共存，最终结果见上表。这是生产准入修复，不归咎于 JS 语言，也不通过增加一个空 service 绕过。

仍未完成：完整可用页面的发送/流/取消/分页历史/断网恢复浏览器联合验收、持久 UI 默认与模板绑定、无 Session 入口、多活动 Surface、Shell/区域/结果呈现。当前一个插件一个活动 Surface，重开会替换原授权；普通 Session events 仍没有 token 持久化重放。页面宿主当前也不消费 UI contribution 自身的依赖/资源图，发布端明确拒绝这些混用声明，而不是接受后静默忽略。后续若开放，必须与真实消费者一起设计，不预建空 port。

这批只核销页面声明/选择/宿主切片，不核销整行 25 或五项完整目标。更适合 Rust 的具体实现按报告 §27.13 延期；适合 JS/UI 但尚未验收的工作保持在途，不能也归入 Rust 延期。

### 2.22 Agent 参考页面沿原草稿发布链交付

按用户“JS 不适合的实现等待 Rust，不交付临时版本”的要求，本切片只做自然适合 UI 的真实消费。不增加 Rust/native 后端、原生代理、Node service、第二个 Session owner、消息账本或授权表。

交付路径：Agent 页点击“创建参考视图草稿” → 现有 `/plugins/create/:draftId` 编辑/预览 → 用户显式保存 → 原 source/build/publish 流程 → 回到 Session 刷新候选并显式选择。`POST /api/plugins/drafts/from-template/agent-session-view` 只写普通 owner-scoped ready 草稿，使用 UUIDv7，不调用模型、不发布、不授予会话权限。双击受控；离开页面后的迟到创建结果不导航到旧上下文。创建结果未知时提示先检查已有草稿，不自动重试制造更多草稿。

`plugin_product/templates/agent-session.html` 是随产品交付的可编辑基础源码，而非另一套页面框架：

- 使用生产 `window.nomi.agentSession` SDK；预览明确没有 Session 权限，正常保存也不要求配置 authoring 模型。普通插件库独立打开时仍没有会话授权，提示从实际 Session 显式选择。
- 当前页最多 50 条持久化记录，前后翻页只保存游标位置。历史记录可能原位更新，刷新重读当前页并整体替换，不仅从最后 seq 追加；页面数不冒充会话全局游标。满页后可能出现一次空的末页，这是当前无 `has_more` 合同的可见语义，不杜撰总页数。
- 实时订阅只合并请求刷新，不展示 token 缓存。`content/replace/output_discarded/resync_required` 不会与持久化历史拼接；隐藏事件不呈现。定时刷新覆盖遗漏的完成/取消通知，只允许一个查询在途；已卸载页面忽略迟到结果、退订并清理定时器。历史不可用时清空展示并暂停自动查询，明确让用户恢复。
- 消息内容按纯文本展示，不用 `innerHTML` 渲染用户/模型输入，不输出整份内部对象或技术 ID。无法解释的结构化记录提示使用内置页面，并未宣称完整工具、附件或富文本展示已支持。
- 文本发送只消费原 Session API。结果未知时保留页面内不可变输入与幂等键，不自动重试；用户主动“重试同一请求”才重用该键。放弃旧重试身份须在页面内确认已核对历史，不依赖 iframe sandbox 禁止的 modal。取消失败明确标记未确认，查询不自动重试取消。
- 未发送文本和请求键**不跨页面重载保存**，界面提醒复制文本并核对历史。关闭页面不取消/重启 turn；参考源码不是跨重载无损恢复的完成证据。英文参考页可自行修改，并不代表完整本地化产品体验验收。

定向验证（以下最终结果覆盖本切片最后的页面守卫修改）：

| 检查 | 结果与边界 |
|---|---|
| `node --test ui/src/renderer/pages/agentSession/AgentSessionTemplate.node.ts` | 10 项通过。实际模板脚本、DOM 与生产 SDK/Node MessagePort；验证预览、分页/原位更新、HTML 注入防护、实时/历史隔离、超时/显式同键重试、读取失败与恢复、取消失败、游标异常及卸载迟到结果。宿主响应和时间受控，不是真浏览器/后端端到端 |
| `bun run test:agent-view-template` | 上述 10 项 + 原生产 SDK 6 项；Node 24+ 运行 TypeScript 测试。单独脚本避免 Bun DOM 预加载替换原生 MessageChannel；不新加依赖或生产运行时 |
| UI 7 文件联合回归（命令同 §2.21） | 25 项、158 断言通过；页面选择新增草稿双击/错误/迟到导航测试，保留真实宿主组件与既有 Surface/relay/解析/内置导航回归 |
| `bun run --cwd ui typecheck` | 通过；此前一次工具调用超时未得到结果，后续前台重跑成功，不将超时当作通过 |
| `bun run check:desktop-ui-boundary`、`bun run check:i18n` | 通过；未扩展低于 880×600 的布局，中英文入口文案与生成 key 一致 |
| `cargo test -p nomifun-app --lib router::plugin_product::templates::tests --no-default-features -- --test-threads=1` | 最终 1 项、2.04 秒通过；真实 DB/普通草稿持久化、owner 隔离、UUIDv7、重复创建独立身份、无隐式发布，显式保存构建发布并保留相同 HTML |
| `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1` | 最终 1 项、4.83 秒通过；保留生产 Session 发送/取消/撤权/流投影回归，新增真实 HTTP 模板创建 → 保存 → Catalog → 精确 Surface 授权 → 历史查询；断言创建/打开不增加模型请求。上游为 wiremock，流验证仍是现有 WS manager 队列，不冒充浏览器联网 |
| `git diff --check` | 通过；未改 canonical 协议或数据 schema，本切片未重跑 workspace 全量测试 |

过程记录：首次把原生端口测试放在 Bun 环境，DOM 预加载导致 `port.on` 不存在，7 项测试在初始化失败；转为 Node 内建测试运行器后使用原生端口，未将 SDK 改成测试专用路径，也未增加端口模拟器掩盖问题。曾运行带模板过滤器的 `--lib --test plugin_ui_sessions` 命令，集成目标匹配 0 项；该输出不计产品验收，最终单独无过滤器执行的 1 项结果如上。

当前不关闭整行 25 或需求 2/5。P4 剩余：完整消息呈现、真实浏览器桌面/WebUI 发送/取消/断网/视图卸载联合验收、跨重载输入与不确定请求恢复、持久默认/绑定、无 Session 入口。源码草稿模板不是 Preset/UI 绑定模板；多 Surface 和 Shell 仍未交付。上述天然 UI 问题属于适合 JS 的未完成项，不归咎于缺少 Rust。原生密集实现继续按报告 §27.13 延期，延期不计为已解决。

### 2.23 参考页面恢复收尾与实施优先级纠正

本节更新 §2.22 的“输入/请求键仅页面内保存”限制：现已通过原插件私有 KV 的版本读取与 CAS，按已授权 Session UUID 保存草稿及待确认发送意图。SDK 新增 `storage.read/compareAndSwap`，复用现有事务、Surface 校验与删除墓碑版本，不新增数据库协议、消息账本或发送执行器。Session 分键只是模板约定，不是插件私有存储内的新安全隔离边界。

发送前必须确认意图保存成功；重开仅恢复，用户显式重试才复用原输入/幂等键。并发冲突、读取/保存超时、损坏记录和发送成功后清理未确认分别处理，不自动重发；关闭期间迟到保存不会触发发送。只承诺恢复已确认保存的数据，即时关闭/离线未确认编辑仍可能丢失；插件草稿也不随 Session 删除自动清除。

已有定向验证：模板与生产 SDK 的 Node 测试 26 项通过；UI 联合回归 28 项/175 断言及 typecheck、桌面边界检查通过。真实 HTTP/Session 集成 1 项通过（`.tmp-open-ui-recovery-product.out`，6.36 秒），覆盖保存后发送、重开读取、同键重试只执行一次、旧 Surface 写入拒绝和 CAS 冲突/墓碑。前端测试使用受控宿主响应，集成使用模拟模型上游；没有真实浏览器联网故障联合验收，不将局部恢复视为完整 UI 验收。上述是该切片已有结果，本次文档同步未重复跑整套测试。

用户指出近期存在过度打磨，执行优先级据此纠正：停止参考页面扩展，保留完整呈现、默认/绑定、多 Surface、Shell 和浏览器验收等未完成项；下一批回到真实部件替换，先核对 ToolSearch 的候选发现/排序消费链。仅提供 trait、接口或样例不算完成；需同一 Catalog/冻结计划选择、真实调用及必要故障验证。适合 JS 的未完成项不冒充 Rust 延期，确需 native 的实现仍按报告 §27.13 延期。此收尾不关闭行 25 或整个目标。

### 2.24 ToolSearch 回到部件开放主线：内置排序与激活职责拆分

重读实际链路后确认：`nomi-agent/bootstrap.rs` 注册唯一内置 ToolSearch，共享 Registry 的 deferred 目录；插件 actions 经 `NomiPluginToolSession::register_into` 进入同一目录。`tool_execution.rs` 保留当前模型轮次的可见性边界，搜索后完整 schema 只在后续请求出现。Role/Provider 已有精确选择、冻结和 Kernel 分发，但尚无发现策略合同及对应 Nomi 消费者。发现/排序是粗粒度异步决策，当前没有证据需要因 Rust 插件延期；不能用 Context 贡献或覆盖 ToolSearch 名称伪装策略替换。

本次仅改两个宿主文件：`nomi-tools/src/registry.rs` 与内部 `registry/deferred_search.rs`。内置排序只读取名称/描述/别名，返回有序名称；激活提交按捕获目录及当前条目身份校验整个结果后，再写入原 Session 激活集合。目录条目用共享引用避免为捕获复制大块 schema；名称相同的替换也不能复用旧条目身份。越界、重复、别名充当路由、条目移除/重注册、跨 Session 结果均不产生部分激活。内置路径仍在单次锁内完成，保留原搜索顺序、五项上限及输出格式；没有新增第二目录、权限源、公开 SPI、JS 消息协议或 Rust 插件后端。

定向验证：`cargo test -p nomi-tools --lib registry:: -- --test-threads=2` 49 项通过（含新增 5 项）；`cargo test -p nomi-tools --lib tool_search:: -- --test-threads=2` 16 项通过；`cargo test -p nomi-agent --test deferred_activation_test -- --test-threads=2` 5 项通过，覆盖下一轮完整 schema、保存/恢复激活及动态注册。未运行整个 workspace，不将受控 Provider 测试视为真实 JS 插件端到端。

**行 05 仍未完成。** 此次是被内置实现实际消费的内部职责拆分，不是用户可选择的 JS 策略交付。下一步须以同一 Catalog/Role 合同发布默认与用户策略，冻结选择后沿原 Kernel/JS adapter 调用；同时验证限时/取消、候选负载和错误退出，再由宿主提交选择。不得新增私有策略注册表或用普通可见 Tool 旁路；不得把未接线状态当作 Rust 延期。

### 2.25 ToolSearch 隐藏策略与既有 Role/JS 消费接线

在 §2.24 的内部拆分上接通实际消费者：内置 `agent.tool-discovery` / `system.tool-discovery` 进入现有 Nomi Catalog；用户发布独立 ID 的隐藏 `tool.discovery.rank` action，可直接选一个策略，也可用原 Role Provider 映射替换内置实现。安装/选择/冻结继续复用原链路；策略不会暴露为另一个模型 ToolSearch。没有新增 Rust 插件后端、私有策略注册表或第二份 Session 状态。

Nomi Session 从精确验证的包 manifest 识别合同，沿原 `KernelRegistry::invoke_shared`、Role 分发及 JS adapter 调用。安装型 `ResolvedCapability` 锁 manifest 摘要而不重复 actions，不能误读其空 actions 为没有隐藏策略；发布型 Product release 的 actions 与执行 adapter 不同，本切片未接通，明确不计作支持。隐藏策略只输出 names，宿主按当前候选身份整体校验后激活；超时、取消、输出非法、包撤下均不静默回退。

合同与使用方法见正式设计“ToolSearch 发现策略”：256 KiB 元数据、4 KiB query、5 秒调用期限、最多 5 项；现有通用 Role 选择 UI 无需新增控件。未选择策略的 Session 保留原内置行为。此合同为纯排序，不把远程模型调用或原生索引引擎纳入已交付范围。

最终验证记录如下。本次收尾核对既有结果与测试代码，不重复运行未改动的整套测试：

| 检查 | 结果与证据边界 |
|---|---|
| `cargo test -p nomifun-app --lib discovery --no-default-features -- --test-threads=1` | 3 项通过，见 `.tmp-open-discovery-app-final.out`。其中 2 项验证生产内置隐藏 Role 和真实包导入/安装、同目录选择、保存两个版本并重开；旧版本仍使用内置，新版本调用 JS。另 1 项为同名过滤匹配的既有测试，不计作新增策略覆盖 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=2` | 最终 30 项通过。发现策略用例实际经过 Nomi ToolSearch → Kernel → Node，覆盖内置/用户 Role、直接能力选择、短查询自定义、查询预算、非法/重复结果不激活、内部错误不向模型泄漏、取消到达 JS、真实 5 秒超时及撤下无 fallback |
| `cargo test -p nomi-tools --lib registry:: -- --test-threads=2` | 49 项通过，含候选快照、整体校验、移除/同名重注册和 Session 隔离 |
| `cargo test -p nomi-tools --lib tool_search:: -- --test-threads=2` | 16 项通过，保留内置查询及结果投影行为 |
| `cargo test -p nomi-agent --test deferred_activation_test -- --test-threads=2` | 5 项通过，覆盖后续模型请求才暴露 schema、保存/恢复与动态激活 |

产品测试的调用终点是原 Kernel/真实 Node；Nomi ToolSearch 的真实调用由消费者测试覆盖，两者不能合称完整浏览器/模型端到端。取消后 Host 异步清理尚未结束时，测试曾过早停止 Host；最终测试等待既有 quiescent 屏障，未修改生产清理逻辑或强制杀进程。未运行整个 workspace、真实浏览器或大目录性能测试。

行 05 仍保留未完成项：发布型 Product 策略执行、保存前多策略冲突诊断、schema 暴露策略/预算和大目录性能验收。此处推进的是已安装 JS 包的发现/排序真实替换，不核销整个平台或五问。

### 2.26 产品发现策略接入前：修复原 Service 调用的放弃等待

核对产品执行链发现：`invoke_agent_capability_inner` 复用原 Service Host，但调用 future 被丢弃后，Host 的强引用在途登记不会自行退出；Node 取消又依赖已被丢弃 future 内的轮询。这会破坏发现策略的超时/清理语义，因此先修复原链路，没有提前开放 Product 策略。

Host 的在途项改为由调用 future 持有的弱引用登记，放弃时触发原取消标记，后续调用或既有空闲维护清理失效登记；正常完成不取消。结果返回只移除相同登记，旧调用不能移除后来同 ID 的新调用。Node Actor 在原 watchdog 中检测取消及已关闭的响应接收端，取消原请求；移除每调用轮询和 `CancelCall` 队列路径。取消 ACK 不视为调用完成，不延长原请求期限，忽略取消仍由原 watchdog 回收。未新增后台清理任务、原生代理、Session owner 或协议。

验证：`cargo test -p nomifun-plugin-platform --test service_process --no-default-features -- --test-threads=2` 共 7 项通过（`.tmp-open-product-discovery-cancel.out`，22.69 秒，本机 Node 24.18.0），其中新增 3 项覆盖真实 Node 放弃调用、保留另一调用、Host ID 复用/无后续请求时空闲回收及忽略取消的 watchdog 退出。`cargo test -p nomifun-plugin-platform --test service_application --no-default-features -- --test-threads=2` 2 项通过，覆盖原构建/发布/启停和 Agent 调用；后者使用测试 Runtime，不冒充真实 Node 产品纵向验收。

本节仅关闭这项真实消费者的生命周期前置缺陷。产品隐藏策略声明/装配、保存期冲突诊断和发布到 Nomi ToolSearch 的完整验收仍待完成；不核销行 05、14 或整个本批，也不将它归入 Rust 延期。未运行 workspace 全量、UI 或浏览器检查。

### 2.27 发布型发现策略沿原执行链接入与保存冲突诊断

Product Active Release 的精确隐藏 `tool.discovery.rank` 现已沿原 `NomiPluginProductToolInvoker` 接到同一 Nomi ToolSearch。装配使用冻结的 release/epoch/catalog 身份并核对输入、输出 schema 摘要；策略不进入模型可见 actions。选择了 Product 却未绑定对应适配器时拒绝注册，不回退内置实现。没有新 Registry、执行器、JS 协议或 Rust 插件后端。

同一选择校验复用于 Session 装配和 Nomi 组合根配置的编译检查；内置/安装包/Product 多策略冲突在保存前拒绝。通用 compiler 只接收宿主的只读校验函数，不依赖 Nomi，也不二次解析或重写计划；无改动保存复用旧快照时仍执行校验。**当前没有独立 preview API/UI**，不把这项编译校验记成预览功能交付。

| 定向检查 | 最终结果与证据边界 |
|---|---|
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=2` | 31 项通过，`.tmp-open-product-discovery-consumer.out`。新增 Product 消费者用受控 invoker 验证冻结身份、反向排序、隐藏不暴露、缺适配器、输出摘要错误、非法/重复/多余字段/执行错误不激活；不是实际发布链 |
| `cargo test -p nomifun-agent-control-plane --lib compiler:: --no-default-features -- --test-threads=2` | 17 项通过，`.tmp-open-product-discovery-compiler.out`。覆盖新计划与无改动复用都执行校验，拒绝不改写旧 Revision，成功复用原快照 |
| App `--lib discovery --no-default-features`（与首次 Product 测试一起执行） | 3 项通过，`.tmp-open-product-discovery-app.out`。安装型测试增加真实保存冲突及无改动复用检查；包含 1 项无关同名匹配，不把它算新增覆盖 |
| `cargo test -p nomifun-app --test plugin_product_discovery --no-default-features -- --test-threads=1` | 最终 1 项通过、10.96 秒，`.tmp-open-product-discovery-product.out`。真实源码创建/编辑/构建/发布/启用 → Catalog → 保存/冲突拒绝/复用 → Nomi 模型工具循环 → Node Service；包含非法结果、真实 5 秒超时、停用后已有 Session 明确失败且不新增 schema。模型上游为 wiremock，无浏览器联调 |

真实产品用例用一字符查询证明所选 JS 策略被消费，而非内置低信息查询拒绝；候选允许为空，不把它声称为大目录排序/性能验收。非空候选顺序及原子激活由消费者/Registry 测试覆盖。Service 的取消到达 Node、放弃调用回收与忽略取消 watchdog 证据沿用 §2.26，本次未重复跑未改动的进程测试；不声称已完成真实 UI 取消或 release 升级重开联合验收。首次 Product 负例漏选内置 Role Provider，被原有未绑定检查拒绝；修正测试显式选择后再验证多策略诊断，没有放宽生产约束。

Product 声明使用现有 `nomifun.plugin.json` 的 `contributions.capabilities` 和 `schemas`，能力为 Agent + PluginService 的 Tool、单一精确隐藏 action、无资源需求。普通 `actions` 简写仍生成可见工具，不适用此合同。Product 当前只能直接选择独立 capability；release 仍禁止 Role 声明，不宣称 Product Role Provider 已支持。保存冲突检查不是所有运行失败的静态证明，发布身份、消费者及可用性仍由原调用链校验。

`git diff --check` 通过；没有运行 workspace 全量测试、UI/typecheck 或浏览器联调。本批到此停止，不继续 UI 打磨、schema 预算、算法或 Runtime 改造。行 05 的 schema 暴露策略/预算、大目录验收，以及跨领域升级/资源/生命周期等仍未完成；29 行及五项总体目标保持进行中。

### 2.28 在途：before_model 请求消费者与 Product 来源

Nomi 主模型/工具循环新增真实请求中间件，不将它塞入 additive Context 或注册成模型 Tool。消费者输入仅为阶段、当前用户输入元数据、可变提示词视图与工具名称/描述；输出为可选提示词替换和工具子集/顺序。无历史账本、图片字节、schema、原生对象或可变 Engine 引用。没有新增 JS/Rust 后端或第二个执行 owner。

当前请求的工具权限快照与“本次暴露了哪些工具”的统计统一移到变换之后。链式结果不得增加原候选之外的工具、重复工具或恢复前序已移除工具；链失败不调用 provider，也不提交部分提示词变更。修改只作用于请求视图，不回写基础提示词或持久化历史。开启中间件时，计划模式指令和既有路由/资源/回合事实位于可修改视图之外；未选中时保留原提示词排序。压缩等独立模型调用尚未接此阶段。

Product 用 `TurnMiddleware`、Agent + PluginService consumers、单一精确隐藏 `agent.before_model` action 表达贡献；schema 与发布形态检查放在 `nomifun-agent-contracts::model_middleware`，同名 action 不得伪装成 Tool 或混入其他贡献/资源。复用原源码发布、Service invoker 与精确 release/epoch/schema 身份校验；缺适配器明确失败。`actions` 简写不适用；N1 kind/handler 尚未扩展。普通 Node 的 OS 权限边界没有因此变成强沙箱，`Pure` 是受管操作合同而非对任意 JS 副作用的物理证明。

本次主循环共享 5 秒中间件期限，每次输入最多 256 KiB、patch 最多 64 KiB；不截断后静默执行。超时/放弃请求丢弃当前 future，Product 沿 §2.26 原 Service Host/Actor 机制处理取消，不新增轮询器。

| 定向检查 | 当前结果 / 证据范围 |
|---|---|
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=2` | 最终 36 项通过，`.tmp-open-before-model-consumer-complete.out`；包括非空工具候选、真实 Engine 连续模型请求、提示词不持久化、恶意模型调用被拒绝、链式过滤、非法/超大结果、5 秒期限与外层放弃调用回收。invoker 为测试实现，不冒充 Node |
| `cargo test -p nomi-agent -p nomifun-agent-contracts --lib before_model --no-default-features -- --test-threads=2` | 2 项通过，`.tmp-open-before-model-core.out`；实际 Engine 保留计划/资源指令及无中间件路径，发布验证拒绝错误 kind、action/schema、资源和 consumer |
| `cargo test -p nomifun-app --test plugin_product_discovery --no-default-features -- --test-threads=1` | 最终 2 项通过、16.52 秒，`.tmp-open-before-model-product-final.out`；新增 middleware 的真实源码发布→选择/保存→Nomi→Node 调用，验证非空工具过滤、两次 turn 不累积 patch、非法结果/真实超时/停用后不调用模型；另一项为原发现策略回归。仅模型上游为 wiremock，无浏览器验收 |
| `cargo test -p nomi-agent --lib engine::plan_mode_tests --no-default-features -- --test-threads=2` | 4 项通过，本轮终端输出；保留原计划模式状态转换回归，不扩大为全部 Engine 已验收 |

首次消费者测试把模型调用已隐藏工具误当作可恢复工具错误，实际引擎会立即拒绝该 provider 请求；已改为明确断言拒绝与零工具执行，没有放宽生产校验。首次产品夹具未启用 ToolSearch 却要求存在该候选；修正为显式选择内置发现 Role 后通过，没有删除非空候选断言。外层放弃调用的直接证据来自消费者测试，Node 清理沿用 §2.26；不声称真实 UI 取消/升级重开联合验收已完成。`git diff --check` 通过；未运行 workspace 全量、UI/typecheck 或浏览器检查。

**本节记录时尚未收口：** Product 多贡献当时只提供 capability ID 默认顺序；用户指定顺序及冻结/保存复用的后续交付见 §2.29。来源/请求消费者不重复建设，不启动 B 的 UI 或其他 middleware 阶段。整行 12、需求 3/5 均未完成。

### 2.29 Product before_model 用户排序与冻结收口

原 Revision、Snapshot、API 文档和编译配置摘要增加可选 `middleware_order`，空值省略以保留旧序列化摘要。显式项先执行，其他已选贡献按 ID 接续；重复、未选中、仅依赖用途或非请求中间件的排序项拒绝。顺序不改变选择、授权和 Context 顺序。原编译器复用判断包含该字段，原 Session 只消费冻结计划；不新增目录、解析器、执行 owner 或 Rust 插件后端。

原 Context 排序控件抽为共用贡献排序控件，Agent 编辑器增加请求中间件上下移动/恢复默认，保留不可用显式项，显式移除 capability 时同步移除排序项。没有扩展参考 UI 或全阶段总线。

| 定向检查 | 本次结果与证据范围 |
|---|---|
| `cargo test -p nomifun-agent-contracts -p nomifun-agent-control-plane --lib order --no-default-features -- --test-threads=2` | 5 项通过（其中 4 项直接涉及 Context/middleware 顺序，另 1 项为同名匹配）；`.tmp-open-middleware-order-core.out`。证明缺省摘要、唯一/选中/贡献用途校验、运行配置摘要变化、保存复用、DTO 回读及拒绝非中间件；通用编译夹具不是额外运行时来源交付 |
| `cargo test -p nomifun-app --test plugin_product_discovery --no-default-features -- --test-threads=1` | 3 项通过、46.45 秒，`.tmp-open-middleware-order-product-complete.out`。两个真实源码发布的 Node 中间件以非交换嵌套变换证明默认/部分/完整显式顺序；无改动复用 Revision/快照，修改顺序新建版本/快照，旧 Session 保留旧顺序、新 Session 使用新顺序；非法排序拒绝。含原请求变换/失败路径与发现策略回归；上游模型仍为 wiremock，不是浏览器或真实模型验收 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=2` | 36 项通过，`.tmp-open-middleware-order-consumer.out`；保留真实 Engine 工具权限、连续请求、错误/期限/取消以及其他已有消费者回归 |
| `bun test --cwd ui src/renderer/pages/agentSettings/AgentContextOrder.interaction.test.tsx` | 5 项通过，`.tmp-open-middleware-order-ui.out`。原 Editor 保存动作同时提交 Context 和 middleware 顺序；筛选、缺失项、禁用态、恢复默认、移除与选择不变。不代替浏览器验收；仍有 React/Arco 的 ref 与测试布局 NaN 警告 |
| `bun run typecheck` / `bun run check:desktop-ui-boundary` / `bun run check:i18n` | 均通过；类型检查记录于 `.tmp-open-middleware-order-types.out`，桌面最小 880×600，翻译类型由原生成器同步 |
| `cargo test -p nomifun-agent-contracts --lib preset:: --no-default-features -- --test-threads=2` | 13 项通过（终端输出），包括 Revision/Snapshot 缺省序列化和摘要、依赖用途以及现有合同回归 |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write` / `-- check`；`git diff --check` | 原合同生成器同步并检查通过，日志 `.tmp-open-middleware-order-schema*.err`；diff 空白检查通过。首次前台检查超过工具等待窗口，改为有日志的后台检查后确认结束，无运行中编译任务 |

首轮新产品测试出现 ID 字符串转换编译错误，之后的断言又将 API 省略空 allowlist 与显式空数组误判为选择变化；修正测试为 canonical 类型比较。一次补丁未匹配格式化后的代码导致重复失败，确认落盘后才取得上述最终证据，没有放宽生产验证。没有运行 workspace 全量、浏览器或跨 release 升级/恢复联合验收。

**A 卡在 Product + 主模型/工具循环 + 用户冻结排序的限定范围到此收口。** N1 middleware 来源、其他阶段、压缩等独立模型调用仍在行 12；完整 29 行和五问保持进行中。本批不启动 B、其他阶段或新后端，后续按报告 §27.14 独立拆卡。

### 2.30 预设级默认 Agent 页面（限定范围验收）

在 §2.21 的实际页面选择上增加可选“记住此 Agent 默认页面”。原 `nomi_agent_presets` 行存储 `ui_binding_json`（版本 + 精确 UI contribution 或 null），沿原 ControlPlane 注入存储；Session 查询由原 owner 的元数据派生 preset。读写绑定不修改 execution Revision/Snapshot，也不创建/发送/取消 turn。无存储实现的其他宿主返回 503，不伪装为产品交付。

保存检查 live preset、同 owner 产品及当前 Catalog 精确 capability/version/release，标签由服务端规范化；独立 CAS 防止多窗口覆盖。遗漏 selection 的请求拒绝，显式 null 才清空。正常写入与恢复审计均检查 owner；插件删除后的历史引用按 KEEP_HISTORY 保留，不能凭这条记录打开失效插件。修改 fresh `060` schema，不新增兼容迁移；旧 checksum 数据集不是当前 schema，本次不操作真实用户数据。

进入路由按明确保存的不可变 release 自动申请新的 Session grant；停用/升级失效显示提示与内置页面，不自动改选。当前路由内撤销锁存，之后同 release 重新启用的下一次路由进入仍可消费原意愿。临时选择不写绑定且优先于迟到默认读取；导航后的迟到保存不会选择新 Session 的页面。保存当前已打开的同页不重开 Surface。默认仅属于该预设，不涵盖安装默认、模板传播或全 UI。

| 定向检查 | 结果与证据边界 |
|---|---|
| `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1` | 2 项通过，29.84 秒；`.tmp-open-ui-binding-product-durable.out`。本卡使用隔离的磁盘数据库、真实 HTML 发布和生产 HTTP 路由，验证保存/回读、同 preset 新 Session 默认、另一 preset 不受影响、并发 CAS、遗漏字段/错误精确目标/缺失与退役 preset/跨 owner 拒绝、展示字段规范化、停用后保留与禁止新选/打开、显式清空，以及 Revision/Snapshot/既有 Session 不变、零模型请求。原 UI 发送/取消/恢复测试保留回归 |
| 同一产品测试内的 schema/数据/持久化检查 | 调用原 `validate_id_schema_contract` / `validate_id_data_contract`；验证缺字段/负版本 SQL 拒绝、跨 owner 审计失败、非法 plugin ID 拒绝及历史缺父项允许。原 `snapshot_into` 验证当前 lineage/数据集合同，临时磁盘快照关闭重开后保留精确选择；不是整个产品重启或 side-store 备份验收 |
| `bun test --cwd ui src/renderer/pages/agentSession/AgentSessionViewHost.interaction.test.tsx` | 15 项通过。记住/清空与重进、精确新版不自动同意、保存冲突、临时选择优先、迟到保存/打开不跨 Session、重复点击保护、读取失败恢复；保存当前页保留同一 iframe，未重开授权。不代替真实浏览器 |
| `bun test --cwd ui src/renderer/pages/agentSession/AgentSessionPage.structure.test.ts src/renderer/pages/agentSession/navigation.structure.test.ts src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.interaction.test.tsx` | 9 项通过；保留公共 Session API、导航、Surface 监听卸载和恢复检查 |
| `bun run gen:i18n`、`bun run typecheck`、`bun run check:desktop-ui-boundary`、`bun run check:i18n`；`git diff --check` | 均通过；最小桌面视口保持 880×600。未运行 workspace 全量，不新增 Rust 插件执行后端 |

首轮夹具错误地重组装同一个 AppServices，触发原有 Provider 单次安装保护；修正为单次组装，不放宽生产约束。随后内存数据库的导出快照无法重开；本卡改为从初始化起使用隔离磁盘数据库，生产备份逻辑未改，最终取得上述落盘证据。未将这次夹具调整包装成通用内存数据库备份问题已修复。

此子卡收口后交付审查，不自动追加相邻功能。模板传播、无 Session 入口、完整浏览器/跨进程产品启动、多 Surface 与 Shell 验收未完成；不能用本节关闭 B、行 25 或原五问。

### 2.31 工作台会话前页面配置与空会话入口（限定范围验收）

个人预设编辑器增加 `AgentPageSettings` 页签，消费 §2.30 原有 GET/PUT binding 与同一 Catalog，不新增后端 API/表或 JS/Rust 执行器。页面偏好独立于未保存 Agent 草稿；失效精确 release 保留展示，显式选择内置页才写 null，CAS 冲突要求刷新后再决定。页面身份比较抽为前端纯函数，两处消费者复用，不是新注册表。

“打开已保存的 Agent 页面”仅以 preset ID/title 调用原 Session create，再进入既有路由。需要稳定 Revision、Agent 草稿无未保存变化和已保存页面选择；不带模型覆写/资源绑定/首条消息，不调用 turn 或 bridge。原“使用 Agent”引导与模型/资源选择保持不变。创建返回后刷新原会话历史；切换预设的迟到结果不导航，重复点击不会重复发起操作，失败提示用户核对历史且不自动重试。

| 定向检查 | 结果与证据范围 |
|---|---|
| `bun test --cwd ui src/renderer/pages/agentSettings/AgentPageSettings.interaction.test.tsx` | 9 项通过：真实 Editor 页签→独立保存→空 Session→实际 `AgentSessionViewHost` 精确 grant；Agent 草稿与页面保存分离、未保存/无 Revision 的打开保护、失效默认清空、CAS 刷新、迟到保存/创建与双击、读取/创建失败。不是浏览器或真实 HTTP 联合测试 |
| 同时运行 `AgentSessionViewHost.interaction.test.tsx`、`startConversation.structure.test.ts`、`AgentContextOrder.interaction.test.tsx`、`AgentRoleProviderPicker.interaction.test.tsx` | 保留已有 29 项回归；与本卡 9 项合计 38 项通过。原 React/Arco ref、布局 NaN 警告仍在旧编辑器测试；本卡异步初始化/刷新用 act 等待，未通过增加测试超时掩盖问题 |
| `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1` | 在原真实产品测试增加“新预设先保存、再创建第一个空会话”的顺序，检查页面归属、另一个预设不受影响、空消息与零模型请求；沿用既有磁盘/撤销/owner 验证，最终 2 项通过、12.28 秒，见 `.tmp-open-ui-entry-product-final.out` |
| `bun run gen:i18n` / `bun run typecheck` / `bun run check:desktop-ui-boundary` / `bun run check:i18n`；`git diff --check` | 通过；桌面最小 880×600 不变。未运行 workspace 全量 |

首次 UI 测试误用了不存在的 `sessions.turn` spy，改为原 `sessions.createTurn`；冲突刷新测试缺少异步 act，修正等待后保持默认测试期限通过，没有改变生产 CAS 行为。

此卡只交付无需已有会话的**工作台配置与打开入口**。插件接管创建前欢迎页/资源选择、模板传播、完整浏览器恢复、多 Surface 和 Shell 未完成；不得将“先创建空会话再加载插件”记为所有创建前 UI 均已插件化。完成后不顺带新增全局创建权限，B/行 25/全目标继续保持部分完成。

**收口复核：同会话重入的默认缓存。** `AgentSessionViewHost` 原先会在 SWR 返回旧缓存时完成一次性页面选择，随后真实读取的新默认被忽略；旧默认读取失败也可能自动打开旧插件。新增三个回归用例在旧实现全部失败，既有 15 项通过。修正为进入路由时沿原 SWR bound mutate 明确读取一次，完成前不自动解析默认；失败时不采用缓存中的默认意愿。单独检查 `isValidating` 不足以覆盖 SWR 在已有缓存时延后启动读取的窗口。临时选择、精确 release 校验和原 Surface grant 保持不变；不新增 API、缓存同步服务、数据库字段或执行后端。

定向验证：`bun test --cwd ui src/renderer/pages/agentSession/AgentSessionViewHost.interaction.test.tsx src/renderer/pages/agentSettings/AgentPageSettings.interaction.test.tsx src/renderer/pages/agentSession/AgentSessionPage.structure.test.ts src/renderer/pages/agentSession/navigation.structure.test.ts src/renderer/pages/plugins/runtime/PluginRuntimeSurfacePanel.interaction.test.tsx`，36 项通过、212 个断言。包含共享缓存快速重入时“已清空／新选默认／读取失败”三种情况；测试保留普通 SWR 的 2 秒去重窗口，不靠全局关闭缓存通过。`bun run check:desktop-ui-boundary` 与 `bun run typecheck` 通过（类型检查最终日志 `.tmp-ui-default-cache-typecheck.out`）。本次仅修改前端默认读取及其测试，不重跑 Rust 产品或 workspace 全量，不声称完成真实浏览器验证；原 29 行缺口不变。

### 2.32 参考插件页的来源消息类型与结构化展示（限定范围验收）

原 Nomi `message_projection` 只暴露正文和位置，来源 `type/status` 丢失。原 `MessageProjection` 补可选 `message_type/message_status`，由持久化消息行派生；原正文、正文摘要、顺序与 Session owner 不变，不新增表、执行器或 SDK 命令。事件型投影没有该来源元数据时省略，沿原事件合同读取。TS 正文改为 `unknown`，避免把所有 runtime 的异构 JSON 误声明为单一事件文档。

参考 HTML 按来源类型呈现 `text/tips/thinking/plan/tool_call/tool_group/agent_status`，包含折叠思考/工具文本输出、计划逐项状态、工具失败修正和隐藏记录过滤；未知/无效类型显式保留不支持提示。只创建文本节点，不自动执行工具、批准权限、打开产物 URI 或转储参数对象。工具行错误优先于正文残留成功，完成状态不是回合或产物 receipt。模板继续普通草稿发布；已有不可变 release 不自动变化。

| 定向检查 | 结果与证据边界 |
|---|---|
| `bun run test:agent-view-template` | 29 项通过：实际模板与实际 SDK 在 DOM/真实 MessagePort 上运行；新增七类展示、安全文本、未知/隐藏类型、同游标错误修正。宿主响应受控，不是浏览器/真实后端联合测试 |
| `cargo test -p nomifun-agent-session --lib -- --test-threads=1` | 25 项通过，0.87 秒；可选来源字段序列化与正文不变，以及原事件投影/存储/回合事实回归。日志 `.tmp-ui-message-types-session.out` |
| `cargo test -p nomifun-app --test plugin_ui_sessions --no-default-features -- --test-threads=1` | 最终 2 项通过，19.21 秒；真实文本回合已有来源类型，新增 repository 消息夹具验证七类来源经原 HTTP 与插件 bridge 投影一致、正文/摘要不变、零额外模型调用。夹具不是实际执行七类工具；模板 DOM 证据与产品证据分开记录。日志 `.tmp-ui-message-types-product-final.out` |
| `bun test --cwd ui src/renderer/pages/agentSession/model.test.ts src/renderer/pages/agentSession/AgentSessionPage.structure.test.ts src/renderer/pages/agentSession/AgentSessionViewHost.interaction.test.tsx` | 25 项通过，保留原事件卡片、路由与默认/撤销恢复回归 |
| `bun run typecheck`、`bun run check:desktop-ui-boundary`、`git diff --check` | 通过；最终类型检查日志 `.tmp-ui-message-types-typecheck-final.out`。无外部依赖、新浏览器平台或移动端布局 |

本卡交付“来源可区分且有七类基础呈现”，不是完整消息交互或 UI 开放目标完成。附件/产物预览与发送、权限审批、其他专用消息、完整浏览器恢复、模板绑定传播、多 Surface 和 Shell 仍保留；原 29 行范围不缩减。适合 JS 的 UI 工作不因延期 Rust 插件而自动延期，原生密集实现仍不做临时代理。

### 2.33 插件页面交互审批的前置核对（仅研究，未实施）

本轮原拟复用共享审批服务接入插件页，但当前代码不支持该前提：

| 当前代码证据 | 能说明什么 / 不能说明什么 |
|---|---|
| `nomifun-common/src/enums.rs` 的 `MessageType::Permission` | 只有消息类型名称；不是可等待、可决定的审批实体 |
| `nomifun-ai-agent/src/protocol/events/mod.rs` 的 `AgentStreamEvent`、`nomifun-conversation/src/stream_relay.rs`，以及前端 `chatLib.ts` 的 `TMessage` | 已读的 Nomi 主链未找到对应交互审批生产/消费分支；不能假定内置页已有可复用批准交互 |
| `nomifun-agent-kernel/src/authority.rs` 的 `ThinAuthority` 与 `nomi-agent/src/skill_tool.rs` | 当前 capability/action/resource 判定、Skill allow/deny 不持有交互审批等待或决定 |
| `nomifun-agent-contracts/src/plugin_runtime.rs` 的 `PluginAgentSessionRequest` 与产品 `router/plugin_ui_sessions.rs` | 当前公开命令仅 observe/turn/cancel，没有待批查询/提交决定接缝 |
| §2.32 的 repository permission 夹具 | 只验证消息类型透传与未知类型提示，没有真实工具等待/批准证据 |

据此停止“只加插件审批按钮”的实现路线，不写空 SPI、页面私有审批 KV、模拟等待或自动重发执行。前置工作归第 21/28 行共享宿主能力，第 25 行消费；顺序为真实请求产生点→原执行 owner 的等待/幂等决定与取消→内置和插件共同入口→真实执行一次及失效场景验收。批准只能进一步确认已获准操作，不能扩大 Snapshot 权限；普通页面选择不自动授予代替人类批准或自批提权能力。授权可委托与不可委托的范围需在共享能力卡落定，不由插件自行定义。

这是缺宿主生产链，不是 JS 不适合或需要 Rust 插件。未来共享宿主实现可以修改 Rust；本轮不增加 Rust 插件后端、不把该需求从本期总目标删除。其他无此依赖的 B/C/D 项可继续，不能因此宣称整目标阻塞。仅更新报告/台账，执行 `git diff --check`，不跑构建或冒充新增功能验收。

### 2.34 远程模型 Provider 开放的消费链预检（仅研究，未实施）

按报告 §27.14 从 D 选择远程模型方向进行定向预检，没有继续扩展参考 UI。当前代码不能支持“补一个 JS adapter 即完成模型替换”的估计：

| 代码证据 | 实际边界 |
|---|---|
| `nomi-providers/src/lib.rs::LlmProvider::stream`，`nomi-agent/src/bootstrap.rs::AgentBootstrap::provider/build` | 引擎已有可注入的流式模型接口，默认仍由 `create_provider(config)` 构造；不需要另造 Agent 引擎接口，但不能把进程内 Rust trait 当成用户插件装载链 |
| `nomifun-ai-agent/src/manager/nomi/agent.rs` 的生产 bootstrap、`factory/provider_config.rs`，`nomifun-app/src/router/nomi_core_session.rs::NomiCoreSessionOwner` | 当前产品 Nomi 经原 Conversation/runtime registry 构造并运行；读取的生产 bootstrap 未注入 JS 模型实现。Provider 配置仍通过现有模型/连接解析器得到，不能靠只改变平台 Broker 来证明这条路径已替换 |
| `nomifun-agent-platform/src/platform.rs::open_model_stream`，`nomifun-chat-model-broker/src/broker.rs` | 平台另有真实 Broker 消费入口，拥有路由、凭据和重试边界；构造器要求精确六协议集合，adapter 以协议为键。它不是已经支持按用户 capability 选择任意实现的目录 |
| `nomifun-agent-contracts/src/model_route.rs::ChatRouteProtocol/ChatRouteCandidate` | 路由协议是六值枚举；provider/model/connection 的路由身份不等同于插件 capability/制品身份，不能把二者字符串混用或覆盖内置 ID |
| `nomifun-plugin-platform/src/runtime/service_host.rs::PluginRuntimeServiceProcess::invoke`，`service_process.rs::dispatch` | 原 Product Service 等待 `service.invoke` 后发送单个结果；已经有 request、generation、取消和进程 owner，但没有模型增量事件和消费者背压合同 |
| `nomifun-chat-model-broker/src/adapter.rs::ProviderTransport/ChatProtocolAdapter`，`nomifun-model-invoke/src/chat_executor.rs` | Broker 有单次 HTTP 流传输和不透明凭据租约；adapter 的同步 encode/decode 不能未经设计直接跨进程调用 JS。现有能力值得复用，但不自动成为安装型 JS 接口 |

上述是静态代码核对，不是两条路径同时执行同一次请求的证据，也没有声称发现了生产重复计费或重试缺陷。未调用外部模型、未读取真实凭据、未修改运行配置。此次没有新增模型 capability、流式 SDK 或生产代码，也没有运行构建/测试。

实施依赖与首卡边界统一记录在报告 §27.14 的“D 远程模型 Provider 前置核对”。关键结论：先落实一个真实 Nomi 消费链中的选择/执行/取消所有权，再演进原 JS Service 流式合同并接入该消费者；不能单独交付无人使用的新 port，也不能要求先全面重写两套模型路径才允许首个插件实现。完整行 17 仍未实现，本问题不归 Rust 插件延期。

### 2.35 原 Service 请求的有界增量交付（模型功能卡的在途前置）

沿 §2.34 发现的流式缺口修改原 `PluginRuntimeServiceInvocation`、Service Host 与 Node Actor，不另建流服务、模型进程或 Session owner。宿主可传入有界事件通道；原 `invoke` 委托同一 `invoke_with_events` 路径且不启用事件，保留既有单次调用行为。Node 仅在本次请求显式启用流时提供 `emit(value)`，插件须逐个 await；原调用返回值仍是最终成功/失败，事件不能代替它。

实现边界：

- 每个请求最多一个尚未 ACK 的 IPC 事件。Actor 非阻塞地尝试投递，消费队列满时最多缓存该事件，原 watchdog 再检查容量；没有逐事件后台 task、轮询 KV 或全量结果缓存。其他调用、取消与原存储请求继续由同一 Actor 处理。
- 事件绑定原 generation/request/call 与递增 sequence；未请求的事件、错误身份或顺序导致该进程代际失败。沿原单帧字节上限限制事件；本卡验证的是每请求缓冲边界，不是全平台并发容量或恶意 Node 强沙箱证明。
- 消费者关闭、调用 future 丢弃或显式取消进入原取消路径；版本切换沿原 Host 的登记与代际失效，既有调用不重放。ACK 仅表示进入宿主有界队列，不表示用户已看到、模型回合完成或副作用已提交。背压不会延长原请求 deadline；超时仍按原进程级 watchdog 处理。
- JSON 编码失败拒绝当前调用，不等待不存在的 ACK；显式 `null` 最终结果与遗漏 `value` 字段区分。保留已经交付的部分事件与最终失败的区别，调用方不能据此自动重试已有可见输出的模型请求。

| 定向验证 | 结果 / 证据边界 |
|---|---|
| `cargo test -p nomifun-plugin-platform --test service_process --test service_storage_ipc --test service_application --no-default-features -- --test-threads=1` | 进程 16 项、存储 IPC 3 项、应用 2 项通过；日志 `.tmp-model-service-stream-final.out`。进程测试新增 9 项：首段早于完成、有界背压且 unary 不阻塞、三种取消/丢弃、入队前关闭、部分失败、原 deadline、身份/顺序与大小拒绝、JSON 编码失败、Host 重复登记/版本切换不重放。真实 Node，不依赖外部模型 |
| `cargo test -p nomifun-plugin-platform --lib service_response_keeps_null_distinct_from_missing_value --no-default-features -- --test-threads=1` | 收尾修正后 1 项通过：显式 null 保留为值，缺字段仍缺失 |
| `cargo test -p nomifun-plugin-platform --test service_process partial_events_do_not_turn_a_final_failure_into_success --no-default-features -- --test-threads=1` | 收尾修正后该真实 Node 用例再通过，包含部分输出后失败及显式 null 终态。是上面 16 项中的复跑，不重复累计为新用例 |

首轮进程测试 13 通过、1 个既有取消用例失败：计数等待从冷启动前开始，首个 stats 返回零时已超过两秒窗口。测试改为先正常启动并确认零调用，再测原取消/登记语义；未加大超时、未改生产取消规则。测试时间包含 Node 可执行文件校验，不作为流吞吐量测量。没有运行 workspace 全量、浏览器或真实付费模型调用。

这不是模型 Provider 已开放：产品 capability 声明/选择、模型路由与实现身份的关系、Nomi 消费者/凭据与重试所有权、模型事件语义和公开 SDK 尚未接线。当前增量通道只在宿主 Service 层显式启用；现有 Agent/Product 普通 action 不会自动变成流式模型。下一步继续同一模型功能卡的实际消费者设计和接线，不追加通用事件总线、其他协议或 UI。§2.34 的“单次结果”是预检时基线，最新底层状态以本节为准；行 17 和完整目标不因此关闭。

### 2.36 Product 内部受管增量调用（模型功能卡的在途前置）

原 `PluginRuntimeAgentCapabilityPort` 增加显式传入有界事件通道的调用方式，原 unary 调用委托同一实现并传 `None`。应用服务仍使用原 owner、启用状态、精确 release/epoch、Catalog digest、capability/action、consumer 与资源限制检查，再经原 Runtime binding 进入 §2.35 的 Host/Node 请求。不改调用身份 DTO，不新增 HTTP/页面调用权限、Catalog、Session owner 或执行器。

生产 Runtime 转发增量通道；只支持 unary 的 Runtime 在请求事件通道时明确返回不支持，不能执行一次普通调用后静默丢弃事件。原 action allowlist 语义和资源限制未放宽。通道中的 `StrictJsonValue` 仍是未经模型领域校验的值，既不是模型事件合同，也不是新增的 JSON Schema 结果校验；未来消费者必须校验事件、终态和工具调用语义。关闭接收端沿原取消链退出，不据此宣称已接入 Nomi turn 的取消。

| 定向验证 | 结果 / 证据边界 |
|---|---|
| `cargo test -p nomifun-app --test plugin_product_discovery service_stream:: --no-default-features -- --test-threads=1` | 1 项通过、11.74 秒，日志 `.tmp-model-product-stream.out`。真实源码创建/构建/发布/启用后，经产品应用服务调用真实 Node；验证首段早于完成、容量 1 背压与并发 unary、七类身份/授权变更拒绝且 JS 未执行、接收端关闭导致 JS abort、部分输出后错误不重放、原 HTTP 禁用使在途调用退出且旧身份不能再次调用。夹具为普通 Service action，不是模型或 Nomi 回合 |
| `cargo test -p nomifun-plugin-platform --test service_application --no-default-features -- --test-threads=1` | 2 项通过、6.84 秒，日志 `.tmp-model-product-stream-application.out`。已有应用生命周期与 unary 回归；在原调用测试中新增不支持流的 Runtime 明确拒绝且零事件的断言，不另计为第三个测试 |
| 改动检查 | 相关已跟踪代码 `git diff --check` 通过；新产品测试文件另行检查尾随空白通过。未运行 workspace 全量、UI、浏览器或付费模型测试 |

本切片至此收口，但模型功能卡与行 17 不关闭。下一批须落实模型实现的选择/冻结身份、已有模型合同的复用、凭据/重试所有权和原 Nomi 实际消费者；不能继续以新增内部通道或通用事件接口代替该结果。公开模型 SDK、N1 流支持、资源绑定等未由本卡交付；不得把这次“产品内部入口”写成用户已有模型插件配置入口。Rust 插件后端仍未新增，原生密集具体实现仍按 §27.13 延期。

### 2.37 复用同一模型数据合同（归位完成，消费接线未实施）

为避免后续插件发布层另造一套模型消息协议，将原 `nomifun-chat-model-broker/src/contracts.rs` 的纯数据定义及验证移动至 `nomifun-agent-contracts/src/chat_model.rs`；Broker 原模块直接重导出同一类型，旧调用方无需转换。没有新增依赖、Broker 实例、模型注册表、执行端口或插件后端；路由解析、凭据租约、传输和重试实现均留在原层。只公开原 wire encoder 使用的 `tool_call_names` 数据查询方法，不授予工具执行权限。

| 验证 | 结果与边界 |
|---|---|
| `cargo test -p nomifun-chat-model-broker --lib --test conformance -- --test-threads=1` | 10 项单元测试与 18 项一致性测试全部通过，日志 `.tmp-model-canonical-contract.out`；覆盖原六协议、凭据拒绝、重试归属、语义输出后禁止切路由与流丢弃。新增一项验证新旧路径可直接赋值、反序列化和序列化，无第二份数据类型 |
| 移动内容核对及空白检查 | 对照原文件，除模块说明、crate 内引用、查询方法可见性及说明外，定义和验证代码不变；相关 `git diff --check` 与新文件尾随空白检查通过。没有扩大协议枚举、修改线格式或重新生成无变化的 schema 制品 |

定向阅读另确认三项消费接线条件，**本轮未实现转换器**：Nomi 把工具结果放在 user 消息中，canonical 合同要求 tool role，转换须保留混合消息内容顺序；canonical 原始工具参数增量不同于 Nomi 的结构化预览，不能直接强转；模型 `Completed` 不等于 Service 最终成功，不能提前向 Nomi 确认 Done 后忽略调用失败。原 Nomi 已校验工具名/参数、重复调用和终态，后续复用这些检查，不另建工具执行 owner。空工具结果、推理配置和 provider round 等语义仍需逐项验证，不将类型复用等同于无损转换已完成。

模型配置的 provider/model/connection 身份仍不等于实现 capability/制品身份；原 factory 会解析宿主凭据，当前 Service 增量通道并未提供模型连接的受管授权传输。不得把宿主 API key 放入普通插件 payload、使用虚构 Provider 配置或伪造 Broker causality 来跑通演示。下一批沿同一真实模型功能卡解决选择与连接授权，再接原 Nomi 消费者；不是新增 Rust 插件才能解决的问题。

按用户“尽快收尾、不要过度发散”的要求，本批在合同归位、原消费者回归和文档同步后结束，不继续编写转换器、凭据桥、通用事件平台或 UI。此项是内部归位而非新增用户功能，不抵扣行 17 的完整验收；整目标保持未完成。

### 2.38 远程同步与新 Engine 架构整合

已在 `rf/agent-capability-platform-v2` 快进到 `2ff029512`。同步前所有本地改动及未跟踪文件通过 stash 完整备份，并保留 `refs/backup/plugin-openness-before-sync-20260914` 和 `refs/backup/plugin-openness-worktree-20260914`；不删除快照、不提交、不推送。本地与远程有 56 个已跟踪文件重叠，恢复时产生 36 个冲突路径，按语义整合而非整体选择一方。

整合内容及限制：

1. 接受远程移除的旧 AgentPlatform 执行宿主与 release fixtures，将本地 Role Catalog 投影迁至 `nomifun-agent-control-plane::kernel_catalog`。保留新的 Engine 选择/运行准入、原执行权限限制、资源和副作用结算；Provider 选择及贡献排序仍走原唯一 Compiler。
2. 插件恢复只在 Kernel 成功发布后保存注册来源，且发布与保存之间没有新的 await。MCP 刷新复用此来源，不能抹掉已恢复插件；持久化 inventory 仍是安装状态权威，不增加第二套依赖解析器。
3. 生产 Nomi Skill 装配不再同时调用两个制品读取器。共享 `engine_skills` 读取/验证后，模型消费其正文及类型化资源，显式命令复用已验证正文并挂接原 Kernel 来源/依赖检查。命令标记为仅显式调用，不增加第二套模型 Skill 加载入口；受限执行会话不增加 Skill 命令。包含不支持执行模式的正文仍是参考数据，不生成 shell/fork/hooks 命令。低层旧 package adapter 的定向测试保留，但不再作为生产会话的第二装载链。
4. 保留上游 Provider 推理块语义。纯数据及验证仍只有 contracts 层一份类型；精确 route 绑定/match 作为 Broker 内部扩展保留，不为公共数据类型添加凭据或执行权限。
5. 工作台保留“能力 / Provider / 设置”三个页签，运行引擎选择与本地定制同时存在。此处是兼容整合，不是新 Rust 插件系统，也不代表安装型 JS 引擎已支持。
6. 受限执行会话除了不装载插件工具，也不执行本地新增的 Context、发现策略和模型 middleware；否则会从工具以外的通路绕过上游执行限制。相关回归覆盖零 Context 执行、无动态贡献和不绑定 Product middleware；这不是放宽权限或改变普通完整会话的选择。

本次合并后的定向验证如下；§2.1～2.37 的历史通过数量不外推为当前通过：

| 检查 | 本次结果与边界 |
|---|---|
| `cargo check -p nomifun-app --tests --no-default-features` | 通过，日志 `.tmp-sync-app-final.err`；覆盖 App 及测试代码编译，仍有既有 unused/dead_code 警告，不等于执行全部 App 测试 |
| `cargo test -p nomifun-ai-agent --test plugin_tool_consumer --no-default-features -- --test-threads=1` | 39 项通过，日志 `.tmp-sync-consumer2.out`；含真实 Node、Context/发现/middleware、Skill 命令与执行权限限制回归 |
| `cargo test -p nomifun-chat-model-broker --lib --test conformance -- --test-threads=1` | 10 项单元与 22 项一致性测试通过，日志 `.tmp-sync-broker2.out`；合同同一类型、六协议、重试归属、取消/凭据边界，不是调用外部模型 |
| `cargo test -p nomifun-app --lib router::plugin_platform::restore_tests:: --no-default-features -- --test-threads=1` | 3 项通过，日志 `.tmp-sync-restore.out`；覆盖跨包恢复、坏插件隔离/发布缓存保留、默认变更与冻结会话选择 |
| `cargo test -p nomifun-app --test official_preset_catalog_integrity skill_discovery:: --no-default-features -- --test-threads=1` | 1 项通过，日志 `.tmp-sync-skill-product.out`；真实产品安装/保存绑定/HTTP 命令发现，不启动 runtime 或 Context，并验证来源撤销 |
| UI 类型与交互 | `bun run typecheck` 通过（`.tmp-sync-ui2.out`）；OfficialTemplateOverview、AgentRoleProviderPicker、AgentContextOrder 三个定向文件 14 项/63 断言通过（`.tmp-sync-ui-tests.err`），不是浏览器端到端验收 |
| UI 边界与语言资源 | `bun run check:desktop-ui-boundary` 通过（880×600）；`bun run check:i18n` 通过，6 个自测、34 个模块/7909 个键 |
| canonical 生成物 | 按合并后的源合同运行 `agent-v2-contract write` 后，直接运行生成器二进制的 `check` 通过；不恢复上游已退休的 release fixtures |

Broker 首次运行有 3 项失败：上游 Anthropic/Bedrock/Vertex 样本仍设置 512 输出上限、128 推理预算和显式 effort，与新 Messages encoder 的校验不符。仅将对应样本修正为 2048 输出上限、1024 推理预算、无显式 effort 后重跑通过，没有放宽生产校验；共享模型合同对照上游除归属引用及既有查询方法可见性外，保留同一数据/验证语义。

本批已按新基线兼容整合的停止条件收口；`git diff --check` 通过，无未解决冲突，暂存区为空，同步前备份保留。完整模型 Provider 用户功能仍未交付；本批不新增其执行器/凭据桥。没有运行 workspace 全量、浏览器端到端或付费模型测试；本节的定向通过不能代替整个全栈插件平台验收。

### 2.39 新 Engine 基线下的模型接线核对（2026-09-15，仅决策）

继续报告 §27.14 的同一模型功能卡，核对范围限定在 Nomi 构造、Engine 模型入口/调用事实、Product 增量调用和模型连接解析，没有再做全仓架构盘点。

| 直接源码依据 | 当前事实及实施影响 |
|---|---|
| `nomifun-chat-model-broker/src/engine_port.rs`；`nomifun-app/src/router/engine_session_host.rs::open_model_port` | 已有 `EngineModelPort` 和 Broker 实现；后者接收真实 turn receipt 并创建 journal。可复用，不新建第二模型端口；不能跳过调用事实绑定 |
| `nomifun-app/src/router/engine_journal.rs::append/authorize` | 模型操作有持久登记和一次认领，且校验 Session/turn/根消息/Snapshot/路由和取消状态。随机生成一个 operation ID 不能替代先登记，按会话共享旧 causality 也不能支撑多次模型采样 |
| `nomifun-ai-agent/src/manager/nomi/agent.rs` 的生产 bootstrap；`nomi-agent/src/bootstrap.rs::provider` | Nomi 有 provider 注入接缝，但产品构造尚未通过该接缝消费新 Engine 模型入口。不能把编译期引擎扩展的交付等同于 Nomi 模型替换 |
| `nomifun-ai-agent/src/factory/provider_config.rs::resolve_provider_fields_at_revision` | 当前仍按内置协议解析实际连接，并将认证材料构成原生 provider 配置；这份配置不可整体传给 JS。模型路由与实现 capability 需要分开，不用虚构 Provider 配置跑通演示 |
| `nomifun-plugin-platform/src/runtime/m1_application.rs::invoke_agent_capability_inner` | 保留已有增量调用，同时明确拒绝非空资源需求。真实模型连接的授权/受管传输仍缺失，不以删校验、普通 payload 带密钥或另起本地代理绕开 |

决策及三个相依实施切片统一记录于报告 §27.16，不在本台账复制第二份排期。下一实施切片是 **M1 的真实 Nomi 主循环接入现有宿主模型入口**；之后是模型 Role/受管连接和 Product 发布选择闭环。模型配置的公开字段与认证材料须保持分离；JS 请求成功前不能提前将模型终态或工具调用当作完整成功。

**本次没有修改生产代码、添加空 SPI、开放模型 capability 或交付用户模型替换。** 这是局部实施决策，不抵扣行 17 的用户验收，也不重记 §2.38 的测试为本次新证据。只做文档空白检查；文档改动不运行 Rust/UI 构建、全量测试、浏览器或付费模型。本阶段没有新增 Rust 插件后端，原生密集实现仍按 §27.13 延期。

### 2.40 同步与原生插件删除的历史记录（2026-09-15）

当时按授权安全同步到 `e617feb2b`（7 个上游提交），保留既有改动和同步备份，当轮没有 commit/push。曾实现 Rust 原生 Service 后端及作者 SDK，后按用户范围收敛决定删除；代码删除已完成，最新检查与待补回归见发布台账 §2.5。

原生专属架构、启用、SDK、打包及测试开发方案不再保留。Windows 原生进程和 application 的 2＋3 项曾通过，仅作为历史事实，不代表现行支持，不是删除后的验证证据。共享 Service 流式与模型合同工作见 §2.35～2.39，保留且不改写为原生插件专属能力。

本节保留的非 native 历史验证（不外推为当前候选通过）：

| 检查 | 历史结果/证据范围 |
|---|---|
| Node `service_process` + `service_storage_ipc` | 16＋3 通过；首次并行流式事件等待超时，串行完整重跑通过，未放宽断言或跳过失败 |
| 共享 `service_runtime` | 4 项通过，涉及 Node 候选切换、run-key 与一次性 Test receipt |
| 合同及 App 组合检查 | 当时合同 100 项及生成器 write/check、App tests 编译检查通过；首次编译时序交叠后冻结源码重跑，既有告警保留 |

当轮未执行 workspace 全量、浏览器/Nomi 主循环端到端、付费模型或其他 OS/架构验收。不据此关闭 29 项长期领域清单或完整模型/UI/Runtime 替换目标。


<a id="3-原始-29-个环节不得缩减的跟踪清单"></a>

## 3. 原始 29 个环节：长期能力评估清单（非上线欠账）

29 项保留用于长期评估，不承诺全部实现，不是本次上线必须清零的欠账。下表的“待核/实施”“在途前置”“适合本期”及后续动作保留历史语义；当前正式、实验、延期范围以顶部及发布台账为准，尤其模型插件、整 runtime、Shell 和深基础不排入本轮。

下表使用报告 §27.2 的同一编号。`待核/实施` 表示本轮尚未取得完整验收证据，不表示代码中完全没有基础；`部分` 也不能关闭该需求。已有统一工程与历史平台证据需要按当前目标复核，不能从提交名推断全量通过。

| 编号 | 范围 | 当前状态与后续动作 |
|---|---|---|
| 01 | Catalog / 发现 | 部分：共享目录、用户 Context 准入及同快照 Role/Provider 候选已接线；模型工具发现/排序策略的已有切片见行 05，不能反推 Catalog 候选发现/排序策略已全部开放 |
| 02 | Preset / 依赖 / 模板 | 部分：用户 Role、选择/默认保存、冻结执行、不同资源/特性需求与跨包恢复已有切片；冲突见 §2.12，受管调用见 §2.13～2.14，递归异构依赖/消费用途及验证见 §2.15；全量领域消费者/升级影响、全局故障诊断、历史计划迁移与模板发布仍待实施 |
| 03 | Session / 资源绑定 / 初始化 | 待核/实施：可扩展资源解析及初始化贡献 |
| 04 | 初始 Tool | 部分：当前 Plugin 消费定向测试通过；全量替换/旧旁路仍需核对 |
| 05 | on-demand / ToolSearch | 部分：§2.24～2.25 接入排序/激活、隐藏策略与安装型 Role/JS；§2.27 接通 Product 直接选择、保存冲突及真实调用。schema 暴露策略/预算、大目录验收仍未完成；Product Role Provider 尚不支持；模型可见性不等于 Kernel 新增授权 |
| 06 | Context / persona / Prompt | 部分：初始/动态消费见 §2.10，用户顺序/工作台保存见 §2.11，Context 发起受管依赖 action 见 §2.18。跨全部贡献的总预算、其他组件相对顺序和完整 Prompt 管线仍待实施 |
| 07 | Skill | 部分：精确包锁/只读消费见 §2.8；显式命令/补全见 §2.9，冷启动产品证据见 §2.16，附图/装饰输入消费见 §2.17；其余来源/执行模式、完整权限提示、历史制品保留和迁移仍未完成 |
| 08 | MCP | 部分：上游新基线已有生产目录、映射工具及资源消费；本批整合其发布与插件恢复（§2.38）。用户替换、所有来源和故障覆盖仍待验收，不再按没有生产链从零建设 |
| 09 | ResourceProvider | 部分：所选 Provider 的不同私有资源需求、隐式工厂可用性与真实 Node 消费已验证；产品可扩展 kind、多 binding 与租约竞态仍待实施 |
| 10 | 发布型 Service 资源 | 待核/实施：带资源调用和统一授权 handle |
| 11 | Browser / Computer Role | 部分基础：通用 JS typed Role 映射、不同私有资源需求已验证；真实 Browser/Computer 用户 Provider、领域资源/动作与全消费者替换仍待实施 |
| 12 | Turn Middleware | 部分：§2.28～2.29 验证主模型循环 before_model、Product 发布来源与用户指定冻结顺序；其他阶段、独立模型调用和 N1 来源仍未完成，不能以单阶段关单 |
| 13 | EventSource / EventConsumer | 局部接缝：UI 的授权 `message.stream` 投影和有界交付见 §2.20；通用事件贡献、其他消费者和生命周期仍待实施 |
| 14 | Lifecycle / BackgroundService | 待核/实施：作用域生命周期、健康、恢复与资源回收 |
| 15 | Transport / Channel | 待核/实施：部署/安装级 Provider 与统一 Session API |
| 16 | Scheduler / Automation | 待核/实施：触发、调度、任务执行与单一所有权 |
| 17 | 模型 Provider / 路由 | 在途前置：§2.34 定向预检，§2.35～2.36 原 Service 有界增量与产品内部受管入口已有真实 Node 证据；§2.37 已有模型纯数据合同归位并保留 Broker 消费。Nomi 实际消费者、插件选择/路由身份、模型事件转换和凭据/重试边界未接通。用户 JS 模型替换仍未交付，不因 Rust 插件延期而删去 |
| 18 | 上下文选择 / 历史 / 压缩 | 待核/实施：可替换策略，视图不篡改持久化事实 |
| 19 | 长期记忆 / RAG / embedding | 待核/实施：资源与各策略可独立绑定、溯源与删除 |
| 20 | Planner / 推理循环 / 停止 | 待核/实施：Nomi 内策略与整个 Runtime 的替换分别验证 |
| 21 | 工具编排 / 并行 / 结果转换 | 待核/实施：实际调度、副作用重试与授权边界；交互审批需先补共享宿主请求/等待/决定链，不能用页面模拟，见 §2.33 |
| 22 | 子 Agent / 多 Agent 协作 | 待核/实施：协调、子任务授权/预算/取消传播 |
| 23 | 整个 Agent Runtime | 部分基础：上游已有编译期 Engine Catalog/SDK、共享 Session owner 和 Coding 引擎；不是安装型 Rust/JS 插件后端。JS 替代引擎及插件生命周期接入仍待实施，复用新基线而非重建 Registry |
| 24 | 日志 / tracing / 评估 / 计量 | 待核/实施：用户导出/评估与慢消费者隔离 |
| 25 | UI / Agent 页 / Shell | 部分：§2.19～2.20 命令/实时流接缝；页面声明、同目录候选与真实路由局部选择见 §2.21；普通草稿参考页、分页历史、显式重试与七类来源消息呈现见 §2.22/§2.32。预设级持久默认及会话前工作台配置/打开见 §2.30～2.31；附件/权限等交互、模板传播、插件创建前欢迎页/资源选择、完整浏览器恢复体验、多视图和 Shell 仍未完成，属于适合本期 JS/UI 的剩余范围 |
| 26 | Session / 事件 / checkpoint 存储 | 待核/实施：部署级 port、替代后端、一致性与迁移验证 |
| 27 | 编译选择策略 / Registry 存储 | 待核/实施：可替换策略/存储但保留一个最终校验权威 |
| 28 | 认证 / 授权 / Secret | 待核/实施：部署者选择，普通插件不能自批权限；共享交互审批与可委托决定范围见 §2.33，不因 Rust 插件延期而移除 |
| 29 | supervisor / sandbox / Kernel | 待核/实施：监督/隔离后端接口；最小信任根及独立 Kernel 的例外不算普通插件完成 |

## 4. 接续顺序与完成审计

本节以下是历史接续计划与长期能力完成标准，不再驱动本轮开发；当前只按 [发布台账](2026-09-15-plugin-release-readiness.zh.md) 的六项 P0 收敛。历史编号未关闭不等于发布阻塞，历史编号关闭也不等于本轮发布验收通过。

**当前执行优先级只在报告 §27.14 维护。** Product 发现策略批次按 §2.27 收口；A 卡的 before_model 消费者、Product 接线和用户排序按 §2.28～2.29 收口，限定范围之外的缺口保留在 §3。

剩余工作分组为：B，Agent 页面产品闭环；C，资源/领域/Skill/MCP/生命周期；D，深层策略及异步业务；E，独立 JS Runtime 与 Shell。分组不是必须串行清空的阶段。B 的 §2.30～2.32 子卡已收口，先交付审查，不自动扩展；附件/权限等交互、创建前插件 UI、完整浏览器恢复与模板等缺口仍在本阶段。C/D 按具体依赖拆卡交错，不等待整个 B；E 的两项分开验收，不是无限扩展的大批次。原生密集和部署底层具体实现按 §27.13 延期，不建立临时 JS 替代；普通 UI/异步业务与必要宿主修改不随之延期。

已收口 A 卡的边界、主责、原始估计和停止条件见报告：只开放一个实际请求阶段，N1 来源和其他阶段仍未完成；多贡献顺序、工具权限快照、错误/取消及真实产品消费证据见 §2.28～2.29，不从头重做。现有 Context 每次模型请求的追加行为不抵扣通用 middleware。完成一张卡后先报告结果，不自动加做相邻 UI、模型或整个 Runtime。

已有实现不从零重做，证据按 §2 的具体子项抵扣；完整缺口继续看 §3。进入新领域只重读有关架构/消费者，运行直接覆盖本次改动的检查，避免反复全仓核查。跨包历史兼容、Host 默认高并发风险、全局恢复诊断等已有未完成边界仍留在所属卡，不因整理排期消失。

每个编号关闭前须同时具备：可发布/安装的贡献、用户选择入口、唯一编译/锁定语义、真实生产消费者、失败与取消/回收测试、相关旧分支退出，以及与范围相匹配的平台证据。整目标目前保持进行中。
