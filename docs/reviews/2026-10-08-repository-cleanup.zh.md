# NomiFun 全仓代码审查与清理记录

日期：2026-10-08。审查基线：`3b1fb52e2`。状态：当前仓库的清理和本地验证已完成。

本轮清理确认没有生产消费者的实现、由旧测试维持的机制、重复包装和失效规格，同时修正验证过程中暴露的旧夹具与跨平台测试问题。现役功能的入口、数据所有权、权限、取消、恢复和实际回执作为保留依据。现役设计以架构文档和源码为准，本记录保存本次审查证据。

## 差异统计

清理提交 `a7ed69e29` 相对基线涉及 305 个文件，净减少 13,457 行：代码、测试和配置净减少 12,854 行，文档净减少 373 行，锁文件净减少 230 行。统计包含该提交中的报告，排除生成构建产物和本机日志。

两个失效 crate 完整退休，各业务与共享 crate 的 13 项无用直接依赖及退休机制的消费者引用删除，两项只用于测试的依赖移到 dev-dependencies。测试统计按最终 crate/runner 汇总，同一用例的修复重跑与中间定向测试不重复相加。

## 覆盖与方法

基线包含 70 个 Rust crate（agent 11、backend 54、shared 5）、Desktop 与 Web 两个宿主。前端扫描覆盖 18 个产品区及 showcase、共享组件、适配层、worker、翻译和样式。构建、发布、验证脚本及现存设计文档同时纳入扫描。

先建立文件库存、Cargo 与 feature 入口、前端 AST 生产闭包和依赖关系，再扫描孤立文件、导出、方法、字段及仅测试引用的候选。删除候选逐项核对全仓调用、动态入口、平台条件、替代实现及相关行为测试；其他文件按入口、引用和 feature 边界扫描。文件长度与测试数量没有作为删除依据。

| 功能范围 | 核对与保留的生产链路 |
| --- | --- |
| Agent、会话、协作、控制平面 | canonical Session、Turn、Event、Payload、Effect，binding transition、原生 checkpoint、调度及未知效果围栏 |
| 模型、协议、媒体创作 | 精确 provider/model/task 路由、重试与故障转移、真实生成服务、任务接纳和作品归属 |
| 伙伴、记忆、技能进化 | SQLite 伙伴记忆、typed memory owner、当前 Skill Sink、采集与学习的所有权 |
| 渠道、客服、机器人、语音 | 原子用户撤销、连接代次、设备授权、音频协议、当前 WS 与生命周期 |
| 定时任务、需求、IDMM | typed scheduling owner、durable reservation、generation fence、canonical 投递与取消 |
| 知识库、文件、Office、素材、画布、模板 | 事务、路径与 SSRF 限制、预览所有权、后端模板 repository/run aggregate、任务轮询和失败隔离 |
| MCP、Gateway、Remote、认证 | 现役传输、工具准入、安装令牌、JWT/CSRF、Host 凭据边界 |
| 终端、SSH、浏览器、Computer | ProcessSupervisor、真实 PTY、SSH teardown、平台宿主、输入和资源围栏 |
| 插件与小程序 | 沿用基线已完成的插件清理；本轮全仓编译、前端及应用回归继续覆盖共享接缝 |
| Desktop、Web、构建与发布 | 原生窗口与 IPC 身份、完整 bundle、签名、更新、启动监督和静态 WebUI 构建 |

## 删除与收敛

