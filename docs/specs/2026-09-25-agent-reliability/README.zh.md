# Agent 执行可靠性重构与验收

日期：2026-09-25。状态：实施中，**尚未证明 99%**。

最新开发交接见 [DEVELOPMENT-HANDOFF.zh.md](DEVELOPMENT-HANDOFF.zh.md)，分阶段验收见
[TEST-MATRIX.zh.md](TEST-MATRIX.zh.md)。本文保留各阶段的历史状态和失败记录，不把后来实现追写为当时已通过。
用户已选择“开发优先、后续机器分阶段测试”；新增用例写好/编译通过不代表行为验收完成。
最新补充实现见 PAUSE-RESUME-V2.zh.md 和 EVIDENCE-PIPELINE.zh.md。V2 本轮没有执行应用类型检查或行为测试，以下通过记录仅为历史快照。

## 目标与不可替代的验收范围

同时改善工具调用成功率、任务执行稳定性和实际交付质量，支持长周期任务。
这不是只增加重试次数、放宽完成判定或增加单元测试数量的任务。
现有权限、用户停止、桌面产品边界和副作用审计约束必须保留。
不得通过删除失败样本、自动缩小任务范围、模型自评或将冒烟测试算成统计证明来达标。

当前工作树起点：`1348e4def`；核查时无未提交改动。
初始定向基线：`cargo test -p nomifun-agent-runtime -p nomifun-chat-model-broker --lib`，
73 + 14 项通过。这只证明已有单元测试通过，不代表产品任务成功率。

## 成熟方案调研及适配决策

来源均为第一方资料，查阅日期为 2026-09-25：

1. Temporal：Activity 建议幂等；Workflow 的确定性状态与外部 Activity 分开，失败恢复
   不能把外部副作用当作普通函数任意重复。来源：
   `https://docs.temporal.io/activities`、`https://docs.temporal.io/activity-execution`。
2. LangGraph：持久执行从已存状态恢复，非确定性操作及副作用应成为独立任务；节点重入
   不等于从中断代码行继续，必须设计重放边界。来源：
   `https://docs.langchain.com/oss/python/langgraph/durable-execution`。
3. Anthropic 长任务 harness：上下文窗口之间需要显式进度、可检验产物和增量工作流；
   仅靠一次提示或对话摘要不足以维持长期进度。来源：
   `https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents`。
4. Anthropic 工具设计：工具描述、错误的可修复性及真实使用评测属于 agent 的接口设计，
   不应要求模型反复猜协议。来源：
   `https://www.anthropic.com/engineering/writing-tools-for-agents`。
5. Anthropic agent eval：一次成功不等于稳定成功，需要任务级独立验收和多次运行，
   同时检查轨迹与交付物。来源：
   `https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents`。
6. StepFun 官方 Coding Plan 接入文档确认当前测试模型 `step-3.7-flash` 与专用端点
   `https://api.stepfun.com/step_plan/v1`。来源：
   `https://platform.stepfun.com/docs/zh/guide/stepplan`。
7. HTTP 语义标准允许 `Retry-After` 使用非负秒数或 HTTP 日期。等待时长不能在
   传输层被缩短后仍声称遵守服务端冷却。来源：`https://www.rfc-editor.org/rfc/rfc9110`，10.2.3。

采用这些设计原则，不直接引入需要额外常驻服务的工作流平台替换桌面主链路。
当前已有 SQLite canonical session/event store、Kernel 工具权限和 owner 资源管理；
应在它们上面实现确定性恢复，避免第二套互相竞争的 session/权限/历史数据库。

## 起点架构审计（`1348e4def` 的代码证据，不以旧设计文档为实现证明）