| 清理项 | 删除证据 | 保留或替代路径 |
| --- | --- | --- |
| `nomi-protocol` 旧 JSONL 宿主协议 | Command/Event/Emitter/stdio reader 只在本 crate 与专属测试使用 | 唯一生产使用的 ToolCategory 迁入已有 nomi-types，wire 名称保持一致 |
| `nomi-memory` 旧文件记忆库 | 唯一生产引用来自无人构造的 CompanionStoreSink 镜像桥 | 伙伴 SQLite 记忆、CompanionAgentCapabilityOwner、Wave1CompanionMemoryHost 及 owner receipt |
| AgentRegistry 握手后台任务 | 没有生产 sender；任务持有 sender 与自身 Arc，永久等待 | 真正的 Agent 注册、刷新和 metadata 配置读写 |
| 握手缓存 DTO 与数据库列 | 五个发现缓存字段及 yolo_id/available_modes 无当前生产消费者 | 003 前向迁移删除七列，保留其他配置与 canonical Agent 表 |
| 旧 Cron Sink 和工具 | 启动不注入单例，消费者只剩旧工具与测试 | 当前 typed 调度 owner 与规范回执 |
| 原始 shell hooks、旧配置初始化及独立 Session 文件设置 | 配置仅复制，HookEngine 从未构造或执行；初始化模板仅旧测试使用 | 当前 ToolMiddleware、macOS SupervisedShell 与 canonical Session |
| 测试专属旧 PTY、PersistentShell | 仅 cfg(test) 模块互相调用，架构测试反而要求它们存在 | ProcessSupervisor、现役 PTY test helper；边界现在禁止恢复旧源及依赖 |
| Gateway 空 streaming 层 | 没有构造注册者或派发消费者 | 真实 MCP/HTTP/WS transport 的流式协议 |
| Channel、Weixin、Voice、SSH 的旧包装 | 旧清理、授权、SSE、别名和投影没有生产调用，或仅自测 | 原子 revoke_user、带 shutdown 的 pairing 定时器、WS payload、当前 voice lease/route |
| 空视觉拒绝缓存 | 生产从不写入，所有查询永远为空 | 精确协议的 ImageInput 特征判断 |
| 旧伙伴 UI 与预览 | 生产无调用，仅旧预览或旧结构/组件测试引用 | /nomi 的 WorkspaceShell 与 CohabitView |
| 模板本地 document/reducer/serialization | 仅旧测试使用，与后端现役聚合重复 | backend template repository 与 run aggregate |
| 旧画布菜单、面板、receipt/recovery 包装 | 生产 AST 与全仓调用均不可达 | 当前 zoom popover、controller、任务轮询与并发保护 |
| 闲置 StarOffice、computerPermissions、task 客户端组 | 没有 UI 消费者；task 为假成功 stub | 当前 systemPermissions 和真正的业务接口 |
| 孤立翻译与类型 | 只剩已退休接口、loader/type generator 引用 | 删除 StarOffice namespace 并重新生成 i18n 类型 |

其他无人调用的便捷方法、常量、无写者锁、依赖和重复校验随所属链路闭合删除。Completion 有效回归改为直接验证生产入口。旧实现的专属自测一并退役；权限、并发、恢复和真实失败用例保留。

三篇旧创作与伙伴施工规格已退休。其中包含已撤换的工作台路由、旧预览和历史消息/草稿导入要求；历史通过 Git 查询。当前指南、通信架构及 crate 目录已同步。

## 数据与协议边界

`001_canonical_baseline.sql`、`002_simplify_plugin_library.sql` 内容及校验值保持原样。新增的 [003](../../crates/backend/nomifun-db/migrations/003_remove_agent_handshake_cache.sql) 只删除 agent_metadata 的七个旧缓存列，Agent schema/generation 与事实表保持现役合同。

迁移回归使用隔离数据库，覆盖已知前缀升级、重启、其他 metadata 与用户配置保留、canonical 表定义保持一致，以及未知 checksum、缺失基线和失败 receipt 拒绝。边界脚本精确限定七条 DROP COLUMN，不能借该例外改动 canonical 事实或 lineage 收据。

伙伴旧配置中的 bridge_to_memory_dir 由现有 non-Agent 配置加载器有限剥离，测试验证 null/path 两种旧值及其余设置原样保留。记忆内容仍由现役存储读取，不导入或重建历史 Agent 数据。

## 验证中修正的问题