| 环节 | 当前证据 | 问题及改造方向 |
| --- | --- | --- |
| 模型请求 | `nomifun-app/src/router/chat_broker_host.rs::invoke_error_to_chat_error`，`nomifun-chat-model-broker/src/broker.rs::run_broker` | 网络、超时、429、部分 5xx 被标成仅 failover；单路由不能自恢复。Broker 不消费已有 `retry_after_ms`，也无退避。只在尚未提交语义输出时重试；保留单一重试 owner。 |
| 流式协议 | broker decoder、`nomifun-agent-runtime/src/turn.rs::StepState` | 已有 call/result 关联、参数预算及终止事件校验。补充断流、半截 JSON、取消与输出已提交后的禁止盲重试回归。不能将工具调用文本直接执行。 |
| 工具 admission | `nomifun-engine-core/src/kernel.rs`，runtime `tool_dispatch.rs` | 已有权限、事件先写与 attempted 标志。需要统一区分参数错误、拒绝/未执行、可重试读取、执行失败与副作用不确定；不能把它们都当成重新规划的理由。 |
| 计划控制 | runtime `planning.rs`、`requirements.rs`、`completion.rs` | 重复提交无变化计划被算成错误；64 次修订是硬门槛；无变化/拒绝控制也使完成报告失效。改为确定性的幂等状态转换、精确失效条件与结构化修复提示，保留原始输入和需求账本。 |
| 长任务预算 | runtime `turn.rs`、`stream_limits.rs`、`context_lifecycle.rs` | 默认 32/64 次模型调用耗尽后直接失败，计划/调用 ID/输出预算同样有长期积累边界。引入分段执行、可持久化安全点和总预算，而不是无限循环或只调大常数。 |
| 重启恢复 | `nomifun-app/src/router/nomi_core_session.rs::reconcile_orphaned_active_turns` | 当前把失去内存 owner 的 running turn 结算为失败以解除 busy；不是自动恢复。必须增加可验证的持久恢复点和不确定副作用核对。 |
| 日志/存储寿命 | host `engine_journal.rs::append`、session `store.rs::insert_payload_tx` | 单轮普通 journal 上限 3200 条/4 MiB，另有清理保留额度；单 Session stored payload 合计上限 16 MiB。单纯提高 32/64 模型轮数仍会撞上这些边界。需要分段、可回收检查点和独立资源预算。 |
| 用户纠正/取消 | runtime `steering.rs`、host `runtime_steering.rs`、shared lifecycle SDK | 保留 terminal fence、输入 receipt 和取消传播；恢复不得复活已取消任务或使用已过期授权。补充崩溃/取消竞态测试。 |
| 资源清理 | host `engine_process_host.rs`、`engine_process_recovery.rs`、SDK cleanup | 进程启动/轮询/退出及结果不确定性以 owner 为准；不能以模型回复或宿主 future 结束推断清理成功。 |
| 上下文与工具发现 | runtime context/compaction/history/discovery modules | 已有压缩和历史工具；验证长链中需求、最新进度、未解决副作用、调用配对和工具版本不会丢失。 |
| 交付质量 | runtime `report_completion` 与 live smoke | 自述证据关联不是独立语义验证。需要真实产品入口运行、独立产物断言、约束检查及失败分类。 |
| 展示与可观测性 | canonical event journal、UI transcript | 区分恢复中、等待用户、预算暂停、执行失败和成功；不得把自动恢复/跳过调用计成第一次成功。涉及 renderer 时运行 desktop boundary 检查。 |

上表路径均相对于 `crates/backend/`。没有完成的审计项不能以附近测试通过替代。

## 目标执行模型

`接受输入 → 恢复/建立任务状态 → 模型建议 → 确定性校验 → 写入执行意图 → owner 执行 →
写入真实结果 → 保存安全点 → 继续/暂停/独立验收 → 发布终态`

### 一、传输与工具执行

- Broker 是唯一模型传输重试层：错误分类、同路由瞬态恢复、指数退避/抖动、
  Retry-After 最短等待、总尝试上限和取消感知。
- 不允许在已向上层提交模型语义输出之后静默重放同一流；恢复必须显式丢弃未执行
  proposal 并记录新 attempt，不能复制已经执行的工具。