| 问题 | 修正与验证依据 |
| --- | --- |
| JWT blacklist 清理从未进入生产 | blacklist_token 清理过期撤销项，保留 expiration leeway 的有效性测试 |
| 旧初始化/安装审批/SessionPurpose 夹具 | 清理旧初始化专属测试；安装 fixture 直接使用现役接口；会话 fixture 明确设置 canonical purpose |
| 前端结构断言定位到旧入口 | 改为核对现役 MCP、Skill 和重试入口 |
| Guid 断言被全局侧栏刷新污染 | 隔离侧栏读取 fixture，保留全部创建请求和 projection 断言 |
| 素材删除测试约 15 秒超时 | await act 完成异步更新，原失败隔离和仅重试失败项断言保留；定向耗时降到约 100–250ms，未提高 timeout |
| Cargo spawn 错误后 stdout 读取不结束 | 报告读取与子进程 completion 并行等待，finally 释放 reader/stream；新增缺少 Cargo 的真实失败回归 |
| macOS 脚本在 Windows 上硬编码斜杠 | 使用 node:path，保留目录归属、准确 binary suffix、cache seal 与 generation 围栏 |
| 混用 Bun 与 node:test | 按源码使用的 runner 分组执行，并修正 voice smoke runner 命令 |
| POSIX 权限与信号 fixture 假设 Windows 行为 | POSIX 专项明确平台条件；Windows 文件拒绝和真实 native termination 继续执行，未弱化生产验证 |
| 多条 migration 收据被 fixture 改成同一版本或插入已占用版本 | 仅修改最后一条收据为当前最大版本加一，继续验证未知 lineage 拒绝与原数据保留 |
| 取消测试要求已被停止的用户脚本一定写出 marker | 保留准入 race 顺序，以平台 PID、owner 回执和进程观测验证退出；Unix 迟到探针只读，禁止对可能复用的 PID 发终止信号 |
| 所谓 offline 抠图测试可能下载真实模型 | 使用两个封闭 loopback 503 fixture，验证回退和坏缓存保留；生产下载策略保持原有语义 |
| 退休 Agent 的已有 Session GET 返回 404 | 展示读取复用已验证的当前代 exact owner、冻结 Revision/Snapshot，再读取同 preset 的展示 metadata；新 Session 准入仍拒绝退休配置 |

## 保留理由

- Vite 注入的 IconParkHOC、根生成器使用的 localeKeyParity、动态翻译入口都有实际消费者。
- 当前 renderer 不导入的 generated protocol bindings 属于共享 Rust 合同，按生成来源保留。
- Session、browser、process、SSH 的测试 seam 支撑真正的失败、取消、owner 清理、未知效果和恢复，不能只按生产引用数量删除。
- macOS/Linux 及 feature 专属代码由宿主与条件入口使用，不以 Windows 当前编译可达性判断其无用。
- 大型知识、文件、伙伴和运行时文件包含大量有效 inline tests；本轮没有为减行数删除这些行为验证或做无依据的文件拆分。
- 本轮修复的是确定的死链、重复与验证缺口；其余复杂设计按当前生产职责保留。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| 全前端 bun test --cwd ui | 4,200 通过，0 失败，723 文件 |
| 全仓 bun run check | 通过：类型、桌面、i18n、主题、图标、CSS、安装器及架构边界 |
| cargo check --workspace --all-targets | Windows 全目标编译通过 |
| Bun 脚本组 13 文件 | 82 通过，20 平台跳过，0 失败 |
| Node 脚本组 6 文件 | 53 通过，1 平台跳过，0 失败 |
| 前端生产构建 | 通过，7,711 模块，51.84 秒 |
| Agent 生成合同 | 已重生成 Cargo.lock 摘要对应的三个文件，check 通过，未改变 API/schema 语义 |
| Voice Rust 合同 | check 通过，wire/schema 未改 |
| 受影响 Rust 库 38 个 | 7,234 通过，27 原有 opt-in/平台专项跳过；修复失败项后全绿 |
| Channel 全 features | 878 通过，0 失败；已在库统计中，不重复相加 |
| Control Plane 退休配置与 owner 验证 | 完整 55 项通过，已在库统计中 |
| 前向迁移、认证、渠道及应用 E2E 10 targets | 114 通过，0 失败 |
| Process runtime 最新只读探针回归 | 完整 133 项通过，已在库统计中 |
| diff 与已发布迁移校验 | diff --check 通过；001/002 没有差异 |