- 工具执行使用稳定 operation/call identity、请求指纹和 owner 结果；先记意图再执行。
- 只读操作可在无结果时按明确策略重试；写文件、启动进程、发消息、浏览器提交、
  MCP 远程调用等副作用不能只因超时重复。先核对结果；无法核对时持久化
  `outcome_unknown` 并暂停相关 effects，读/诊断可继续。
- 参数/计划协议错误返回可修正的字段、允许值、当前状态及下一步，且不使
  无关正确状态失效；不从任意错误文本猜权限或成功。

### 二、确定性控制状态

- 计划状态是模型建议、引擎验证的派生状态，不是权限或完成事实。
- 相同计划/需求重复提交幂等确认：不多写修订、不使证据失效。
- 修订使用足够宽且 checked 的单调计数，不用 64 次协议上限限制长任务。
- 真实状态变化才使对应完成报告失效；失败/无变化控制不能凭空更改执行状态。
- 单独的无进展检测器记录重复 proposal/拒绝，不能靠把幂等调用标为失败抑制循环。
- 原始输入、后续纠正、需求、解释和证据分别持久化。不得靠自动改写用户要求
  或让模型的“已完成”状态代替验收。

### 三、持久长任务与恢复

- 区分 task、turn、segment、model attempt、tool invocation；UI turn 不应成为任务寿命。
- segment 预算到达是安全点切换/显式暂停，不是任务成功，也不应一律是致命失败。
- 恢复点包含绑定版本、事件 cursor、输入/计划版本、上下文引用、未决调用、
  使用量和进度；仅在一个已结算 batch 后推进可继续游标。
- 检查点写入与 canonical 事件一致；错误、旧版本或缺失引用必须拒绝恢复。
- 重启先取得单任务执行 lease/fencing token，核对 owner 的运行进程和不确定效果，
  再从下一个未完成步骤继续；禁止并发双 owner 或重放已成功副作用。
- 取消/权限变更/新输入优先于恢复；等待用户、预算暂停、不确定结果都不是完成，
  必须保留原因和下一步。

### 四、质量与持续评测

- 独立执行最终测试/产物断言；保留不允许跑测试时的未验证状态，不擅自扩权。
- fixtures 不可被 agent 修改验收逻辑；评测在独立副本/进程验证，检查禁止改动。
- 冻结任务集、模型/端点、工具版本、预算及提交身份；结果包含所有运行，失败不能
  因后续修复被覆盖。恢复成功另报，不与首次成功混为一谈。

## 99% 的明确口径

三项指标必须分别报告，不能相乘、平均或只取最高的一项：

1. 工具链成功：合法且在授权范围内的工具意图，在规定预算内得到正确可用结果；
   同时报告 first-attempt、recovered、invalid-proposal、policy-denied、unknown-outcome。
   预期非零测试退出可作为正确观察，但不能当作修复成功；修复后必须重新验收。
2. 执行稳定性：被接受的完整任务在指定故障模型和时长下推进至真实终态，无卡死、
   丢任务、重复副作用或假成功。用户取消单列；崩溃/预算耗尽不能从分母消失。
3. 交付质量：独立验收覆盖用户全部约束和产物；模型的报告只作轨迹数据。

单项独立二项样本在 **299 次零失败** 时，其单侧 95% 精确置信下界才超过 99%
（零失败下界为 `0.05^(1/n)`）。三项同时给出至少 95% 家族置信度，保守使用
Bonferroni `alpha=0.05/3`，零失败至少需 **408 个有效独立样本/项**。
有失败时需精确计算新的区间；重复同一个 mock 单元测试不是独立产品样本。
任务分布、长任务时长和故障注入必须单列，否则不能外推到“所有任务”。

阶段性小样本只用于发现问题；最终必须给出样本不足/不达标，不能写已达 99%。
真实调用先小规模验证，确认错误分类、凭据隔离和实际成本后，再运行明确有界的批量评测。

## 实施与验证追踪