平台专项不包含本机无法执行的 macOS CEF、Linux 原生及物理机器人验收。真实外部模型调用只接受已有 opt-in 与隔离凭据要求；本轮自动验证使用仓库的本地替身与受控 fixture。

`scripts/check-voice-contracts.mjs` 的 TypeScript 检查指向独立 sibling Mobile 仓库中的 `src/features/voice/contracts.generated.ts`，本机该文件不存在，检查未通过。本轮没有改动独立 Mobile 工作区；当前仓库的 Voice Rust 合同和协议 fixture 通过。

十个 integration targets 为 agent_metadata_schema、plugin_library_schema、id_schema_contract、jwt_tests、session_action_integration、architecture_contract、official_preset_catalog_integrity、nomi_core_route_gap、plugin_e2e、skills_builtin_e2e。canonical route gap 完整 42 项、官方预设目录完整 3 项、插件完整 21 项均通过，退休配置展示读取及伙伴实际记忆 owner/receipt 获得真实接缝覆盖。

## 复现要点

- 前端：`bun test --cwd ui`、`bun run check`、`bun run build:ui`。
- Rust 编译：`cargo check --workspace --all-targets`。
- 命令测试先执行 `cargo build -p nomi-tools --bin pty_test_helper`，库测试再按受影响包运行 `cargo test --lib --no-fail-fast`，Desktop 接缝启用 `nomifun-app/browser-use,nomifun-app/computer-use`，本轮固定 `--test-threads 4`。
- Agent 生成合同：`cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`；Voice 本仓合同：`cargo run -p nomifun-voice-contracts --bin voice-contract -- check`。
- Node 文件须使用 `node --test`：agent-reliability-collect、agent-reliability-report、idmm-demo-evidence、owned-native-deadline、probe-stepfun-tool-schema、run-agent-voice-smoke 六个 `.test.mjs`；其余脚本测试用 Bun。不要把 Node 嵌套用例交给 Bun 后将兼容性错误认作产品失败。

本机 `.tmp-review/` 保存完整命令输出与扫描库存，未作为新的生产代码或发布输入提交。原始失败和最终重跑日志分别保留；本记录的通过数采用最终结果。

## 提交前远端合并验证

清理提交完成后，合并远端 main 的六个新增提交，远端头为 `8e3a0c0ea`。保留 v0.8.0 发布、更新器修复和 Agnes 模型合同变更；仅 Cargo.lock 发生冲突，处理为保持两个退休 crate 的删除及所有现役包的远端版本。合并后的锁文件由 `--locked` 检查确认，三个 Agent 生成合同摘要重新生成并校验通过，API/schema 语义未变化。

以下是合并后的定向重跑，不与前面的全仓测试数量重复相加：

- `bun run check` 全项通过。
- Windows `cargo check --locked --workspace --all-targets` 通过，仅已有未使用警告。
- API 类型、模型调用和 System 三个库共 1,176 项通过（523、488、165），无失败或忽略。
- model_fetch_routes 与 provider_model_routes 共 53 项通过（40、13）。
- 创建参数、视频画布、更新器及错误展示的 10 个 UI 文件共 83 项通过，无失败或跳过。
- macOS bundle 脚本 4 项通过，10 项在 Windows 按平台条件跳过。

日志保存于 `.tmp-review/merge-*.log`。前述独立 Mobile 生成文件和原生平台验收限制仍适用。