- [x] 当前工作树/主链路检查、第一方方案调研、初始单元基线。
- [ ] 模型同路由瞬态重试、退避/Retry-After、取消与禁止重复语义输出测试。
- [ ] 幂等计划更新、修订生命周期、控制状态精确失效与无进展检测。
- [ ] 工具错误分类、参数修复反馈与不确定副作用核对。
- [ ] segment/checkpoint/lease 的 canonical 持久实现及生产 host 接入。
- [ ] 中断/重启/取消/权限变化/压缩/长过程的故障注入矩阵。
- [ ] StepFun 真实模型通过产品 Session → Runtime → Kernel → 工具链的基线和回归。
- [ ] 独立质量评测、统计门禁、持续 soak，全部指标的 99% 完成审计。

凭据只从既有安全 runner 的环境/标准输入交给 fixture；Cargo、工具子进程、报告和
仓库不得含 key。评测日志需结构化且脱敏。当前文档不声称任何尚未运行的测试通过。

## 已实施的第一组改造

- Broker：原路由瞬态恢复、最多 4 次总尝试/单路由 3 次、带抖动的指数退避、
  可取消的等待；不在已提交语义输出后盲目重放。超过 120 秒请求内等待预算的
  Retry-After 会作为失败返回，不截短后提前重试。HTTP 传输保留秒数和日期形式，包含 503。
- 工具：按本轮实际暴露的冻结 schema 缓存 validator，在所有 model effects 之前
  校验整批参数。任一调用参数错误时整批未执行，反馈 schema 位置/允许形状，
  不回显参数值；禁止 schema 校验触发外部网络或文件读取。
- 计划：重复提交幂等成功、真实变更才失效已有完成报告；单独检测无进展控制循环。
  修订扩展到 checked u32，并同步历史交接。长度校验使用 schema 的 Unicode 字符口径，
  同时保留总字节预算。读取/未执行提议的失败不凭空要求重开有效计划。
- 输入账本：`requirements` 在 schema 中是可选项，不能在执行时又成为隐藏必填项。
  引擎为缺失账本覆盖的已接受输入建立完整范围引用；短引文仅定位，原始输入不截断。
  模型显式提交的错误引文仍被拒绝；已有需求仍不可改写或丢弃。
- 夹具：协作调用等待新 durable link（排除旧链接、保留截止时间）；coding 通过正式
  API 绑定准备好的项目，验证 opaque workspace identity 与文件/进程的共同根目录。
  保持生产 Session 工作区隔离、原始任务、不可改动的验收测试和最终产物断言。

这些不是持久任务分段/自动重启恢复的替代实现；那部分仍待完成。

## 真实测试发现（失败记录保留）

1. 初次 `--model-smoke` 失败于 `CLUSTER_LEAD_LINK_MISSING`。原夹具在异步 turn
   admission 后立即要求链接存在；已改为有界等待，补充迟到/旧链接/缺失链接测试。
2. 初次执行到模型的 `--long-coding-smoke`：25 个模型步骤、23 次压缩调用，
   0 次源码写入，最终 provider gateway failure，并有 `PLAN:REQUIREMENT_COVERAGE`。
   后续代码核查确认该夹具仍将项目放在公共 work root，但 2026-09-24 的
   `416e47e16` 已将默认 Session 隔离到独立子目录。这是无效的工作区绑定，
   不能把该样本当成产品质量统计样本；保留诊断，不改写为通过。
3. 等待链接后的模型复测仍有 `CLUSTER_LEAD_LINK_DEADLINE_EXCEEDED`，尚未证明
   协作链稳定。失败时增加固定类别、计数及脱敏终态诊断，继续定位。
4. 显式绑定项目后的 coding 首次复测在模型执行前触发
   `SESSION_RESOURCE_SELECTION_MISMATCH`：正式 API 将默认 workspace selector
   转换为 `selected-workspace-<hash>`。夹具改为验证目录、资源哈希、binding ID 和
   process cwd；未放宽资源权限或只接受任意 opaque ID。

5. 正确绑定 workspace 后，`--coding-smoke` 一轮通过：官方 coding Agent 完成
   1 次写入、1 次回读和最终回复，独立文件断言通过。它只是一个有效冒烟样本。
6. 协作最新复测仍失败：25 个模型步骤中 23 次参数预检拒绝，0 次 delegation
   admission；固定形状诊断显示 `tasks` 缺失或为 string，`synthesize` 为 string。
   union/oneOf 反馈已展开为候选分支的字段提示，但该问题尚未解决。
7. 修正 workspace 的长任务复测读取到了真实源码/测试并执行了失败基线，随后因
   计划 schema 错误和以文字输出的伪工具调用而中止（9 个模型步骤，未写源码）。
   运行时没有将这些文字当成可执行工具，安全边界保留；下一步需要原生格式纠正和
   对参数类型产生环节的定位，不能用任意 XML/字符串执行作为兜底。
8. 新增不执行工具的原始 schema 探针，对比平面与联合 schema；独立 Bun HTTP
   客户端在非流式和流式请求上都得到 403，未获得可用形状样本。因此不能据此把
   参数字符串化归因给供应商；需使用与正式应用相同的传输环境继续对照。

上述失败及尚未完成的重测都不支持任何 99% 结论。

## 已运行的定向验证

| 检查 | 当前结果 |
| --- | --- |
| `cargo test -p nomifun-agent-runtime --lib` | 89 通过 |
| broker `--lib --test conformance` | 17 + 32 通过 |
| contracts `--lib` | 68 通过 |
| app `--lib chat_broker_host::tests` | 15 通过 |
| model-invoke `--lib retry_after` | 2 通过 |
| Wave 5 `--lib delegate_schema_accepts_one_parallel_fanout_with_optional_synthesis` | 1 通过，含精确单任务/false 形状 |
| app `--test nomi_core_live_provider_smoke`（无凭据） | 11 通过；7 个 opt-in live 测试跳过，不能算实测通过 |
| `node --test scripts/validation/agent-reliability-report.test.mjs` | 10 通过，全部为门禁单元测试的合成数据 |
| 原始 schema 探针离线解析测试 | 2 通过；没有修复或强制转换原始参数 |
| credential-isolated runner `--self-test` | 通过 |
| `check:agent-vocabulary`、`check:desktop-ui-boundary`、`help --check` | 通过 |
| `check:uarc-boundary` | 后台扫描完成、无 violation；报告仍有 1 项既有 macOS gap，非全平台完成声明 |
| 真实 `--coding-smoke` | 一轮通过 |
| 真实协作和长 coding | 仍失败，见上节；不发布可靠性百分比 |

前台扫描/编译曾因工具调用超时未返回完整结果；最终结果以上述重新捕获的完整输出为准。
尚未运行全仓库、macOS/Linux、真实多小时重启恢复和统计规模产品评测。

## 第二阶段实现与验证（2026-09-25）

本阶段继续保持整体目标未完成，具体推进如下：

1. `nomifun-engine-core/src/tool_schema.rs` 为简单、闭合的对象联合 schema 补齐根层
   `properties`、公共 `required` 和字段类型，保留原始 `oneOf`。带有限制性根约束、
   引用作用域或 pattern properties 的复杂情形不擅自改写。枚举边界输入验证改写前后
   的合法参数集合一致；没有把字符串强制转换成数组或布尔值。
2. `protocol_recovery.rs` 与 `ModelResponseRejected` 记录支持有界原生格式纠错：
   拒绝的整批提议（包括其中已形成完整 JSON 的调用）不执行；先记录丢弃事实，再用新
   模型步骤纠正。连续最多 2 次、单轮最多 8 次。历史重放不能用该事件隐藏已 admission
   的工具，取消/保存失败不会开启下一次模型调用。仍然不解析、执行伪 XML。
3. `checkpoint.rs` 捕获的是派生进度和引用，不是模型内存转储。原始请求、原始工具输出、
   provider-private reasoning、provider credential 和活动进程句柄不被复制进快照。
   计划文本仍遵循既有任务账本的数据策略。用户纠正 receipt 保留实际应用顺序，不能按 ID
   字典序重排，否则其 input 索引会改变含义。
4. `agent_turns` 新增 latest native checkpoint 字段；forward migration 005 将 schema
   migration head 升至 4，未修改旧迁移。快照与 journal 元数据在同一事务中提交，
   校验 owner、Snapshot、active generation、CAS revision 和 checkpoint writer fence。
   快照上限 256 KiB，原地替换，不叠加到 immutable payload 的 16 MiB 预算中。
5. 正式 Runtime → Host → Journal → Agent Store 已接入静止边界保存。仍有未结算
   effects 时会推迟快照而不是伪称保存成功；完成/取消释放快照，失败保留进度作为数据。
   失败的 Turn 不会因此被重新打开，加载快照也不授予恢复/执行权限。

本阶段验证：runtime 97 项、Session Store 54 项、engine-core 22 项通过（另 1 项
opt-in process 测试未跑）；DB ID/schema 6 项通过，前向迁移/保留数据检查通过；
正式 Journal checkpoint 测试及完整 Session → Runtime → Host 的离线产品链路测试通过。
这些仍不是 99% 的统计样本。

真实模型复验：通过正式应用的首个模型请求返回 HTTP 403 权限拒绝，没有发生工具调用。
已停止使用该凭据重试，不能用这次运行验证 schema 展示改造或协议纠错的模型成功率。

明确未完成：

- `reconcile_orphaned_active_turns` 尚未接入自动恢复调度；快照保存/加载不等于自动续跑。
- `execution_fence` 当前只校验 checkpoint writer，尚未覆盖旧模型/工具 producer 的所有
  admission，不能声称已解决恢复接管时的双执行者问题。
- 恢复时还需核对检查点后的事件、待处理用户输入、未知 effects 和当前工作区，
  重建模型上下文并使历史完成证据失效；不能直接重放旧工具。
- 执行分段、原有 32/64 模型步骤与 journal 3200 条/4 MiB 的联合预算处理仍待完成。
- 真实多小时、崩溃/恢复矩阵、独立交付验收与大样本统计门禁尚未完成。

后续优先推进上述恢复接管与分段链路；真实模型验证需在调用权限恢复后继续。

## 统计门禁使用契约

实现：`scripts/validation/agent-reliability-report.mjs`。
命令：`bun run check:agent-reliability --input evidence.json`。
退出码：0=提供的固定样本通过统计门禁，1=尚未证明，2=证据格式/来源声明无效。

输入包含：

- `schema_version: 1`、`suite_id`、唯一 `runtime_build_digest`、`model: step-3.7-flash`。
- `strata`：预先固定的场景 ID、`minimum_samples`、`minimum_duration_ms`、
  适用的 `metrics`（tools/execution/quality）以及每项的独立验收 `checks` 名称。
- `scheduled_trials`：执行前保存的 `{id,stratum}` 清单；丢失的运行仍在分母内，且阻止通过。
- `samples`：`id`、唯一独立任务 `session_id`、相同 suite/build/model、`stratum`、
  `duration_ms`、`source: live_product`、`grader: independent_assertions`、
  与清单完全一致的 `checks[metric][check]`。true=独立验证通过，false=失败，null=未验证。

不可将同一 Session 的多个 turn 当成独立任务，不可混合不同构建，不可删除失败后只选成功
样本。空数据、少量全成功、缺少场景、长任务时长不足或缺少验收都不能给出通过。
二项置信界要求适当的独立同分布任务抽样；不能把任意固定难度/重复任务序列直接当成该假设。
来源字段不是数字签名或独立性证明；最终审计仍必须核对预先冻结的清单、原始 canonical
事件和真实产物。当前 live fixture 的诊断日志还不是这个标准化评测数据集，证据采集器、
长期运行/重启矩阵及最终批量验收尚待接入。门禁单元测试的合成数据绝不能冒充实测样本。
