# 多 Agent 生命周期可靠性与用户控制：最终实施方案

日期：2026-10-18；2026-10-19 启动实施。状态：代码实施及当前 Windows 可执行验收完成，macOS／Linux 原生待验；分项证据见第 10 节。

审计基线：249082fe78caf771116f11f4206ac0595c22b61c（0.8.14）。执行者必须核对实际 HEAD、工作区改动和现役实现，不能把下面的基线行号当作永远不变的证据。

本文件是本专项唯一现役施工与验收说明，整合用户要求及最终复审，取代此前对话中的草案。源码、正式类型、canonical 事实和实测结果共同决定实现事实；本文件区分目标与现状，不替代现行安全合同。不宣称未经比较与实测的“全局最优”。

启动入口见[干净会话启动 prompt](agent-lifecycle-reliability.start-prompt.zh.md)。执行前阅读根目录 [AGENTS.md](../../AGENTS.md)、[Agent Session](../architecture/agent-session.zh.md)、[AgentExecution](../architecture/agent-execution.zh.md)、[伙伴架构](../architecture/companion.zh.md)和[浏览器架构](../architecture/browser-platform.zh.md)。涉及全局文件、进程或 Computer 时同时阅读[全局 OS 能力](../architecture/global-os-capabilities.zh.md)。

## 1. 要解决的问题与交付边界

解决伙伴及其他多 Agent 场景中的 subagent 长期停留运行／等待、无法准确停止、局部控制故障扩散、重启恢复冲突、共享能力无关阻塞、完成对象无人退休以及清理饥饿。

最终交付必须同时满足：

1. 所有 Agent 执行实例可发现、可由用户直接控制停止、可查看实际执行与清理进度，包括 root、subagent、伙伴委派、自动任务、恢复／异常／清理中的任务。
2. 独立停止一个任务不默认取消整个 Execution；明确选择整个协作时维持整组取消语义。
3. 恢复继续原任务，不把状态未知、缓存缺失或暂停变成隐式从头执行。
4. 慢 Stop／Steer、某个清理失败或某个浏览器 target 的慢操作，不拖住无关健康任务。
5. Turn 终结、Runtime 退休、Session 后台资源释放分别取得真实依据；正常授权的后台工作不被回收误杀。
6. 相同工作负载反复创建并完成后，废弃运行实例、任务、订阅、句柄和缓存不持续累积。合法持续工作的资源不要求归零。
7. 维护路径减少、职责收敛；不以禁工具、降低并发、缩短任务寿命或添加操作门槛换取通过。
8. 不依赖用户关闭已完成会话：使命已完成且无剩余使用责任的托管重资源自动退休；NomiFun 真正退出时回收其持有的全部应用生命周期资源。保留历史会话不等于保留重型运行载体。

本专项允许在现有 owner、合同、UI、测试与文档中做完整必要调整，不等于只改按钮或补几个 timeout。它不授权建设通用资源治理平台，也不授权改写现役数据、清理真实用户数据目录或发布上线。

## 2. 用户要求与执行偏好

### 2.1 对话明确的产品要求：不可被实现便利性覆盖

| 编号 | 要求 | 执行约束 |
| --- | --- | --- |
| U01 | 全面追到根因，不能打补丁 | 修复跨层责任和真实故障路径，替换旧的错误裁决；不能只修改显示状态、增加重试或掩盖异常。 |
| U02 | 架构简单、强大、易维护 | 复用现役 owner、TaskTracker、进程 Supervisor、outbox 和事实链；不新增平台级 Broker、万能生命周期服务、总状态机或第二账本。 |
| U03 | 能力不下降，体验不损失且提升 | 保留工具、权限、模型选择、任意 shell、共享目录、长任务、后台执行及已有明确配置；不以支持全部控制动作作为 Agent 的运行资格。 |
| U04 | 不因资源压力误杀合法任务 | 不新增按 CPU、内存、空闲、等待时长、数量或深度自动 kill；不新增固定整机配额和统一硬 TTL。已有用户设置及工具真实期限保留。 |
| U05 | 协作保持灵活 | 不强制 worktree、读写路径声明、只读降级或新的资源选择步骤。不能假装任意 shell 的副作用可被完整推断。 |
| U06 | 已授权委派不被父回合误停 | 父 Turn 正常结束、模型返回、前台页面关闭不能成为取消后台委派的新增理由；用户明确选择的取消范围才生效。 |
| U07 | 缓存不能使任务重新开始或循环 | 缓存层没有启动、恢复、重试、新建 Attempt、发送原始输入或执行工具的权限；加载失败不 fallback 到重跑。 |
| U08 | 不丢失活任务现场 | 等待、暂停、恢复及未知效果中的不可重建现场必须保留；不能仅凭 Finished、空闲、checkpoint 或 active count 决定回收。 |
| U09 | 不能完全信任 Agent 自己管理生命周期 | 每个 Agent 实例都有宿主侧用户停止入口，控制不经过模型，不等待 Agent 同意或在途 Steer 的业务响应；异常态也不能隐藏或失去操作入口。 |
| U10 | 停止不能误伤其他任务，也不能自动复活 | 精确目标、持久作用范围和迟到事件围栏必须完整；独立兄弟保留，已停止范围不能被恢复、重试或派生绕过。 |
| U11 | 继续与重新开始严格分开 | 不能用 cancel＋retry 假装暂停／继续；原进度、效果与计数不因缓存或恢复失败重置。用户现有显式预算调整与合法重试策略仍保留。 |
| U12 | 不增加面向用户的工程门槛 | 用户看到清晰的任务、等待原因、停止与清理状态；内部代际、owner、checkpoint 校验不变成普通工作前的额外配置或确认流程。 |
| U13 | 用户不手动关闭也必须自动回收重资源 | 完成使命的任务／会话不因仍在列表就永久保留浏览器、进程、连接等；真正退出应用必须收束所有应用自有重资源，异常退出有原生父死兜底。保护活任务不是无限保留废弃资源的理由。 |

这些是本专项强制产品约束。不得通过删除测试、调低负载、降低并发、关掉能力、增加默认超时或人工确认来取得表面正确。

### 2.2 本专项的执行约定

以下用于可靠地完成本任务，不应伪称为用户对所有未来任务的全局偏好：

- 使用启动 prompt 后即进入实施，持续完成必要代码、测试、清理和文档。不能反复审查计划代替实施，也不能只输出建议后结束。
- 常规实现选择自主解决；只有必须改变上述产品合同、需要破坏性／外部发布动作，或缺少无法推断的关键事实时才提出具体问题。说明依据，同时继续不受阻的工作。
- 先保护和识别无关改动；不覆盖、回滚、格式化或清理不属于本专项的工作。
- 不使用真实用户会话、已登录浏览器、生产数据根或凭据做破坏性故障实验。使用任务专用临时数据、mock provider、受控本地进程和浏览器测试实例。
- 保持可审阅差异。默认不自动提交、推送、合并、打包安装或发布；后续若用户明确授权，再按根 AGENTS.md 的身份与 Git 规则操作。
- 定期说明已证实的变化、当前不确定点和下一步；更新本文件第 10 节的单份进展。上下文压缩或换会话后从已完成位置继续。
- 不把源码、模拟测试、实机测试和生产用户故障复现混为一谈。未运行、平台不可用、测试被过滤为空，都必须如实记录。

## 3. 现状基线：实施者需复核的证据

下面是源码审计依据，不是已交付修复。CPU／RSS 实际上涨的归因仍需指标与实测；running／waiting 持久状态不证明旧 OS 进程仍活着。

| ID | 基线事实与风险 | 实现入口与基线位置 |
| --- | --- | --- |
| R01 | canonical Attempt cancelled 会将整个 Execution 取消，并终止其他孩子；单任务按钮不能直接复用此语义。 | [scheduler](../../crates/backend/nomifun-agent-execution/src/scheduler.rs) 2073；[repository](../../crates/backend/nomifun-db/src/repository/sqlite_agent_execution.rs) 4427；现有 canonical cancellation 测试也锁定该行为。 |
| R02 | Execution pause 会撤销调度、reconcile 运行 Attempt 并触发 cleanup；没有可信终态时可能进入 review_blocked，resume 明确拒绝。 | [engine](../../crates/backend/nomifun-agent-execution/src/engine.rs) 1382、1442；[repository](../../crates/backend/nomifun-db/src/repository/sqlite_agent_execution.rs) 1867、1898。 |
| R03 | Session 异步恢复原 Turn 与 Execution 恢复并行；后者可能先退休关联、停车并清理原 Turn。两类租约分别为 90s、30s。 | [state](../../crates/backend/nomifun-app/src/router/state.rs) 310、633；[native recovery](../../crates/backend/nomifun-app/src/router/native_turn_recovery.rs) 8；[scheduler](../../crates/backend/nomifun-agent-execution/src/scheduler.rs) 1458。 |
| R04 | Stop／Steer 在主循环 inline await，会阻止兄弟 jobs 被 poll；错误还可能退出整代并触发兄弟的中断恢复。 | [scheduler](../../crates/backend/nomifun-agent-execution/src/scheduler.rs) 996、1046、1339、1414、2685。 |
| R05 | Waiting／Paused 会延长观察 deadline 并每 500ms 查询；Attempt 仍占现有并发和工作区预约。 | [attempt runner](../../crates/backend/nomifun-agent-execution/src/attempt_runner.rs) 317；[scheduler](../../crates/backend/nomifun-agent-execution/src/scheduler.rs) 2973、3016、3050。 |
| R06 | Full／ReadShell 不是只读沙箱，任意命令与后台写者无法靠当前工具状态完整推断，因此共享工作区现有保守顺序有明确原因。 | [scheduler](../../crates/backend/nomifun-agent-execution/src/scheduler.rs) 3021；[全局 OS 能力](../architecture/global-os-capabilities.zh.md)。 |
| R07 | Attached Chrome provider 的锁跨整次命令执行；disconnect、close、shutdown 也先取该锁。managed Browser 则已有按 Session 的隔离，不能混为全局锁。 | [attached provider](../../crates/backend/nomifun-app/src/browser_workspace_provider/attached_provider.rs) 261、359、436、690。 |
| R08 | Wave2 Host Context 未携带 Kernel 的 control；Computer 等待物理锁及 native port 没有完整取消传递。普通 wait 有 5s 上限；永久 native hang 仅为结构风险。 | [Wave2 contract](../../crates/backend/nomifun-agent-domain-wave2/src/lib.rs) 358、1918；[role host](../../crates/backend/nomifun-app/src/router/agent_role_host.rs) 373、678；[Computer](../../crates/agent/nomi-computer/src/tool.rs) 24。 |
| R09 | 固定最旧 100 项的 cleanup 查询会被持续失败项占住。此前原 SQL 内存复核连续三轮取不到第 101 项；执行阶段须固化为仓库回归测试。 | [scheduler](../../crates/backend/nomifun-agent-execution/src/scheduler.rs) 442、584；[repository](../../crates/backend/nomifun-db/src/repository/sqlite_agent_execution.rs) 4550、4573。 |
| R10 | cleanup 的 cancel_for_execution 在无活动 Turn 时可直接返回成功；释放 Turn admission 不等于移除 Runtime。UserCancelled 又明确保留 Session 进程。 | [session owner](../../crates/backend/nomifun-app/src/router/nomi_core_session.rs) 4918、4942；[runtime registry](../../crates/backend/nomifun-ai-agent/src/runtime_sessions.rs) 1475；[runtime host](../../crates/backend/nomifun-app/src/router/unified_runtime_host.rs) 44、1667。 |
| R11 | Session teardown 会 close 知识回写并 cancel_all；主 Turn 已终结不证明回写已结束。进程 owner 已可独立于 Runtime transport 存续。 | [kernel session](../../crates/backend/nomifun-app/src/router/engine_kernel_session.rs) 1487、1499；[writeback](../../crates/backend/nomifun-app/src/router/engine_knowledge_writeback.rs) 47；[Wave2 owner](../../crates/backend/nomifun-app/src/router/nomi_core_wave2.rs) 498。 |
| R12 | checkpoint 不保存 live handles；构造异常在资源归属未证明时关闭 registry 是安全围栏，不能为可用性直接删除。 | [checkpoint](../../crates/backend/nomifun-agent-runtime/src/checkpoint.rs) 1、91；[engine recovery](../../crates/backend/nomifun-app/src/router/engine_recovery.rs) 14、50；[acquisition](../../crates/backend/nomifun-ai-agent/src/runtime_session_acquisition.rs) 20。 |
| R13 | Attempt 再委派是在同一 Execution 追加 DAG Steps，依赖边不等于委派归属。既有事件记录 caller 与 added_step_ids。 | [engine](../../crates/backend/nomifun-agent-execution/src/engine.rs) 954、985；[repository](../../crates/backend/nomifun-db/src/repository/sqlite_agent_execution.rs) 2810、2909。 |
| R14 | Browser workspace 主要在显式关闭、确认重建、Session 删除及 app shutdown 时释放；没有任务完成自动退休闭环。正常退出已有 browser shutdown，但其他领域前置失败可能跳过浏览器清理。 | [Browser service](../../crates/backend/nomifun-browser-platform/src/workspace.rs) 817、929、969；[AppServices](../../crates/backend/nomifun-app/src/services.rs) 1208、1427。 |

基线中已经有效的 MCP cancellation、process Supervisor、target claim、incarnation、Session 归属、效果回执与 checkpoint 校验必须保留。未证实的问题不能写成必现 bug，也不能以旧行号为由重做后续已经完成的修复。

## 4. 最终架构与行为合同

### 4.1 保留三个现有职责，不合并事实域

| 现有职责 | 保留的权威 | 本次修正 |
| --- | --- | --- |
| Canonical Session owner | 精确 Turn 的输入、执行代际、暂停／恢复、取消与终态／效果事实 | 对原 Turn 恢复，暴露可观察的恢复事实；管理指令精确作用且不经过模型。 |
| Execution engine／scheduler | DAG、Step／Attempt、策略、租约、预约和业务结算 | 识别原 Turn 正在恢复，跟随同一 receipt；维持本领域结算，不把控制元数据当任务成果。 |
| 具体资源 owner | 原生操作、句柄、真实互斥、取消能力与清理证明 | 缩小无关锁范围、贯穿控制，准确退休自己的对象。 |

统一入口和语义不等于万能生命周期服务。AutoWork、Cron、伙伴、Remote 等仍走现有正确领域 owner；不能直接按 Session 全部取消绕过领域事务。Session 与 Execution 的状态机均保留，修复的是交接，不是建立总状态机。

### 4.2 用户控制目标与作用范围

控制目标必须来自已验证的当前事实，根据对象绑定 Session、Operation／Turn、Step／Attempt、execution generation 及必要的 expected version；不要求每种对象虚构不存在的 Attempt。重复命令幂等，旧命令不得临时选择“当前 Turn”。

| 产品操作 | 确切语义 |
| --- | --- |
| 停止当前回复 | 保留原有单 Turn 语义，不暗中升级成关闭整份 Session 及其所有后台进程。 |
| 停止此任务 | 通过明确的领域取消意图停止所选 Step／任务，结算不能默认提升为 Execution cancelled。 |
| 停止该任务委派的工作 | 使用可验证委派归属，不沿 DAG 依赖边猜所有权；并发派生受同一持久范围围栏约束。 |
| 停止整个协作 | 维持 Execution 整体取消；覆盖其未完成工作，迟到结果不能复活聚合。 |
| 全部停止 | 对当前用户范围、明确截止点已接受的活动工作进行领域命令聚合；列出处理中／未确认项，不假装跨领域原子事务。 |
| 停止本次自动运行／停用后续触发 | 分别处理本次实例及未来计划，不能把下一次合法计划触发误称为复活，也不能擅自删除计划。 |
| 结束任务的后台资源 | 根据原创建者、资源 owner 和代际处理仍归该任务持有的资源，不按 Session 粗暴清掉后继或独立服务。 |
| 强制结束 | 仅用户明确触发，仅作用于可证明归属且能独立终止的本地 worker／进程等；不杀用户 Chrome、共享 host 或健康兄弟。 |
| 重试清理 | 重试原对象的释放和结算，不重跑业务。 |
| 重新开始 | 显式新执行，不冒充继续，不由缓存／恢复失败触发。 |

单任务、委派范围、Execution 的取消意图需持久存在于现有领域命令／canonical 控制事实中，投递和结算共同消费。不是新增独立控制账本。已有数据归属不足时做最小正式事实补充，不通过名字、UI 层级或目录猜测。

停止范围内：禁止新增后续派发、重试与委派绕过；停止与派生的竞争由同一适用事务边界处理。与该范围无关的工作继续。确实依赖被取消产物的任务可以等待用户调整／替换，不能伪造依赖完成。

所有实例必须始终有可发现的宿主停止路径，异常、恢复和清理中的实例也不例外。UI 统一展示“请求已接收／执行停止已确认／资源释放已确认”；不能仅凭 cancelled 字段隐藏资源状态。持久写入失败不能显示“停止已持久确认”。

### 4.3 暂停与继续：必须真实，不新增执行资格

- 暂停调度：关闭后续派发，已开始的工作由原 owner 监督、结算；不关闭 Runtime 来冒充无损暂停。
- 暂停原 Turn：请求 native owner 到达真实安全边界并保留可恢复进度。不得复用当前会把任务改为 review 并清理取消的路径假扮此行为。
- 已请求但尚有不可立即中断操作时，显示“正在暂停／操作仍在结束”，保留停止入口。不能把 Rust Future 被 abort 当作 OS 操作已经结束。
- 继续使用原任务、适用的原 Turn／checkpoint、原效果和进度；遵守现有 build、Snapshot、cleanup、unknown effect 校验。缺少可恢复依据时说明原因，不自动重跑。
- 缓存或普通恢复不能重置预算／进度；用户通过现役明确恢复合同调整预算等能力仍保留。
- “不支持立即冻结／强制结束所有操作”不能成为拒绝该 Agent 正常工作的理由。所有 Agent 的可控制性底线是可发现、可提交精确停止、可查看收束结果；暂停／继续能力按真实阶段和 owner 支持呈现。

### 4.4 等待、预约与工作区

逻辑任务存续、当前可运行预约、实际资源占用是不同事实的派生观察，不另存一套运行真相。

1. 优先移除 500ms 全量 history hydration。用现有 typed receipt／canonical cursor 与 owner 通知等待；订阅后复读，断线补读，通知丢失时低成本兜底，不因丢通知永久挂住。
2. 通知仅唤醒，最终状态以 canonical receipt 为准。不能让 UI WebSocket 成为执行唯一唤醒来源。
3. 只有真实静止且完成交接时才可让出运行预约。唤醒、native resume、问题续执行、重启恢复等全部适用路径须重新获得原 Execution／Participant 的现有预约。
4. 唤醒排队继续原任务，不能因为暂时没有预约而失败、kill、新建 Attempt 或反复暂停。保留用户原有 max_parallel、participant concurrency 等设置，不新增整机限额。
5. 预约重新准入闭环和并发测试未完成前，禁止只修改 active count 排除 Waiting／Paused。相应目标不能标为已完成或悄悄降级成“不做”。
6. 不按“现在没工具调用”推断任意 shell 没有后台写者。保持共享目录和原有 shell 权限，不要求预声明文件 footprint；不得将 Full 或 ReadShell 自动降级。
7. 修复错误阻塞不等于宣称任意共享写者可安全并行。保留现有必要一致性策略；对可证明独立的 target／Session 提升并行，完整解决共享写并发需要另有明确一致性方案，不能混入本次生命周期重构。

### 4.5 控制投递与恢复交接

- 同 Attempt 的常规业务控制投递保持因果顺序、一个在途，使用原稳定幂等键、精确目标和版本确认；跨 Attempt 独立推进。
- 用户 Stop 的持久取消围栏及宿主取消信号直接到原 owner，必须能够抢占在途／排队的 Steer 或其他控制，不等待其业务响应或投递完成。它不能被实现成普通 FIFO 队尾命令；复用现有取消事实与 token，不新增控制中心。
- 抢占不代表旧操作已经结束。原 owner 继续收束在途操作并保存真实结果，迟到 Steer／控制回执只能结算原操作，不能覆盖取消、清掉后续意图或重开执行；不能靠 drop Future 伪造停止。
- 使用现有 tracked control jobs，与工作完成、取消、租约事件同时推进。不能无序 spawn 全部效果，也不能让一个控制错误通过 result? 退出整份调度集合。
- 原意图成功确认前不丢弃；失败只保留该项待处理，不能重发业务 Turn。取消请求不意味着清理已经完成。
- Session 恢复原 Turn；Execution 区分有效原租约、正在恢复、已有终态和确需对账，不在恢复途中退休 link 或触发旧 cleanup。
- 人工停止优先于继续恢复。已有可信终态必须继续经过原 typed output 与业务结算，不能把“native 已结束”当交付成功。
- 不靠增加启动 sleep 或放宽 checkpoint 校验修竞态，不复制输入、不导入旧代 Agent 数据。

### 4.6 Browser／Computer 等资源

- Attached Chrome 短锁内完成 connection incarnation、关闭准入检查和操作归属注册，锁外执行原生调用；disconnect 先关准入，再取消已登记操作并等待原 owner。
- 沿用现有 run／target claims 和 settle／finish，不新增 broker，不顺带将同一个 run 的所有操作无序并发。
- managed Browser 已有 Session 隔离，应保持；失败 release 的责任由现有 owner 继续持有，不能删除证明或要求模型再发一轮才能尝试收敛。
- Wave2／Computer 贯穿现有取消控制。队列取消应及时退出；已经进入 OS 的操作如不能立即中断，显示真实收束状态，不假称可回滚。
- 同一物理桌面必要输入串行保留，缩短无关状态锁持有；管理控制不排在业务长锁后面。
- MCP、Terminal 和 Process 已有有效 owner／Supervisor 能力优先复用。不得为了统一强杀按钮让所有工具改为独立进程，或改变正常工具使用步骤。

### 4.7 Runtime 退休、后台资源和缓存

必须分别确认：Turn 已终结、exact Runtime 已退休、Session 仍合法持有／已释放哪些后台资源。现有 cleanup ack、Finished、active count=0、保存过 checkpoint 都不足以同时证明三者。

正常完成的临时 Attempt：由原 owner 触发退休检查；防止后继准入竞争；待属于该载体的后续责任自然结算后，执行正确范围的 teardown 并注销无用句柄／缓存。不能只 cancel 不存在的 active Turn 就宣布全部释放。

知识回写、文件发布、未提交结算等合法责任不能因主 Turn 已成功而自动 cancel_all。主任务成功可以先正常展示，后台责任单独可见并自然收束；用户明确停止／清空／删除时沿既有适用合同取消和 join。

保留 Session process owner 不等于永久保留整个 Runtime transport。沿用已经证明安全的闭轮重建／soft recycle；不得误停进程、丢句柄后靠 PID 或重启命令补回现场。

| 对象 | 处理规则 |
| --- | --- |
| 纯派生、可重读数据缓存 | 可回收；重新加载只有读取／反序列化权限。 |
| 已退休、无后续责任的对象 | 精确释放并删除 registry／Host 缓存等引用，保留 canonical 历史。 |
| 活动／等待／暂停 Runtime 的不可重建现场 | 本次不新增 idle／LRU／内存压力自动淘汰策略。 |
| 独立 Session owner 已承接的合法后台工作 | 保持原责任与控制入口；不以其存在为由永久 pin 所有上层对象。 |
| checkpoint、输入、效果、暂停／取消和委派归属 | 不是缓存，不删、不回退、不重新解释。 |

缓存加载前后任务身份、cursor、工具／模型调用计数、重试、预算和控制事实不因缓存变化。加载失败保留原任务，明确错误；缓存模块不能调用 send、resume、retry、start 或业务进程启动。

### 4.8 清理公平性、异常与数据安全

- 优先在现有单个 cleanup worker 使用稳定 keyset cursor 遍历待处理项，失败项进入后续轮次；保留已有幂等、single-flight、精确校验与合理重试节奏。
- 不能每次失败都从最旧 100 项开始。动态新增项在有界后续轮次获得机会，不能扫描到空之前一直占满调度线程。
- 不预建通用持久 claim／重试基础设施。只有现有字段无法表达必要事实，或确实存在跨进程调度需求时，说明最小 schema 扩展理由并沿现有表演进。
- cleanup 调用者等待超时不等于真实操作已退出；owner 保留清理任务及结果证明，不能用 timeout 或 drop Future 假释放。
- 普通失败及可证明归属的异常按 exact instance 隔离。构造资源尚无法归属时，先补原 owner 登记闭环，再缩小围栏，不能直接删除 uncertain_acquisition 保护。
- 数据库 I/O 失败、现役 schema／数据损坏、效果未知仍遵守根 AGENTS.md 的 fail-closed 合同。不能把“其他 Agent 继续”扩写成对共享基础设施损坏的无条件可用承诺。

### 4.9 重型资源的自动退休与应用退出：不依赖用户关会话

这是用户明确补充的强制合同。禁止把“用户没有关会话”“以后可能继续问”“Profile 要保留”作为浏览器／进程／连接永久常驻的理由。也不能把保护合法后台工作的规则扩大成所有重资源不回收。

#### 生命周期触发

| 触发 | 自动行为 | 必须保留的边界 |
| --- | --- | --- |
| 任务／委派使命完成 | 原 owner 在原生操作与后置责任结算后自动退休任务专用重资源，不等待用户关闭 Session 或 Agent 发 close。 | 任务完成来自正式领域结果，不从一句模型 final、一个工具完成或面板隐藏猜测。 |
| 后台下载、回写或阶段性进程自然完成 | 完成事件再次触发退休，删除无用引用和句柄；主任务结束时暂留不等于永久留存。 | 当前确有工作的责任保留；不能为回收截断写入、下载或服务。 |
| 旧 Session 不再承载任务与用户使用责任 | 持久历史保留，已无责任的重型载体自动释放；重新使用按正常准入懒创建。 | “仍在会话列表”不是资源引用；不能重放原输入、原网页动作或后台命令来恢复载体。 |
| 关闭主窗到托盘／WebUI 断开 | 不改变活任务授权；已完成资源的自动退休仍照常进行。 | 这不是真正退出，不能杀合法后台工作，也不能解释为保留所有旧重资源。 |
| 明确 Quit／应用重启前退出 | 关闭各域准入，向全部应用自有资源发送停止／关闭并分别收束；包括隐藏面板、旧 Session、已终态但尚驻留的资源。 | 应用退出的范围大于停止一个 Turn；普通 Session 后台标记不能使应用私有资源逃逸退出。 |
| crash／OS 强制终止宿主 | 复用原 Job、watchdog、parent-death、连接断开与原生销毁机制，精确收束独占子资源；重启仅补偿核对尚不确定的清理。 | 不能依赖已死亡进程执行 finally／Future，也不能只寄望下次启动才处理活着的自有子进程；不扫杀用户共享进程。 |

#### 按真实归属释放，不新增用户配置

| 资源 | 默认退休与退出规则 |
| --- | --- |
| 任务专用 browser view／context、headless、临时渲染实例 | 任务完成且责任结清自动释放；真正 app 退出全部收束。 |
| 用户在 NomiFun 内手工使用的 BrowserWorkspace | 属于应用托管原生载体，真正 app 退出必须关闭；运行中自动退休须先区分实际使用／待保留编辑现场，不能以一次 Turn 结束直接销毁。 |
| 用户原有 Attached Chrome／既有网页 | 释放 NomiFun 的连接、claims、overrides 与可证明独占的临时辅助资源；不关闭用户 Chrome 或原有页面。 |
| 在用户 Chrome 中新建的临时网页 | 仅对有真实创建归属、仍归本任务持有且未转为用户工作区的页面自动关闭；不能按 URL、标题或整个窗口推断所有权。 |
| 进程／PTY、MCP 子进程、SSH／采集连接、Computer worker、watcher、订阅及其他现役重型实例 | 按原 owner 与实际用途在任务责任结束后自动收束；应用退出时释放应用持有部分，不因 Session 记录保留而常驻。 |
| 现役支持的独立系统服务／外部持续作业 | 只有既有明确语义及真实独立 owner 已承接才可跨 app 退出；普通 Agent 说“后台运行”不足以形成例外。关闭本应用控制连接不等于删除已安装服务。 |
| 历史、用户文件、已交付产物、持久 Profile／登录资料 | 保留数据；释放原生进程／view 不等于删除资料。不能承诺完整保存任意网页 JS、未提交表单或未完成动作。 |

实施前核对现行 Browser 合同：Agent 与用户使用同一个 BrowserWorkspace，隐藏面板仅 detach；现行 close_idle 是明确确认重建入口，并不是任务结束 GC API。不得伪造“用户确认”直接复用它，也不能新加一套用户接管模式或让每次浏览都增加配置。

需要在现有 Browser／资源 owner 中用真实创建来源、任务／Session 责任、原生操作、用户使用和现有 UI 交互事实，补齐最小的退休依据。任务专用、用户仍在使用／保留现场、外部借用必须可区分；优先复用既有事实，缺少必要归属时在原领域补最小正式事实，不建立全局资源账本。

用户打开过或 Session 仍存在不能永久 pin 全部载体；用户正在编辑、跨 Turn 继续浏览、未完成下载、暂停恢复或 unknown 则不能被误判为废弃。无法证明可以无损关闭的用户工作区不准以“缓存优化”破坏性释放，必须具体列出未覆盖范围并完成相应生命周期设计，不能据此虚称旧会话驻留问题已全部解决。相关稳定合同与 browser-platform 文档须在实现时同批正式更新。

#### 退出收敛不能被无关领域错误跳过

复用现有 shutdown owner：先封住各资源新准入，再独立发取消／关闭并收集结果。某个 SSH、知识源或其他领域失败，不应使已可安全关闭的 Browser、PTY 等连清理都没有发起。实际依赖仍保留：原生共享设施只有在依赖 producer 收束后才能最终销毁，不能简单删除所有 shutdown prerequisites。

正常退出成功必须有相应 native 关闭、任务／进程树退出及句柄注销依据；不能隐藏窗口、丢 Arc、清空 registry 或关闭数据库就显示完成。失败保持真实清理状态与可重试责任，不反复发业务操作。强制退出的清理保证依靠已有 OS owner，不承诺退出钩子会运行。

Windows／macOS 的 WebView2／WK 共享进程由 OS 引擎管理，应释放自己的 controller／view／context／连接，不为实现“零进程”杀掉系统共享 WebContent 或用户浏览器。验收看本应用自有载体、精确子进程和可归属资源，无权处理的共享进程不计为可随意杀掉的对象。

## 5. 实施工作包与退出标准

采用完整纵向链路的可审阅改动；各包独立验证，不等待一场大重构才交付。包内替换必须同步类型、调用方、测试和当前文档，不保留两套裁决或双写。

| 工作包 | 主要工作 | 退出标准 |
| --- | --- | --- |
| P0 基线与失败用例 | 复核 R01–R14，盘点所有 Agent 入口、重资源寿命及正确 owner；建立现有能力基线和针对真实缺陷的失败测试 | 每项问题有现役路径、适用范围和可验证标准；不以继续写计划替代实施。 |
| P1 精确用户停止 | 明确 Step／委派／Execution 范围；修正取消结算；处理并发派生；区分 Turn stop 与其后台资源结束；补所有实例停止入口 | 单 child 停止不误停兄弟；整组取消语义保留；旧按钮不命中新代；停止事实重启后不复活。 |
| P2 真实暂停与预约 | 替换不真实的 pause→review/cleanup 路径；区分调度暂停／native 停车；打通原任务继续和预约重新获取 | 不 cancel＋retry；支持路径无损继续，不能即时停车时真实可见；唤醒不超出现有并发、不反复开新 Attempt。 |
| P3 调度与恢复 | 每 Attempt 保序业务控制 jobs；用户 Stop 可抢占；失败局部化；Session／Execution 恢复交接；可靠通知和 cursor 补读 | 控制卡顿不挡兄弟和同一任务的用户停止；恢复不同租约交错不误停、不重放；丢通知不挂死。 |
| P4 资源与释放 | Chrome 原子准入＋短锁；Computer control；公平 cleanup；任务完成自动退休重资源；全应用退出／父死收敛；exact Runtime 与 Host 缓存释放；保护回写及 Session owner | target 隔离有效；第 101 项能清理；用户不关旧会话也不累积废弃重资源；退出覆盖所有应用自有载体；不误杀合法工作或共享浏览器。 |
| P5 控制体验闭环 | 复用任务面板，汇总各域；补 subagent、后台／异常／恢复／清理中任务；展示阶段与范围，区分自动运行和计划 | 所有 Agent 可发现可停止，管理操作独立于模型；不把工程字段或新配置变成普通工作步骤。 |
| P6 集成与交付 | 执行第 6–8 节验证、修复回归，删除被替代代码与文档，更新现役架构 | 已列目标都有证据；能力保持、资源稳态、正确控制及收敛均通过；未运行项清楚列明。 |

P1 的后端与对应入口必须形成完整链路，不等 P2 的全部增强才使用户可停止。P2 不允许以“暂不处理”悄悄划掉目标；确实无法兼容当前 native 合同的部分，要记录具体原因、实现真实控制语义并明确尚未达到的验收，不能虚称完成。P3、P4 可按文件／owner 边界并行，但必须消费相同的精确身份和取消语义。

涉及 API／schema 的变更，同批更新现役调用方及 wire tests；保留支持中的功能语义，不以丢弃合法调用或删除历史解决兼容问题。数据库确需演进时使用现行合法迁移路径，不改已提交 baseline checksum，不导入退役数据，不为本专项执行数据 clean cut。

## 6. 验收矩阵

以下均为待执行。测试应验证行为与竞争结果，不是镜像私有实现或仅断言字段。专用 mock／临时数据库验证与实机验证分别报告。

| ID | 场景 | 必须满足 |
| --- | --- | --- |
| T01 | 两个独立子任务运行，停止一个 | 精确目标终止，无关兄弟继续，Execution 不被默认取消。 |
| T02 | 停整个协作 | 所有适用未完成工作受取消约束；迟到回执不重开；既有整组取消用例保留。 |
| T03 | DAG 菱形／共享汇总节点，停止一个委派范围 | 归属与依赖不混淆；无关分支不被误取消，真实缺失输入有解释。 |
| T04 | 停止与子任务派发同时发生 | 停止范围内已存在和竞态派生均不能绕过；不靠只枚举当前列表。 |
| T05 | 父 Turn 正常结束／前台切走 | 已授权委派继续，可从任务入口手动控制。 |
| T06 | 原生可恢复任务暂停／继续 | 不创建替代 Attempt，不重复已结算工具，不丢原进度；当前不安全时不显示已冻结。 |
| T07 | 无 checkpoint／不兼容／unknown effect 时继续 | 不重跑、不伪造恢复成功；准确显示原因，停止与清理仍可操作。 |
| T08 | 多个等待者同时唤醒 | 重新获取原预约；不超出原 Execution／Participant 上限，不失败、不 kill、不循环暂停。 |
| T09 | 后台 shell 持续写、模型表面等待 | 不误判无写者、不强制只读；现有共享工作区合同不被破坏。 |
| T10 | Stop／Steer 慢、失败、重复、重启 | 常规控制因果顺序和幂等保持，用户 Stop 可抢占；其他 Attempt 持续结算；不退出整个调度集合。 |
| T11 | 两租约均有效、仅 Execution 过期、仅 native 过期、均过期四种组合 | 同一原 Turn 正确交接；不因暂时无终态退休 link 或误取消，不重复效果；旧代际迟到结果不覆盖新恢复决定。 |
| T12 | 停止持久化后立即崩溃重启 | 不恢复已取消业务，只继续必要清理；未持久确认的状态不冒充确认。 |
| T13 | 旧控制／旧 cleanup 与后继 Turn 竞争 | 后继不被取消、不丢资源，旧命令不重新绑定当前目标。 |
| T14 | 通知在订阅边界丢失／UI 重连／owner 通知关闭 | cursor／receipt 补读能收敛；不永久等待，不以模型调用探测状态。 |
| T15 | 不同 Agent 使用不同 Attached Chrome tab | 底层支持的独立操作可推进；同 target 保序，现有 claim／incarnation 不失效。 |
| T16 | Chrome 调用准入与 disconnect 并发 | 关准入后无漏登记新调用；已登记者被准确取消与收束，不误杀用户浏览器。 |
| T17 | Computer 在锁队列中取消；native 操作仍执行 | 队列可及时撤销，禁止新工作；原生未结束时如实呈现，不假释放。 |
| T18 | 单个可独立终止 worker 被用户强制结束 | 不结束共享 host、用户 Chrome 或健康兄弟；已提交外部效果不冒称撤回。 |
| T19 | Turn 成功且知识回写仍在进行 | 显示原成功，回写自然完成；自动退休不 cancel_all，不拖成新的主任务完成门槛。 |
| T20 | Session 后台进程跨 Runtime soft recycle | 原 owner、原句柄与控制有效，无命令重启；上层无用引用可按真实责任退休。 |
| T21 | cleanup 前 100 项持续失败，存在第 101 项及不断新增项 | 后续项得到处理；失败不挡新业务，循环有合理节奏。 |
| T22 | 已无活动 Turn 的完成 child | 不仅返回 cleanup success；真实 Runtime 退休与后台资源状态分别可验证。 |
| T23 | 等待／暂停期间清理纯派生缓存，注入加载失败 | Session／Turn／Attempt、cursor、调用次数、计数、控制事实不变；无 send／retry／新进程。 |
| T24 | 停止本次自动运行、停用后续计划、应用正常退出 | 分别走正确 owner；语义和重启行为不互相替代。 |
| T25 | 任意 Agent 模型不响应、异常、恢复或清理中 | 用户能找到目标、提交停止、查看未确认资源；按钮不依赖模型回复。 |
| T26 | 固定负载多轮短任务完成＋独立长期任务 | 废弃任务、订阅、Host、句柄、缓存无逐轮累积；合法长期工作不被 kill 或清空。 |
| T27 | 普通构造失败／可归属异常／归属未知 | 可证明局部的失败隔离；未知共享归属不被吞掉或错误释放。 |
| T28 | 原有工具与产品回归 | 伙伴、普通会话、协作、自动任务、shell／文件／Browser／Computer／MCP／Terminal、已授权资源与长任务仍可使用。 |
| T29 | 同一 Attempt 的 Steer 永不返回，此时用户 Stop | 停止不等待 Steer 业务完成，可持久接受并送达原 owner、禁止后续工作；原 Steer 仍由 owner 收束，迟到回执不能复活任务。 |
| T30 | 完成一批任务，用户不关闭任何历史会话 | 任务专用和无剩余责任的托管 browser／进程／连接自动退休，资源数量回落到必要基线，不依赖用户点击 close；历史仍可查询。 |
| T31 | 用户手工浏览／编辑、跨 Turn 继续浏览，同时另一任务完成 | 用户现场与活任务不被任务完成／隐藏面板误关；无用户保留责任的任务专用资源仍能自动退休，不把所有 Workspace 永久 pin。 |
| T32 | 真正 Quit，存在隐藏面板、旧 Session 和已终态驻留资源 | 所有应用自有 browser／headless／PTY／MCP／worker／连接按原 owner 收束，不等用户逐个关；历史、Profile／登录与产物保留。 |
| T33 | crash 或 OS 强杀宿主，存在应用自有子进程与重资源 | 三平台适用 Job／watchdog／parent-death 路径实际收束精确独占资源，不靠 finally 或等下次启动才杀活子进程；共享 Chrome／WebContent 不误杀。 |
| T34 | SSH／知识源等清理失败，Browser 等其他资源可安全关闭 | 各域准入先关闭，互不依赖的资源清理仍被执行；真正共享依赖继续保留，不跳过所有 Browser cleanup，也不假报全部退出成功。 |
| T35 | 重开已自动退休资源的历史会话 | 查询历史不启动业务；用户明确使用时正常懒创建资源，保留适用 Profile，不重跑原 Turn／Attempt／网页输入／命令；不声称任意网页 JS／表单已完整恢复。 |
| T36 | App 退出时连着用户 Chrome，包含原有标签和任务临时标签 | NomiFun 连接／claims／overrides 全部收束；用户原有浏览器和页面保留，仅精确独占且可退休的临时页面按真实归属处理。 |
| T37 | 普通任务后台子进程与已合法移交的独立服务并存 | 普通应用私有子进程不逃逸真正退出；独立服务按既有外部 owner 合同保留；Agent 文本不能自行授予跨退出持久性。 |

故障注入尽量使用屏障、可控通知和虚拟时间，不通过随机 sleep 得到偶然绿色。检查实际执行的测试数与 feature，不能接受“命令成功但相关用例被过滤为零”。

## 7. 复杂度与能力退化审查

每个纵向改动交付前回答：

1. 修复了哪个 R／T？现有 owner 为什么不能直接完成，新增结构是否确有必要？
2. 是否复制了状态、日志、checkpoint、取消账本、全局协调或影子资源归属？若是，收回设计。
3. 普通用户或 Agent 是否多了配置、授权、资源声明或支持全部控制的前置要求？若是，违反 U03／U05／U12。
4. 是否通过默认超时、自动 kill、降并发、关工具、强制隔离获得通过？若是，违反用户要求。
5. 是否取消了回写、后台服务，或暗中创建新 Turn／Attempt？若是，先补生命周期语义，不能掩盖。
6. 是否保留了两个相互竞争的旧 reader／writer／DTO／测试／设计分支？替换时同批退役。

下列内容不作为本专项前提：新全局资源平台、万能暂停／强杀引擎、全量 shell footprint 推断、所有工具独立进程化、固定整机限额、统一任务 TTL、为队列公平性预建分布式任务系统。

新增必需的控制 scope／正式事实可以有少量真实复杂度；必须置于现有 owner 与同一事务／事件链，删掉被替代的重复裁决。不能声称新增独立控制能力完全不需要实现成本，也不能用复杂度理由遗漏用户要求。

## 8. 验证与完成标准

按变更选最小有效定向测试，阶段通过后只因新改动／失败／未解风险扩大检查。以下是入口示例，执行时核对实际 Cargo features、资产依赖与新增测试名称：

    cargo test --locked -p nomifun-agent-execution --lib
    cargo test --locked -p nomifun-agent-session --lib
    cargo test --locked -p nomifun-ai-agent --lib
    cargo test --locked -p nomifun-app --test native_execution_recovery
    cargo test --locked -p nomifun-app --test desktop_agent_shutdown

UI 对相关控制、投影、伙伴和重连用例做定向验证。涉及合同／wire／生成绑定时执行仓库现役生成与 check 流程，不手工制造不一致 DTO。

适用的边界检查：

    bun run check:agent-session-boundary
    bun run check:process-runtime-boundary
    bun run check:browser-platform-boundary
    bun run check:uarc-boundary
    bun run check:desktop-ui-boundary
    bun run check:docs

Session 内容／storage 变更同时运行受影响 canonical history、reset、consumer 测试。整合阶段属于跨层变更，运行 bun run check 及受影响 Rust／UI 集成测试；UI 文案、类型与合同同步检查。renderer 只支持桌面，最小 880×600，不添加手机布局、模拟或低于 880px 的断点。

Windows 可作为首轮实机入口，但不能用其通过推断 macOS／Linux 通过。平台开发边界、原生验收和周期见本节后续补充。其他平台环境缺失不阻止已独立验证的修复交付，也不能因此声称相关三平台验收已经完成；这属于验证证据边界，不是运行 Agent 的新资格要求。

资源验证记录原运行构建、负载、轮数、正常长期任务、存活对象／句柄／进程／浏览器资源、cleanup backlog、数据库查询量和可获得的 CPU／RSS；相同负载前后比较。无需让 allocator 立即返还所有内存，更不能要求仍在运行的合法任务不占资源。判断废弃对象是否持续累积，不能只看 healthy active_runtime_count。

满足以下条件才可报告本专项完成：

- P0–P6 的适用交付、U01–U13 和 T01–T37 有明确结果；未满足的目标不能改名为“不适用”来掩盖。
- 每个 Agent 用户控制闭环实际可用；停止与资源释放范围正确。
- 没有隐式重跑、错误取消、重复效果、新能力门槛或合法后台工作被 GC 中断。
- 相关测试通过，实机／模拟证据分开；环境限制和剩余风险具体可审阅。
- 声明跨平台全部验收完成时，适用的 OS01–OS10 必须有对应原生证据；环境缺失只能报告实现或局部验收完成，并保留尚待验证项，不能冒称全平台通过。
- 旧错误路径已退役，现役架构文档与实现一致，差异只包含本专项。

### 8.1 操作系统差异：共用合同，适配现有原生 owner

Session／Execution、精确取消范围、DAG 归属、Stop 抢占、等待预约、缓存与清理公平性保持一份实现合同；不得复制三套生命周期状态机。差异收敛到既有 platform adapter、资源 owner 和桌面退出入口。

| 范围 | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 进程退出证明 | 复用 Job、exact process handle、创建后登记再运行和 Job 成员终态证明。不能用 taskkill 或进程名扫杀代替。 | 复用 session／process-group、watchdog、generation 与原句柄；不能按缓存 PGID 杀掉未知组。 | 复用 watchdog、pidfd／受支持的身份核验 fallback 和原进程组；不能退回裸 PID kill。 |
| PTY／标准输入 | ConPTY 没有通用 Unix stdin EOF 语义；中断、关闭输入、取消、强停和 HPCON close 分别验证。 | 验证 PTY 前台／后台 job、终端退出和 watchdog；逃离原登记组的进程不能假称已被完整回收。 | 分别验证 pipe／PTY、leader 先退出、watchdog 异常及实际 reaping；保留 PID／PGID 复用保护。 |
| managed Browser | 当前 Desktop 的 WebView2；COM controller 关闭与 Tauri／Wry 注销分别确认，不能只隐藏窗口。 | 当前 Desktop 的 WKWebView；AppKit 主线程创建／操作／销毁，保留 WK 回调与实际收束证明，不能 kill 系统共享 WebContent。 | 当前 Desktop 组合没有注入此 provider；GTK／WebKit renderer 不等于 Agent managed Browser。本专项不补新后端。 |
| Attached Chrome | 当前 Desktop 已注入，验收目标独立性、原子准入、断连与 incarnation 竞争。 | 当前 Desktop 已注入，同样验收；不能以 Chrome 通过替代 WK 验收。 | 当前 Desktop 组合未注入。保留现状 unavailable，不以“跨平台一致”为由新增支持或变成其他修复的门槛。 |
| Computer | 普通／高完整性桥、物理目标锁、UAC／锁屏等待；取消一个 client 不关闭共享 runner。 | AppKit／AX 与主线程派发、TCC 拒绝／撤销；管理线程不能同步等待需要主线程回调的工作。 | X11 与 Wayland 两条现役实现分别检查；portal／PipeWire／native worker 的取消、按键释放及 join 不能用 drop Future 代替。 |
| 管理员设施 | 原 UAC runner，按 logical client 保留独立 Supervisor 和 handle。 | 原 SMAppService daemon／XPC，按连接与原进程收束；设施批准不等于任务成功。 | 原 pkexec／系统设施路径；专用创建线程寿命覆盖 child，不能改为临时线程导致父线程死亡保护误杀合法 runner。 |
| 桌面退出 | 关闭到托盘、明确退出、重启等分别走原 coordinator；事件线程在原生关闭确认前不能停。 | 加测 Cmd-Q、Dock／AppKit termination、重启与开发启动者退出；WK／Computer 收束期间主循环仍须服务。 | 加测正常退出、Unix signal 与重启；GTK 主循环和 native worker 停止不互相阻塞。 |

支持矩阵必须由当前 checkout 重新核对。已确认的源码入口：

- [Desktop provider 注入](../../apps/desktop/src/main.rs) 3407–3428、关闭到托盘 3793 附近；[browser surface 模块](../../apps/desktop/src/browser_surface/mod.rs)。
- [Windows process](../../crates/shared/nomi-process-runtime/src/platform/windows.rs)、[ConPTY](../../crates/shared/nomi-process-runtime/src/platform/windows/conpty.rs)、[WebView2 native close](../../apps/desktop/src/browser_surface/windows.rs)。
- [macOS session](../../crates/shared/nomi-process-runtime/src/platform/macos_session.rs)、[macOS watchdog](../../crates/shared/nomi-process-runtime/src/platform/macos_watchdog.rs)、[AppKit 生命周期](../../apps/desktop/src/macos_lifecycle.rs)、[WK 释放](../../apps/desktop/src/browser_surface/macos/lifecycle.rs)、[Computer 主线程派发](../../crates/agent/nomi-computer/src/macos_main.rs)。
- [Linux watchdog](../../crates/shared/nomi-process-runtime/src/platform/linux_watchdog.rs)、[Linux recovery](../../crates/shared/nomi-process-runtime/src/platform/linux_recovery.rs)、[Linux native worker](../../crates/agent/nomi-computer/src/linux_native_worker.rs)、[X11](../../crates/agent/nomi-computer/src/linux_x11.rs)、[Wayland](../../crates/agent/nomi-computer/src/linux_wayland.rs)。
- [三平台全局 OS 合同](../architecture/global-os-capabilities.zh.md)、[现有 Browser 验证](../validation/browser-platform.zh.md)、[现有 Linux 桌面验证](../validation/global-os-linux-desktop-2026-10-17.zh.md)。这些存量记录不代表本次改动已经验收，也不能把其中全部历史未验项无限绑定为本专项。

不得为平台一致性把所有工具进程化、增加权限模式或禁用缺少“立即强停”的能力。统一底线是用户能发现、精确发出 Stop、禁止后续新动作、看到真实收束和未确认资源；系统原生限制如实呈现。

### 8.2 分平台补充验收矩阵

T01–T37 的共通逻辑回归在适用平台运行；下面 OS01–OS10 是原生证据补充，不建立另一套产品行为。

| ID | 场景 | 平台要求与通过标准 |
| --- | --- | --- |
| OS01 | 启动与取消竞争、宿主骤退、孙进程／leader 先退出 | 三平台 pipe＋PTY；使用原 owner 的身份与退出证明，无误杀其他 Session，无旧 PID／PGID 重用伤害；逃逸或无法证明的对象明确未确认。 |
| OS02 | EOF、中断、取消、强停与终端关闭 | Windows 单独验证 ConPTY；macOS／Linux 验证 PTY job control。关闭输入不等于退出；进程、管道、PTY／HPCON 与原生关闭任务都可核对。 |
| OS03 | managed Browser 在导航、modal、popup、下载或 chooser 中停止／退出 | Windows WebView2 与 macOS WK 分别真实验收；保留主事件循环至收束，旧 callback 不命中新对象；Linux 按现有 provider 不适用并注明源码依据，不要求新增后端。 |
| OS04 | Attached Chrome 跨 target 并行与断连竞争 | Windows／macOS；关准入后无漏登记调用，释放 claims／override 有原回执，用户 Chrome 不被关闭；Linux 当前 Desktop 不适用。 |
| OS05 | Computer 排队、权限等待与执行中停止 | Windows 普通／高完整性与锁屏／UAC；macOS AX／TCC／主队列；Linux X11 及真实 Wayland portal 分别验证。取消后无迟到输入、粘住按键／鼠标或丢失原 owner。 |
| OS06 | 单任务 stop、单 client close、管理员设施 shutdown | 三平台分别验证三个作用范围；共享 runner／daemon 的其他 client 继续；认证等待可取消，设施重连不重放 unknown。 |
| OS07 | 关闭窗口、明确退出、重启、crash／强杀、系统退出入口 | 托盘保持合法后台工作，已完成资源继续自动退休；真正退出覆盖所有旧／隐藏托管载体。macOS 补 AppKit／Cmd-Q，Unix 补 SIGINT／SIGTERM；强杀另验原生父死收敛，无关清理失败不跳过可安全释放资源。 |
| OS08 | 系统睡眠／唤醒、锁屏／解锁及租约过期交错 | 核对该平台时钟与唤醒行为，覆盖 T11 四种组合；不新增静默／睡眠 TTL，不重置原预算，不因界面不可用自动从头开始。已有明确任务期限仍按原合同生效。 |
| OS09 | Runtime 退休与资源稳定性 | Windows 记录 native handles／Job／ConPTY；Unix 记录 FDs、进程、watchdog、PTY；按实际支持记录 WebView／WK、portal／PipeWire／worker。固定负载结束后无废弃资源逐轮累积，后台服务不误杀。 |
| OS10 | Desktop 与桌面级 WebUI 的用户控制 | 共享后端合同保持一致；UI 卡住／重连不使控制依赖模型，原生主线程繁忙不阻止宿主接收控制；WebUI 断开不是取消任务，renderer 不新增移动端规则。 |

环境记录至少包括：源码／构建身份、OS 与架构、相关运行时版本、普通／管理员身份、桌面会话种类、实际 provider、权限状态和测试数据隔离方式。沿现役部署基线验证，不随意引入新最低系统版本。

Windows 主机可完成共通 Rust／UI 及 Windows 实测，但交叉编译不替代 macOS／Linux native 运行。macOS 原生主循环／权限需要真实 macOS 会话；Linux Xvfb 可用于确定性 X11 用例，WSLg／XWayland 不证明完整 Wayland portal／用户桌面授权。Wayland 至少需要实际支持桌面的一组真实授权与拒绝／撤销测试；改动触及 GNOME／KDE 不同后端时分别补验。

可复用的进程测试入口示例，仍须核对 cfg／feature 与实际用例数量：

    cargo test --locked -p nomi-process-runtime --test parent_death
    cargo test --locked -p nomi-process-runtime --test pty_contract
    cargo test --locked -p nomi-process-runtime --test process_contract

macOS 复用现有 darwin_registered／darwin_application 与 WK／AppKit fixtures；Linux 复用现有 X11／Wayland fixtures。测试专用进程和浏览器可以受控结束；本专项仍不授权安装替换日常 App、修改系统权限、关闭用户浏览器或破坏用户数据。

环境无法获得时逐项记录“未运行／原因／需要的机器或桌面”，保留当前平台能力，不将其改成禁用条件。可先交付已独立验证的纵向修复；只有相关 OS 原生证据齐备，才声明对应三平台全部验收完成。

### 8.3 实施周期与关键路径

以下为基于当前源码规模与风险的工程工作量区间，不是 AI 运行小时数，也不是对某次连续会话的完成承诺。按熟悉 Rust／Tauri 的主要执行者、有自动化测试、三平台测试环境现成、范围保持不扩项估算；P0 和第一条精确停止链路完成后必须更新。

| 工作包 | 估算有效工程人日 | 主要不确定性 |
| --- | --- | --- |
| P0 | 1–2 | 当前 HEAD 差异、现役失败测试与平台准备。 |
| P1 | 4–6 | 单任务取消语义、DAG 委派归属、停止与派生竞争、完整停止链路。 |
| P2 | 5–8 | 原 Turn 真实暂停、无损继续、全部恢复入口重新预约；风险最高。 |
| P3 | 4–6 | 同 Attempt 保序且人工 Stop 抢占、双域恢复、通知补读与代际竞争。 |
| P4 | 5–9 | Browser／Computer 平台适配、退休与回写责任、清理公平及原生异常循环。 |
| P5 | 3–5 | 跨领域完整可见性、作用范围及停止／释放阶段呈现，区别于 P1 的基本停止入口。 |
| P6 | 4–7 | 三平台原生、资源稳定性、回归修复与合同收敛。 |
| 合计 | 26–43 | 可并行的工作量之和，不直接等于日历时间。 |

建议日历安排：

- 第一批可审阅且可用的纵向改进：约 1–2 周。优先单子任务精确停止、Stop 不被挂起 Steer 挡住、清理公平和真实释放状态；不等待全部增强一起完成。
- 一个主要执行者串行推进、由 Agent 辅助：完整范围先按约 6–9 个工作周安排。
- 2–3 条有效工程工作线并行、三平台环境已经可用：完整范围先按约 4–6 个工作周安排。

以上是排期参考，不是必须等待的最低周期；实现提前通过可以提前交付。也不能以到期为由降低能力、跳过 native 验收或宣布完成。AI／子 Agent 数量不线性缩短周期：取消 scope、暂停／预约、Runtime 退休合同需要共同确定，Rust 构建资源和实机故障验证也有串行部分。

关键路径：P0 → P1 的取消与归属合同 → P2／P3／P4 的一致实现 → P5 完整控制闭环 → P6 平台集成。P3、P4 的独立 owner 与平台测试可提前并行，避免多个执行者同时修改同一合同或争用同一浏览器／编译目录。

2026-10-19 按 P0 和第一条停止链路后的实际证据更新：P0 已复核拉取后的 HEAD，P1 已形成 API／原事件与 SQLite writer／宿主／UI 的三种精确 scope；主要剩余风险已转到 P2 的全部恢复入口、P4 的非空用户页面责任与真正退出、P5 的跨领域截止点和 P6 原生资源稳态。上述区间保留为原范围工作量参考，不再代表“全部尚未开始”。当前三条实现线已并行，Windows 定向测试在运行；用户确认暂无 macOS／Linux 主机，相关原生环境等待须单列，不能据此给出全平台完成日期或压缩掉验收。

若缺 macOS 主机、真实 Linux Wayland 桌面或现役 build 的必要权限，单列环境等待，不把等待时间伪装成开发人日，也不通过删除支持、重写所有平台或扩大本专项范围来“消除阻塞”。存量原生缺陷仅修与本次行为直接相关者；新发现会改变预算或交付范围时更新本节并说明依据。U13 已将自动退休与真正退出回收明确为 P4 的必要交付；P0 若发现现行 BrowserWorkspace 用途归属不足，须将补齐合同的真实工作量计入估算，不能沿用旧估算假称已经包含所有原生改造。

## 9. 长周期执行与交接规则

1. 启动先读本文件和根 AGENTS.md，核对 git status、实际 HEAD、现有 diff、当前进展及相关源码；不要重做已完成的纵向链路。
2. 从最重要且可验证的用户控制与真实故障开始。可以并行委派独立只读审计／不同文件实现／测试，但共享合同、编辑边界、编译资源和最终集成由主执行者协调。
3. 每完成一个工作包或发现关键新事实，更新第 10 节：完成内容、验证命令及结果、剩余问题和下一动作。用短证据替代连续日志；敏感内容不写入。
4. 技术难度、上下文不足或耗时较长不是提前停止理由。上下文压缩后继续原目标；如必须换会话，交接到同一进展表，不再写一套竞争方案。
5. 发现基线与当前实现不同，先修正具体事实和受影响步骤；不因此反复从头做“全局最优”讨论。普通实现判断自主完成，实质产品合同冲突才向用户说明。
6. 不把 review 通过、文档写完、mock 通过或预算将尽当作实现完成。清楚区分已完成、待验证、真实阻塞。
7. 最终报告列出行为变化、验收证据、能力保持结果、剩余局限和真实文件路径；不夸大为数学意义的全局最优。

实现稳定后将现役合同合并进 architecture 文档，移除根 AGENTS.md 的本专项活动入口，按仓库要求退役已被替代的施工／启动说明，不保留多版旧计划作为当前开发或生成器输入。需要追溯时使用 Git 历史；本步不得顺带清理用户数据或无关文档。

## 10. 单份执行进展

本节是唯一接续点；实现事实来自当前源码和实际执行证据。逻辑、隔离原生与未运行项分别记录，不将一次 Windows 通过扩大为三平台通过。

| 项目 | 当前值 |
| --- | --- |
| HEAD／分支 | 原实施由8c3a5cb93快进至cfee705634ccd59ae1d621b13d9ed1f95709478e，实施／平台prompt保存为a9c41c38fa276654d34ce020cd22a4a3c3b730f9。交付前整合origin/main084c7dc8d0962567075be6bcc089b39bd032fba8的8个提交；实际交付HEAD及推送状态以Git记录为准。用户仅明确授权本次commit＋push，后续平台任务不继承此授权；未安装或发布 |
| 工作区保护 | 初始AGENTS.md及两份专项文档已保留并纳入用户授权的本次提交。故障注入只用临时DB、目录、专用新Profile及受控进程；未操作日常用户数据。整合保留上游OAuth010，尚未发布的Cron迁移改011；新native一律新dataset，不改写旧临时库或真实数据的迁移回执 |
| 当前阶段 | Windows交付快照见10.1–10.4；macOS本机实施与原生接续见10.5；Linux编译、适用回归、隔离X11与真实Desktop原生验收及必要修复见10.6。真实Wayland portal、管理员设施、托盘／GUI Quit／重启／系统退出入口、睡眠/锁屏与桌面级WebUI浏览器控制仍按各节待验，不声明全部平台验收完成 |
| 本次上游整合 | 实施a9c41c38f＋origin/main084c7dc8d整合源已冻结；OAuth010原字节保留、Cron011及所有静态include／schema／boundary同步。MCP auth沿原HostShutdownStage独立retained worker关闭，正式typed诊断可证明资源收束而业务Unknown保留；新增真实阻塞owner竞争1项及合法／5损坏诊断回归1项通过。官方write/check、完整bun check、UI56、DB73、Contracts118、Session331、Cron169、MCP248+19+11、App1218+5ignored、四integration48、稳态3及新Desktop三native全部通过。以下构建与退出证据已更新为本合并图 |
| 已验证构建快照 | 最终合并图App libtest SHA256 `74f6c91bd4b5dda83f64239844608b9616f6bc37c94ebc62dfba0487661b9537`（UTC2026-10-19T06:04:05.4224637Z，346943488B）；production Desktop `661681a63e3b15fa4190ebefcf78268940b533c4b4f69cd36809426193bdf84a`（UTC2026-10-19T06:14:41.4297828Z，341702144B）。构建时为a9c41c38f＋084c7dc8d的冻结合并工作树，后续仅更新文档。debug夹具原setup/login发普通JWT，不导出trust |
| 已收束验证 | 当前App **1218通过／5原ignored／0失败，363.18s**；Contracts **118/118、0.22s**；Session **331/331、107.84s**；Cron **169/169、13.49s**；DB **73/73**（执行53、reset2、ID6、OAuth迁移3／仓储9）。MCP单元 **248/248、0.82s**，auth integration **19/19、13.71s**，connection integration **11/11、3.45s**。原Execution155/155库源未变；合并图另以当前App／DB执行53／恢复8覆盖整合链路 |
| 环境限制 | 原Windows交付时暂无macOS/Linux主机；该历史限制不覆盖10.5／10.6实跑。Linux为WSL2/WSLg，无真实Wayland portal后端、pkexec或桌面浏览器，具体阻塞见10.6。各平台不擅自提权、锁屏、睡眠、安装或启停服务 |
| 接续范围 | 本机无正在运行的编译或验收；Linux本轮修复为未提交工作区差异，提交与推送需用户另行授权。后续按10.4–10.6各自余项在独立受控环境补验（真实GNOME/KDE Wayland portal、pkexec管理员、托盘／GUI Quit／重启／系统退出入口、睡眠/锁屏、桌面浏览器WebUI），核对同一交付提交及新增差异，不重复已完成实施；有具体失败再修。待验不成为Agent门槛；整体稳定后按第9节退役施工入口 |

### 10.1 实现与验证

| 工作包 | 当前行为与实际证据 |
| --- | --- |
| P0 | 完整读取施工/启动/现役架构，按当前HEAD重核R01–R14和原资源owner；无新broker、第二执行账本、机器配额、任务TTL、worktree或声明门槛 |
| P1 | task / delegated_work / task_and_delegated使用原Step/委派事实及同SQLite writer围栏，DAG不作所有权。局部Stop不聚合取消；派生/父retry/adopt/replan受停止约束。原Attempt精确cleanup覆盖automation和软删除，原Lifecycle保留提交后信号，即使HTTP waiter消失 |
| P2 | 调度pause只停新派发，原运行工作保留。native pause/resume走同Turn/checkpoint/Event意图及原Execution/Participant预约；四租约组合读原observer，不cancel+retry。修复Paused时误关整个Session ProcessScope：native recovery8/8现保同PID/owner/starts=1，继续后可poll/精确Stop |
| P3 | 每Attempt常规控制FIFO单在途、跨Attempt独立，User Stop持久围栏/信号抢占。原Steer future、receipt/cursor补读与恢复交接保留；查询故障不授权重跑 |
| P4 | 原nonce/generation/Arc Runtime自动退休，知识回写自然join；Process/SSH/Browser独立资源owner保留。暂停relay结束后首次正式取消也调同精确退休；duplicate旧Stop不得取当前warmup载体，join前排除旧operation，锁内仍完整复核。已完成资源不靠关闭Session回收 |
| P4 Browser结算补齐 | 原repository单读快照及typed结束证明覆盖Completed automation根、Failed、canonical/scoped/aggregate取消与softdelete/fatal/source取消；无新ledger或普通API可见性变化。同Session活动任务、合法重试、用户页面与下载仍保护，跨Session委派不误当根Browser用途。当前dispatcher／BrowserService **28/28、27.57s**（14项新使命回归）；早期DB并发过载按测试并发2修正，未改产品并发／期限／断言 |
| P4 使命闭合交接 | 旧Attempt／真实current分离、Step终结重唤醒原outbox、正式replan/supersede/skip/adopt证明及active后继deferred已落。当前DB执行 **53/53、48.18s**；原Execution **155/155**库源未变。target读取原正式System DecisionRequested／settlement的canonical引用，清queue仍同continuation，不回initial；App使命14例包含于当前Browser28和完整1223，保留Question Completed及原Attempt事实 |
| P4 来源结束补齐 | Cron NewConversation创建路径捕获原tuple，正式结算封存live回执，跨合法job/run删除保原清理权限；当前Cron169/169含10项来源／删除竞争。私有互斥Cron／Remote typed来源写原accepted input，普通请求不能声明，旧重放不retrofit，不扩AgentInputSource。Remote取消沿原coordinator singleflight／shutdown，不占business64 semaphore，NeverInstalled不warmup或永久重试。Existing/InputRequired、Remote Ready保持续用途。当前source **3/3、3.26s**及完整App通过；无新ledger／TTL／重跑／声明门槛 |
| P4 Computer | 原token贯穿readonly→focus/launch、spawn_blocking、mac主队列、三平台a11y actor及Wayland原worker。queued可撤、已dispatch保原future/按键释放/真实结果。Wayland局部取消保已发布共享grant。Windows每RPC独占Event绑定connection/request，在原64slot前后检查；Stop不排第二RPC，不kill共享runner；原worker保reply/ACK，信号失败保原结果 |
| P4 Quit | Terminal、MCP auth原worker与Runtime／producer／Browser独立清理，挂起auth不挡Terminal实际delete hook或Browser close；仅观察超时不abort／重发原worker，drop外waiter后仍join。聚合前收原worker真实settled结果，native/storage仅在所有原owner已证明后关闭，域错误不跳过可独立释放资源。新增实际auth guard竞争与当前全量／三native通过 |
| P5 | 只读Activity汇总原domain/runtime/resource owners，逐实例Stop及cleanup/unknown可见；AllStop是有限原源快照。Cron停原reservation，AutoWork停原claim、保配置。AutoWork仅原root用lead Session，child独立ChildAttempt，root/child/整组控制不混淆，Remote走原binding fence |
| P6 | 现役架构及正式schema/event同步，替代路径/fixtures同批退役；强Workspace缓存按原Weak实例票据退休，Session/Companion/delete纯gate缓存不启动任务。资源稳态、原生退出、真实Edge控制、生成/边界检查见下 |

| 验证组 | 最新实际结果与范围 |
| --- | --- |
| App完整与定向 | 最终74f6c91b完整1223：**1218通过／5原条件ignored／0失败，363.18s**，日志 lifecycle-merge-app-full-tests.log。同产物Browser **28/28、27.57s**、source **3/3、3.26s**，新增auth独立关停和strict诊断proof各1通过；新进程稳态 **3/3、11.98s**。5个ignored为原需父驱动／浏览器或Windows opt-in条件项，未冒称全部默认运行。原snapshot5b为1213/5ignored，仅保留历史，不混作本轮 |
| App 集成 | 合并图扩为四组，**48/48**：browser_workspace21/0.82s（b06e1fe3）、desktop_agent_shutdown2/33.40s（38a29bbd）、native_execution_recovery8/29.21s（b6aa6880）、mcp_e2e17/22.62s（21c6f199）。构建身份 lifecycle-merge-app-build.jsonl，独立 lifecycle-merge-{name}-tests.log。原writer锁unknown→显式retry及background Process同PID／owner／starts=1暂停继续通过，观察期未改 |
| Canonical／DB／Execution | 当前Session331、DB执行53/reset2/ID6/OAuth迁移3/仓储9均通过，完整73；原remote6与Execution155对应库源未变。涵盖持久精确停止、原继续key、预约／四租约、公平清理、FIFO抢占、丢awaiter提交、专用取消证明、真实continuation及旧Attempt使命闭合。合并图以当前App完整和恢复8补整合链路；不把原库测试冒称新图重跑 |
| 自动化／AI | 当前Cron **169/169、13.49s**；Requirement58/58、AI349/349为原实现且库源未改。真实App root/child原Session／claim／grant／physicalRoot隔离及source取消CAS包含于当前完整回归 |
| Computer／System | a11y **35/35**，Computer **130通过/7原条件ignored**；System最终 **23/23**。包括真实Event只读权限/碰撞、提前/64满槽排队Stop、兄弟/后继、原pipe/ACK、drop waiter、SetEvent权限失败及任意JSON结果形状；原unknown不改写 |
| Process／SSH／Browser | process-runtime **156/156**；Windows parent-death/pipe/PTY及ConPTY/ACP opt-in native共 **27通过**。SSH后端 **47/47**、nomi-ssh **61/61**；Core **66通过/1条件ignored**，另真实Bun1/1。Browser Platform **116通过/2条件ignored**；Attached协议50、真实Chrome3及新增pending Promise跨target/disconnect1通过 |
| 归档／原资源 | App Process资源 **5/5**、Journal **15/15**；Frozen V1黄金数据/raw cursor/decoder carry/EOF/ACP936通过。16MiB原Frozen字节用同agent_payloads分块，重复ACK重验损坏不伪修复，普通/伪装/孤立payload仍计预算；128 roots/1024 chunks查询计划无相关逐行子查询 |
| MCP补验 | 合并图MCP **248单元＋19 auth integration＋11 connection integration全部通过**；App MCP e2e17、原outcome及strict诊断新回归包含于当前全量。仅原资源物理证明可用，业务Unknown／noReplay仍保留；foreign／损坏／active owner不借此放行。新661681a6三native各13原句柄通过。原request_lifecycle14与Windows literal argv1为之前证据，不冒称Unixstdio通过 |
| 普通清理成本 | 保留has_unsettled_effects快速检查及owner/auth前提，仅确有pending/unknown才读完整清理证明；Runtime／Browser复用同canonical proof，不读历史启动任务。当前稳态baseline query3687，三轮3887/4008/4043，无模型／工具重跑。新661681a6三native通过；SQLx计数口径／trace开销见10.2 |
| 真实失败驱动修复 | 已修活动Turn进程归档循环、MCP物理／业务Unknown混淆、reconciliation原producer取消误走新work准入、138-byte合法Operation误用128-byte key上限、无workspace阻退休、Robot完成双快照缺源、AutoWork非终态CAS竞争及正式使命结束未交接旧Browser。均有原DB／失败日志和回归，当前5b全量／f3d三native通过。原失败目录01a151d2-c027…、01a151d5-d0dd…、01a151e7-3e9d…保留；不将失败计为通过，不缩目标、观察期、产品期限或清除Unknown |
| 剩余业务围栏 | Runtime/Browser复用原canonical MCP资源关闭证明；无Browser workspace不承担Browser退休。其他domain/Browser未知及native/user-page/download保护不变。其余has_unsettled调用已核对为新工作/恢复/权限切换/停泊交接/破坏性操作边界，保持严格，不把可退休资源解释为业务已成功或可重跑 |
| UI | 合并图专项＋MCP管理 **56/56、10文件360断言**，typecheck与desktop边界通过。原实施曾全跑5543项：5540通过/3失败；ImageLightbox隔离复跑过，另外两项原cfee基线GuidAgentPresetLaunch.structure／ConversationQuestionLocator.structure断言未改；本轮未重跑全UI，不宣称全产品全绿 |
| 正式检查 | 合并图通过官方agent-v2-contract write→check，未手改generated。完整bun run check exit0：2205 renderer、8016 keys/33模块、3089生产文件UARC零异常及所有现役边界；日志 lifecycle-merge-check.log。现役中英文Session／Execution／Browser合同及平台prompt已同步；最终docs138文件/1421links与diff复核见 lifecycle-merge-docs-final.log。仅880×600及以上 |
| 构建故障已收束 | 旧测试exe运行时重链触发Windows文件锁、一次rustc STATUS_ILLEGAL_INSTRUCTION均已在不删缓存／不回退源码的同flag重编中收束。当前source新fixture UserId错误及两配置／同步失败均已修，保留合法snapshot、精确终态与无重放断言；当前5b完整1218和f3d生产图通过。未把失败运行算通过 |

### 10.2 资源稳态与Windows原生证据

| 项目 | 证据与限制 |
| --- | --- |
| 最终T26/OS09 | 合并图74f6c91b新进程／128MiB stack／SQL trace，**3/3、11.98s**。9短Session三轮+1合法长任务：baseline handles322/RSS171360256/query3687；三轮handles324固定，RSS176246784/176398336/176762880、CPU4968/6515/8265ms、tasks(4,7)、disposed3/6/9、model4/7/10，query7574/11582/15625（3887/4008/4043）。长任务自然结束handles313、registered0/disposed10/query16061。6真文件Session：baseline handles305/RSS169033728/query18880；三轮cache0、handles315固定，disposed2/4/6、model4/8/12、fileReads2/4/6，RSS178946048/179847168/180195328、CPU12296/14328/16171ms，query22216/25557/28907（3336/3341/3350）。SQLx完成查询含夹具观察、不含内部trigger、不存SQL文本；trace有成本，不代表生产绝对CPU/RSS。日志 lifecycle-merge-stability-tests.log |
| 最终真Desktop Quit | 最终661681a6：host41888 exit0/coordinator=true，pipe27480/ConPTY45836/MCP40880及10 WebView2共 **13个原句柄全退出**；树外sentinel41292观察时存活。新root C:/Users/rika0/AppData/Local/Temp/nomifun-lifecycle-01a152cc-6c76-7435-8361-bacad5b56870 |
| 最终真parent-death | 同661681a6，只kill原host10864（exit-1/coordinator=false，不冒正常退出证明）；pipe47560/PTY20940/MCP28920及10 WebView2共 **13个原句柄全退出**；sentinel36192保留。新root C:/Users/rika0/AppData/Local/Temp/nomifun-lifecycle-01a152cc-cf06-7f4e-a97b-e5657662f37e |
| 最终直接Stop／独立资源 | 同661681a6，原138-byte Stop→Runtime released；pipe4540/process01a152cd-369e-7720-9ec5-7df394593c40、PTY22236/process01a152cd-3b42-75c0-8e63-e11f3fbfa817和owner_generation01a152cd-3492-7f41-8e2f-275f11c1a255六身份不变，原Bun句柄仍alive。MCP20760已exit而Unknown/noReplay保留，model5/start2/MCP1不增加；host7392 Quit exit0/coordinator=true后全13退出。root C:/Users/rika0/AppData/Local/Temp/nomifun-lifecycle-01a152cd-2021-76e2-a06a-d2d591afa954 |
| 真实资料／凭据审计 | 三个新root各原13+host+Node+sentinel共16确切PID均无残留；JWT三份删除，DB3805184/3604480/3805184B及Profile／fixture sentinel保留。观察时树外sentinel存活，后仅夹具owner关闭自身sentinel／Node。原PS a738477db0427c40f725389a559cf38aaf37f4d42cb4bc62c9705df569995a70、Node f4ce30a7fd57a2e8cad601289dd09280d8cfbf44cba7a08759f87cd6fd538c56及138-byte目标／45s观察期不变。各post-run-audit.json留证 |
| 现有OS服务保护边界 | 最终三例各自before/after只读LanmanServer CIM均Running、PID5436、创建UTC2026-10-17T02:50:20.4820180Z一致，证据为各root os-service-observation.json；SYNCHRONIZE-only访问错误5，未提权或启停。仅证明现有服务保留，不替代新服务交接、原服务句柄连续性或管理员场景 |
| 最终OS10 | 实际Edge+原cookie认证/WS，d717 backend与driver记录hash一致；原ignored fixture **1/1通过、27.74s**。单Stop仅Alpha，Beta Preparing保留；有限AllStop接受2、owner未join前execution_stopped=0，分别release后消行，turns/models=0。1280×800/880×600无溢出，renderer.errors/messages/requestFailures全空，880截图已视觉核对。证据 `C:/Users/rika0/AppData/Local/Temp/nomifun-activity-browser-Aala91` |
| 最终Windows opt-in | d717的ACP936+UTF8混合text/cursor21/冻结空Poll **1/1通过**；普通token net.exe session真实denied5/exit2、原native reaped **1/1通过**；LanmanServer只读已Running，未启停/提权 |
| 最终T36 | d717原AppServices真外部Chrome **1/1通过**：external_pid22824存活、原2页DOM/草稿保留、claims与focus override释放、storage关闭；随后仅夹具owner收自身Chrome。证明provider/AppServices原生退出缝合，不冒充完整canonical模型授权或Tauri host退出 |
| WebView2其他原生 | 此前独立example构建的interaction/dialog-close/close-all三入口通过：真实用户编辑跨hidden/任务结束/后继保留，独占task-only载体自动退休。chooser最终exe SHA `af410d71f65185644531ff5c58763f27477b5ee6525e40f6bf247b1d3b337826`（UTC2026-10-18T23:22:28.8636606Z）三旗标user-files/user-downloads/picker均通过；证据 `C:/Users/rika0/AppData/Local/Temp/nomifun-chooser-native-71c2191005c043fa8231d46c54c79890/run3.json`。修的是单次trusted点击前实时几何夹具，没改生产权限/重放/期限；非人工真实选文件或保存下载字节证明 |

环境：Windows NT10.0.26200/x64，普通用户，rustc1.97.1/Bun1.3.14，ACP936未改，Chrome155.0.8059.82/Edge155.0.4283.45。所有原生夹具使用专用临时数据库、目录和新Profile；日常Chrome未接入。原生过程和构建身份细节另见现役browser-platform验证文档，不作为另一份接续计划。

### 10.3 T01–T37覆盖索引

| ID | 已执行依据／余项 |
| --- | --- |
| T01–T04 | DB执行53/Execution155及实际App AutoWork child原例：三scope、归属非DAG、派生writer fence、局部不聚合，兄弟/root产物/claim保留；原Source Stop非终态CAS竞争4例及当前完整App已收束 |
| T05 | 原Attempt独立Session/调度与Activity控制；父Turn正常结束不形成取消授权，前台切走不处置授权任务 |
| T06–T08 | Session331/DB执行53及remote-reset-id14/Execution155/current native recovery8：同Turn/checkpoint继续、未知/build/Snapshot限制、原预约；实际Bun同PID/owner/starts=1保留 |
| T09 | Process156/App Process5及T20实际原生组合，后台写者不因模型等待/Turn结束被清理，无声明/只读/worktree新门槛 |
| T10、T29 | blocked_steer_preserves_fifo_while_siblings_settle_and_user_stop_preempts与App丢awaiter/重复ACK，Stop独立持久围栏/精确信号，原Steer仍join |
| T11–T14 | 四租约、同Turn只读observer、持久取消恢复、原key/cursor/订阅补读、原running-successor隔离；新8轮warmup旧Stop同Arc/nonce/无新增退休强引用通过。实际OS睡眠/唤醒另待验 |
| T15–T16 | Attached50+真实Chrome3+pending Promise跨target/disconnect1通过；另一target可读/释放，关准入并join原调用，原页/Chrome保留 |
| T17–T18 | Computer130/a11y35/System23及App桥13：队列撤回与已dispatch结果分开、原exact owner/按键释放、共享host不kill；真实UAC/锁屏/物理输入未验 |
| T19 | App原知识回写自然join/失败保责任，主成功不改写；退休不cancel_all，不把回写变新主任务完成门槛 |
| T20 | 真Bun跨原Runtime退休、原WeakHost释放、显式后继stdin/poll、同native handle/creator/retention/digest、starts=1与最后exact Stop通过；fixture非法stdin wait_ms已移除，arguments_rejected=0 |
| T21–T23 | 第101项公平keyset清理、完成child原Runtime与资源分离、真正workspace缓存退休+factory失败、原native pause/checkpoint/budget/cursor/调用次数不变及Stop后carrier注销通过 |
| T24–T25 | 当前Cron169、App Activity与UI56／真OS10：本run／claim与计划分开，规划／preparation／异常／清理中可发现可直接Stop；当前source3和Browser14使命回归包含于完整1223。Requirement58为原源未变 |
| T26–T28 | 最终稳态3、AI349/原构造unknown隔离及各工具回归见10.1；MCP补14+1通过。完整UI两项专项外HEAD原结构断言未修，不称全产品全绿 |
| T30–T31 | Runtime/真文件缓存稳态、Browser任务页退休、自然process reap/原Frozen回放、SSH exact close；用户编辑与hidden/跨Turn原生证据通过，不能以历史Session永久pin全部Runtime |
| T32–T34 | 当前661681a6真实MCP＋pipe／ConPTY＋10 WebView三native各13原句柄退出，两正常Quit协调器成功；各16PID／JWT／data审计完成。当前shutdown integration2、blocked auth实际竞争和full App通过；未运行平台不扩写通过 |
| T35–T36 | 历史GET/缓存不发业务、显式后继正常懒创建、原生Browser关闭重开与最终外部Chrome AppServices1通过；不承诺任意JS/表单恢复 |
| T37 | 原Process/Job父死与树外sentinel保留；当前三native均同场LanmanServer只读CIM前后身份／Running保留。新建／交接独立服务及原服务句柄连续性未验（标准token错误5），未提权、安装或启停服务；Agent文字不授予跨退出权限 |

### 10.4 Windows 交付时的原生待验边界（macOS 后续见 10.5）

以下保留 Windows 交付时的待验快照；本轮已获得 macOS 主机，新增实测与剩余边界统一接在 10.5，不用旧环境说明覆盖本轮结果。Linux 原生状态未因此变化。

| 项目 | Windows已验／未验 | macOS／Linux |
| --- | --- | --- |
| OS01–OS02 | pipe/PTY/Job/parent-death及ConPTY/ACP共27 native通过，exact handle/owner；不靠裸PID消失 | 两平台watchdog/FD/PTY/job control/EOF/时钟交错未运行 |
| OS03 | WebView2导航/modal/chooser/退出及任务退休原生夹具已验；人工文件选择、保存下载字节/活跃授权下载组合未验 | WK未运行；Linux当前Desktop无managed provider，保持不适用，不为专项补后端 |
| OS04 | 协议50+隔离真Chrome3+pending read并行1+最终AppServices T36通过；任意CPU阻塞renderer/物理输入组合未穷尽 | macOS未运行；Linux当前Desktop未注入Attached provider，保持不适用 |
| OS05–OS06 | 原native排队/token/共享64slot及普通token拒绝通过；真实高完整性/UAC、权限等待/锁屏、物理键鼠执行中Stop及管理员多client组合未验 | macOS AX/TCC/主队列、Linux X11/真实Wayland portal授权/拒绝/撤销及各自管理员设施未运行；源码审查不算编译/native |
| OS07 | 当前661681a6三native各13原句柄退出、两普通Quit协调器成功，JWT删除/data保留/各16PID无残留；三轮LanmanServer CIM前后保留。新服务交接、系统退出／重启及其他worker组合未验 | AppKit/Cmd-Q、Unix信号/系统退出入口未运行 |
| OS08 | 四租约逻辑已验；真实睡眠/唤醒、锁屏/解锁未执行，不能擅自打断日常桌面 | 两平台真实桌面/时钟/睡眠未运行 |
| OS09–OS10 | 本节稳态/SQLx及最终真Edge两桌面尺寸通过；主原生线程任意繁忙/UI断线组合仍有待验范围 | 各自FD/WK/WebContent/portal及Desktop/WebUI原生控制未运行 |

待验不是新的Agent工作门槛。当前未运行项按原要求保留，不伪造通过，不为测试擅自提权、锁屏、安装或修改服务。后续有主机/独立受控桌面后按现役源码、上述精确构建与同矩阵补验。

### 10.5 macOS 接续（2026-10-19）

本轮从本机实际 checkout 接续。10.1–10.4 的 Windows 结果保留为原平台证据；此前“无 macOS 主机”是当时环境快照，本轮结果只以本小节实跑证据更新，不扩大为 Linux 或全部矩阵通过。

| 项目 | 本轮证据与剩余动作 |
| --- | --- |
| 当前收束状态 | 本轮本机授权范围内的实施、编译、适用回归和原生验收已完成；原生验收生产图为下述v4。受控睡眠/锁屏、权限撤销/授权等待、管理员设施及跨平台余项仍明确待验，不能称全矩阵或发布验收完成。下方首失败与v1–v3是各自冻结图的历史证据，不改名为v4。实施阶段未自动交付；用户随后明确授权本次commit＋push，Git交付见本节记录。未安装、提权或发布 |
| 本次 Git 交付授权 | 用户在本轮验收完成后明确要求commit＋push。交付前fetch发现origin/main新增设置UI提交a7a7e0110及整合提交1b28fb1e6；19上游文件与本轮59专项文件无交叉，已安全快进并逐一复核59文件SHA保持。只暂存本轮明确清单，使用原配置NomiFun noreply身份，不改写历史或force-push。上游仅UI/i18n变化，Rust及原生验收源码未漂移；v4原生回执仍归原构建，合并图UI检查单独记录，最终提交与远端SHA以Git记录和交付回执为准 |
| 合并图交付检查 | 快进至1b28fb1e6后的完整 `bun run check`、`bun run build:ui`（22.11s）均exit0；Browser/ChatLayout原204项与5个上游设置测试文件的合并定向UI共263/263、1629断言、13文件、6.02s。最后文档检查与暂存差异检查另执行。仅UI/i18n上游整合，原43份apps/crates验收源码SHA未漂移，未重复Rust530或将旧v4原生App冒称新设置UI整图的原生复验。日志在原根证据目录 `git-delivery-{check,ui-build,ui-tests}.log` |
| 稳定合同与后续入口 | Runtime原flight、Browser用途/门禁及HTTP/资源/storage关停合同已并入现役Execution/Browser/Companion架构。跨平台新整图及独立受控场景尚待验，本轮保留同一专项入口与启动文件供原10节接续；整体稳定后按第9节退役，不另存竞争计划或让旧平台结果冒充当前证明 |
| 最终 v4 构建／源码身份 | 同test/browser-conformance/custom-protocol构建exit0，1m29s；preparer增量构建exit0，7.47s。Desktop source `a5ee8ae5be100052727d00ad894c348f9e0a77d0a0a9679b6a5ccf0481fd0187`（513962912B），独立ad-hoc App `768bfbeabf82f7190fffb8e457ed5d1fa8e54e1ff3b2a5b7b91e32fbd3f27b50`，controller仍`8ff8cec0…`；preparer `40e83927ddf8ffd14d1778ef056478928cda6dea944d7449ed82fc05507a6c4f`。metadata `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-static-package-v4.json`，源码/fixture变更SHA集合 `source-build-v4-identity.json`，无安装/发布 |
| 最终 v4 SDK／App／必跑集成 | App libtest `b068b3abc1e18e89787c7e8e8c28338a08f767864d9900e8258ca51207917b4a`（506618560B，发现1230）；不重叠定向142=Runtime reliability92＋新double direct/background retirement1＋journal15/MCP10/effect receipt13/workspace8/writeback3，全过。AI全库360通过／1原公网search ignored，联合fixture18/18；最终desktop_agent_shutdown2/2、25.69s（`b336f6b5…`），native_execution_recovery8/8、42.18s（`847069d2…`）。合计530通过／0失败／1原ignored，不把App1230发现数计通过。新case证明原Kernel weak已evict后两个直接及两个后台退休都结束，旧Runtime Weak归零；真Bun后台writer原identity/持续写、模型数、canonical Unknown/cursor与后继warmup保持。无carrier的资源retry缺完整flight仍保守Conflict，不以None放行。证据 `/tmp/lifecycle-macos-retirement-v4-final-evidence.json` |
| 最终 v4 T36／真实 Computer 拒绝 | 同b068 AppServices T36 1/1、2.11s，外部原Chrome92609及两页DOM/draft保留，claims/override/连接/storage闭合后仅夹具原owner关闭自己的Chrome，证据 `/tmp/nomifun-macos-attached-t36-v4-final-identity.json`。同签名768bf实际App AX94744／Screen95341两新root均PASS：各自实时bundle/PID/权限before/after `not_granted`，正式typed `SYSTEM_PERMISSION_REQUIRED / not_executed`，原Kernel result23与Runtime result24精确关联，单proposal/单结果、provider2calls/hold1；原Turn Stop→cancelled/producer=false/pending0/unknown0/noReplay，原hold结束。两App/controller/preparer正常exit0/cleanup/final_verified，随机password删除。证据 `/tmp/nomifun-lifecycle-macos-20261019-computer/actual-app-denial-summary.json`；这是host preflight真实拒绝，不冒称AX遍历/capture API内核拒绝或授权弹窗/撤销。首AX driver错把MessageProjection当renderer DTO的失败保留，修原canonical引用读取后换root，未改产品/权限 |
| v4 长观察首夹具失败 | 首root `nomifun-lifecycle-01a15349-73fa-7c5f-8e9a-c2c0131a8103` 把同原Session/op两条owner settled误当重复业务；独立源码核对terminal relay与首次Stop各可持原Runtime观察者，原registry物理注销仅1次，第二worker约1s后复用成功flight，业务Model5/start2/MCP1不变。未改生产，只修harness允许>=1且同scope全部true/已观察前缀不可变，仍要求registry1、成功后无新unconfirmed/missing-owner与原代际保持。首例仅1采样不计70s通过；原FAIL、单独AppKit cleanuptrue/五原NOTE_EXIT和私有失败现场保留，审计 `cmdq-v4-first-longcase-readonly-audit.json`；新root实际通过另列 |
| 最终 v4 原生长观察＋GUI＋Cmd-Q | 新root `/private/var/folders/vp/ljg8x71x2hjdv5ng9xr_5xnr0000gn/T/nomifun-lifecycle-01a1534b-8d1a-73ac-9e84-05729c39256e`，原host96431、同签名768bf。原registry注销成功和原owner full teardown证明后，70s只读观察55采样及截止后再次全量核对PASS；无missing-owner/成功后unconfirmed增长，原Model5/start2/MCP1、Unknown=1/noReplay、原两个Process targets及三原pipe/PTY代际保持。之后同App的完整WK basic PASS（18场景模型调用、总19含seed），人工中文draft跨原Turn完成及原生close-button/恢复保留，两process target前后身份不变；唯一新pending Agent Confirm有准确request/target/operation及真实UI输入门禁。真实Cua Cmd-Q后原controller exit0/cleanup=true/final_verified，五原generation NOTE_EXIT；正常清理与首失败严格分列。原typed PNG、GUI JPEG/AX、键盘回执、canonical原事实和持久数据保留，最终退出时序/审计另取同root `result.json` 与根证据目录 |
| 最终 v4 独立审计 | `/tmp/nomifun-lifecycle-macos-20261019-root/cmdq-v4-final-combined-readonly-audit.json` 与 `cmdq-v4-final-retirement-readonly-audit.json` 均audit_passed=true。窗口70596.804958ms，首末样本单调时钟跨度70305.536833ms；physical carrier1、原scope合法settlement2均true，唯一early active-MCP warning早于物理成功，首次成功后至日志尾unconfirmed0、missing/unproven0。真实键盘→全部五原NOTE_EXIT **412ms**，不以GUI-ready等待203893ms冒充Quit延迟。五原代际已闭合，三个有原记录的辅助PID补查absent（无独立knote的辅助不扩写成generation证明）；JWT/随机password已删，DB quick_check=ok且只读审计前后SHA不变。basic最终PNG338×597/55499B；原draft三AX同URL/姓名/选项，唯一pending工具/原Turn以cancelled seq318结束，不推断缺失callback witness。首次v4失败result SHA复核未变 |
| 同步与保护 | 初始 `main` 工作区干净；`git pull --ff-only` 返回 Already up to date，HEAD=`74646fd874057b70d8b606a18ad1985196ca7e96`，目标提交 ancestor 检查 exit 0。未 stash、reset、提交或推送；后续差异均按专项文件 owner 协调 |
| 环境 | macOS 26.6.2（25G83）、arm64、uid 501 普通进程；rustc/cargo 1.96.0、Bun 1.3.14。Desktop 注入 WK managed 与 Attached Chrome；Computer 使用现役 macOS 主队列/AX；不安装、提权、修改 TCC、锁屏或睡眠 |
| 启动与构建（首轮） | 已完整读取根 AGENTS、方案/启动文件和 Session/Execution/Companion/Browser/全局 OS 架构及 browser 验证文档。按原命令构建 Desktop/browser-conformance；`bun run build:ui` exit 0（20.54s）。构建、回归与原生运行分别留证，尚不把进行中的构建记为通过 |
| 执行分工 | 复用同一 Cargo build 目录串行编译，分别执行 Darwin process/PTY/父死、WK/AppKit/controller、Computer 主队列与只读 TCC、Attached Chrome、App 恢复/退休稳态及桌面 WebUI 控制。证据置任务专用临时目录，首次失败单独保留；结果与具体未验项继续写本小节 |
| 首轮构建／检查 | 原要求 `cargo build --locked --profile test -p nomifun-desktop --bin nomifun-desktop --features nomifun-app/browser-conformance` exit 0，6m11s；binary SHA256 `d07085260a783ea0e830941adf9f83a352eac2e49e7486351e9fab0549048c3a`（505213040B，无 static WebUI，不作为 GUI 证据）。静态 UI 构建另已通过，带 `tauri/custom-protocol` 的后续构建及原生回执见本小节最终图。完整 `bun run check` 首轮 exit 0；独立 Session/Process/Browser/Desktop 边界也全部通过。日志 `/tmp/nomifun-lifecycle-macos-20261019-root/`，后续修复图须另记 |
| OS01–OS02 首轮 | 原五组 integration 实跑：darwin_application 6（4行为＋2helper）、darwin_registered 10（8＋2helper）、parent_death 3、process_contract 19、pty_contract 14；共52顶层执行项／48行为项，0失败。pipe/PTY父死、leader先退出、job-control独立PGID、EOF/真实SIGINT、并发PTY隔离与exact reap通过；不以helper或子进程输出重复增加数量。原日志及构建身份 `/tmp/nomifun-lifecycle-macos-process-20261019/`；macOS多轮FD/PTY稳态另在执行 |
| Computer 适用回归 | `cargo test --locked -p nomi-computer -p nomi-a11y -p nomi-system-execution --lib`：a11y 37通过／1条件ignored，Computer 133通过／8条件ignored，System 14通过；合计184通过／9条件ignored／0失败。mock/协议取消不替代真实AX输入、TCC撤销或管理员XPC批准；真实主队列与静默权限状态另取证。日志 `/tmp/nomifun-lifecycle-macos-20261019-computer/regression-first.log` |
| OS05 主队列 native | 原 `macos_main_queue_lifecycle` 真实 CFRunLoop，3轮 queued 撤回后 effects=0、admitted 调用仍 join 原结果、后继健康调用在主线程执行；FD baseline4→4/4/4，exit0/0.867s。binary SHA256 `2117b3bdb0affa8e33deba6681a9ea5ae2d0658a68ade2b68ba9bc3d5676456c`，证据同 Computer 目录 `native-run-hashed.json`/`manifest.json`。只读 CLI TCC 前后 AX=true、Screen=true，因此真实 denied probe 如实 not_run；该无绑定 Info.plist 的 CLI 身份不证明 NomiFun bundle 已获权。首次误用无 hash 的 example 路径 exit127 保留，不计产品失败或通过 |
| AppKit 主循环 native | 本轮四 examples 按 test/browser-conformance 构建通过（10m59s含锁等待）。原 controller/LaunchServices 实跑主循环 smoke **2/2**：嵌套 NSModalPanelRunLoopMode 中 worker completion各1次、deadline0，原 DeferredEngine get/shutdown 真实 settled；PID67170 kernel exit0、authenticated cleanup=true、final_verified=true。证据 `/tmp/nomifun-lifecycle-wk-native-yV3STn` 和 `/tmp/nomifun-lifecycle-wk-appkit-native.log`。controller SHA256 `beceafc8ac7d8fdc3ae7b5e7c3aef54bc9a24d3eeb4a2cd0c543970784f20fce`；这不替代实际 Desktop 的 Cmd-Q／组合退出 |
| 保留首失败 | Attached 原生首轮非零1例在 NotReady 失败：原 Unix launcher 只开 debugging pipe，而夹具等 DevToolsActivePort。已只为 test/conformance 的自有新 Profile 补显式端口入口，普通生产 Unix 仍 pipe；原Type缺陷及后续真实修复证据见本小节。WK compatibility 首轮在新增夹具 Draft 定位失败：HTML误传入旧 helper 的 script 槽，已修夹具并在后续独立新root通过；原PID67367 exact exit1、cleanup=false，不计通过。原日志 `/tmp/nomifun-macos-attached-native-first.log`、`/tmp/nomifun-lifecycle-wk-compat-native-first.log` 保留 |
| 已证实产品修复 | 隔离 Chrome 端口协议修正后，原 Type 空串替换真实失败（残留“真实输”，`/tmp/nomifun-macos-attached-keyboard-first.log`）。现 Type 用 `ControlOrMeta+A`，Attached 复用原 input dispatch 的 macOS 编辑 commands，保持原 pressed/release owner、两次焦点核对及公共 Press 限制；基础 trusted 输入/清空/focus trap 的新原生运行已通过。另一 iframe fixture 的 End 真实滚动语义暴露位置假设，按原严格 actionability 修测试，不扩大生产 click 容错；最终四例及 AppServices T36 结果见本小节 |
| App 必跑集成 | 原指令两 integration 实跑 **10/10**：desktop_agent_shutdown 2/2（26.86s），native_execution_recovery 8/8（29.11s）。覆盖原写入一次、writer-lock真实失败后显式retry、同Turn/checkpoint/预算继续及背景进程原PID/owner/starts=1，不重放未知效果。日志 `/tmp/lifecycle-macos-app-integrations-first.log`，两binary身份 `/tmp/lifecycle-macos-app-integration-identities.json`；本轮构建在键盘修复前，这些场景无Browser动作，最终 Browser 图另验 |
| 中间源码构建（保留） | 键盘修复冻结后，Desktop＋controller 以 `--profile test --features nomifun-app/browser-conformance,tauri/custom-protocol` 构建 exit0（5m54s含锁等待）；静态 binary SHA256 `bf08953e1fa8f625f2fe4e289734c2c0c34b84543c1048be41c60de187dc82c8`、513788688B。独立 adhoc 测试 App 签名后 SHA256 `fa407643e50e12fa2ab481c4701e3b5a2d2fd95aea887ef2b1ea5e691b82c5ca`，冻结 controller `8ff8cec099d93a105327c67fc23874fdd6fcccbc7651abf689be4d38fb5e2b04`；只构建/签名在任务缓存，未安装。metadata `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-static-package.json`，实际退出仍另取证 |
| 共通／App桥（当时冻结图） | 同图 Session **322/322、114.38s**、Execution **156/156、38.13s**、Cron **169/169、15.48s**，合计647无失败/ignored；使用macOS实际数量，不套Windows数。结果与各binary哈希 `/tmp/lifecycle-macos-core-results.json`。当时 App libtest SHA256 `5486b9aed5441855deb6632221772e278001b5fbe68a297bb0d7d6dc53547e42`（499662720B，发现1228项）；Computer桥整个模块 **26/26、8.91s**，不替代物理设备/TCC撤销 |
| OS09 App 稳态 | 当时冻结 App 独立进程、SQL trace、原固定负载 **3/3、7.66s**。9短Session＋1长任务：baseline FD43/RSS194379776/CPU2819ms/query3687；三轮FD43固定，RSS197427200/197672960/197820416，CPU3381/3937/4497ms，query7474/11261/15058（增量3787/3787/3797），tasks固定(4,7)、disposed3/6/9、model4/7/10。长任务自然结束FD35、registered0/disposed10/query15449。6真文件三轮cache0/FD39固定，disposed2/4/6、model4/8/12、reads2/4/6，RSS201179136/201261056/202620928；query21569/24869/28255（增量3300/3300/3386，baseline18269）。历史/原文件保留，无重跑；trace含夹具观察且有成本，不作为生产绝对CPU/RSS承诺。日志 `/tmp/lifecycle-macos-runtime-retirement-stability-first.log` |
| OS09 Process 稳态 | 新 Darwin fixture 当前源码 **1/1、1.25s**，另 process lib187/187＋startup_probe1/1。4并发短进程预热，三轮各2pipe＋2PTY，12个历史witness保持；长进程原generation/started_at不变，FD13→13/13/13、threads11，原直接子进程集合固定，最后自身stdin EOF自然reap后FD9/children0/admitted0/retained_jobs0/quarantine0。RSS5242880→5472256/5718016/5799936B；fixture宿主getrusage CPU16.585→28.637/40.233/51.712→52.513ms，不包含子进程。早期Mach ticks单位标签更正和所有首日志保留；证据 `/tmp/nomifun-lifecycle-macos-process-20261019/stability-final-evidence.json` |
| OS04 AppServices | 首冻结 App 真外部Chrome **T36 1/1、2.16s**：原Chrome72038在AppServices shutdown后仍存活，原两页nonce/草稿保留，Nomi claims/override/连接释放、重连围栏和storage关闭通过；随后仅夹具原owner关闭自己的Chrome。日志 `/tmp/nomifun-macos-attached-t36-first.log`。两页在attach前由外部夹具创建，当前Attached API无新建tab，不冒称应用新建临时页退休或Tauri宿主退出 |
| OS10 桌面 WebUI | 最终 App 原 ignored fixture **1/1、26.59s**，实际Chrome155.0.8059.82/headless、原cookie认证/WS、新dataset/context。单Stop仅Alpha，Beta Preparing保留；有限AllStop accepted2，原owner未join前stopped0，分别release后消行，turns/model_requests=0。1280×800/880×600无横向溢出，renderer errors/messages/requestFailures全空；880截图已视觉核对。证据 `/var/folders/vp/ljg8x71x2hjdv5ng9xr_5xnr0000gn/T/nomifun-activity-browser-tPXjOA`。UI控制回归34/34、288断言（首次漏ui preload环境失败另保留，未改产品/测试）；不代替WK或任意主线程卡住/重连组合 |
| 实际 Desktop 首轮完整组合 | 首个静态图 fa407643e 的 AppKit／SIGINT／SIGTERM／parent-death／Stop→Runtime退休→AppKit 五例均已完整通过，各新root。每例捕获原host、pipe、PTY launcher＋Bun payload、MCP共5个原generation的NOTE_EXIT；PTY父子unique_id/pid_version匹配，树外sentinel存活，原3Session/owner/event、legacy Profile哨兵/storage-generation保留，JWT/随机password已删。parent-death只杀原host：host65.55ms、全部5原NOTE_EXIT84.90ms，controller signal9/无正常receipt如实记录。正常退出32–40s的真实延迟另作为下面缺陷修复，不把它写成即时退出。完整来源/各root/退出时序 `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-resources-series.json` |
| 实测驱动修正 | 首actual图曾因preparer与Desktop work_root不同被原dataset guard拒绝（保原exit1/cleanuptrue）；下一root暴露Mac PTY ToolResult.pid是session launcher而Bun marker为payload，未改生产PID合同，改用原代际父子证明并均入库存。系统sqlite CLI readonly错误14保留，现用已有Node24 DatabaseSync readOnly真实读同WAL库，不用immutable。所有失败与分立cleanup回执保留，未算通过，不扫描进程名或裸PID清树 |
| 新真实缺陷：退出 drain | 本轮实际静态UI的 `/api/companion/matting-model` 请求持续39021ms；原Desktop先等listener，5s观察失败反复重试且跳过AppServices，直到请求自然完成才开始Runtime清理。已在原HostShutdownStage拆分资源/存储关闭证明，先封HTTP/Relay准入、并行通知原资源owner，再join原listener，全部确认后才storage。原Companion MODNet供给补关闭准入、取消网络/排队、join已开始atomic publish的原worker，不丢Future或增加timeout；模型缓存均在各隔离data/companion/models。原缓存读取open/metadata/bounded-read/hash也由同活动owner持有，丢HTTP waiter仍join原blocking I/O；最终17项MODNet回归和同图真实Desktop退出见下 |
| 新真实缺陷：WK pending dialog | 独立empty native case在原Confirm挂起的service.shutdown中失败，原service ACK=false、ownView0、controller cleanup=false保留。Mac close原先先销毁WK再join已drain dialog的evaluate回调，可能把原完成洗成UNCONFIRMED；现只对已yield且drain确认的原dialog worker在销毁前join，其他需close才能收束的操作保持原顺序、unknown仍严格。默认WK导航/history/popup/四chooser在独立root已完整normal receipt通过；修复图default/empty/nonempty已原生通过（见下），完整GUI的pending Quit已在v3/v4分别取证 |
| U13 非空 WK 真实用途修正 | 原Mac Agent Create要求真实shown/bounds，而旧set_surface(true)无条件永久pin，导致成功非空Agent任务页无法进入task-only退休子集；此缺口未改称N/A或用空carrier替代。现复用同一WorkspaceUses，Slot记录真实native锁ACK；明确Agent自动展示且全程无人类可操作用途、最新hide/detach确认后，才允许原正式使命证明进入退休。任何人工打开/页面命令、缺省来源、未确认门禁或enable前仍可能visible/Attachment均先单调pin；不延长Agent锁、不改Windows原surface路径。原13＋新13退休回归26/26、platform全lib127/127已过；失败门禁worker不能伪造ACK，正式WK非空三轮已原生通过（见下） |
| 来源 UI 与夹具修复 | 原native Attachment新增内部来源（缺省User），macOS自动事件候选仍须core真实锁证明；UI manual intent单调提升，不被自动事件降级；Session key原重挂载与迟到event/link隔离保持，无新用户模式或移动布局。Browser/ChatLayout全目录204/204、1335断言/typecheck/desktop边界通过。App两处原fixture修正：快命令完成后poll是归档只读，mock改选当前schema允许的原poll/launch证据；Activity根canonicalize通过原storage guard。原Runtime89首轮88过/1失败后原例1/1，Activity14/14＋manual1ignored；新test binary7885c27d，原5486证据不改归属，首日志/重现DB均按实际保留范围记录 |
| 最终生产源码构建 v3 | Desktop/controller四examples以 `cargo build --locked --profile test -p nomifun-desktop --bin nomifun-desktop --example macos_application_controller --example macos_appkit_termination_probe --example macos_appkit_main_loop_smoke --example macos_browser_compatibility_smoke --features nomifun-app/browser-conformance,tauri/custom-protocol` exit0（9m42s含锁等待）。source `c7baf937a4aa66d2cbe10fa9d4507abab1ac9c785ffb96e01bafc35814ef1239`、513941760B；任务缓存ad-hoc测试App `5ede9352a4fd7a6ae70d820efe82d8e0895663668ef441bd2965c48e4e74af54`，冻结controller `8ff8cec099d93a105327c67fc23874fdd6fcccbc7651abf689be4d38fb5e2b04`。metadata `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-static-package-v3.json`；未安装或发布 |
| 最终退出／恢复回归 | App libtest `b3a5ba49ec2511a197a596cfbccccd09ad4166ab906516849189225a65270ef2`（506518448B，发现1229）；Desktop/listener19、Services28、Browser gate28、Companion matting17、desktop_agent_shutdown2、native_execution_recovery8，合计102通过／0失败／1原opt-in ignored。实际HTTP屏障证明先封准入、资源关闭而storage仍开，最后HTTP退出才关storage；被drop的cache-reader也保原I/O owner。完整1229未运行，不扩大定向数。证据 `/tmp/lifecycle-macos-shutdown-final-evidence.json` |
| 最终 Desktop 单元／UI | 最终Desktop单元surface-source8＋AppKit termination10＋macOS lifecycle3，21/21；binary `35da1c9f8812fc8c003da4a24494e976faf44936514ba4b2f77e592d45cf1ae8`。Browser平台127/127；Browser/ChatLayout204/204、typecheck、desktop边界通过。完整 `bun run check` 修复图v2 exit0；最终文档与新增夹具检查另记，不套用为全量UI/App测试 |
| 最终 WK/AppKit 组件 native | v2冻结WK源码与v3相同（v3仅增MODNet cache-I/O owner）。同原controller/LaunchServices独立4root：AppKit2 modal case、default导航Stop/queued与claimed popup/精确history nonce与NativeFinished/四chooser取消、empty与nonempty各3轮均exit0＋cleanup=true＋final_verified。非空每轮原Create=1、正式Agent show/hide ACK与exact detach后才使命退休；独立用户页nonce/草稿跨轮保留，原Confirm Stop一次、旧target/dialog不影响后继、pending shutdown与原factory ACK，最终ownView0。binary `fadc47a5373d2f1a11af6b1eed94b1fa9f2934c0d1dc92a82346c9857a41a31f`，签名 `249ac4eded3c89a97e16a61e3dd3d957de1e33f135bc4db02d79ecf127c6435a`；证据 `/tmp/nomifun-wk-v2-frozen-5zcdvaao/final-native-summary.json`。草稿是semantic DOM输入，另补真实GUI输入，不冒称保存文件/下载字节 |
| 最终 Attached Chrome | 当前engine真实四例共7.96s（跨target与原pending disconnect/typed/trusted输入），协议组58（51核心＋7helper，另4ignored）、launch38及mac commands2按原过滤数记录，不重复汇总重叠项。最终App b3a5ba49 的T36 **1/1、1.96s**：外部原Chrome83707仍存活，两页原DOM/草稿保留，claims/override释放并关storage；随后仅夹具原owner清自己的Chrome。证据 `/tmp/nomifun-macos-attached-evidence.json`、`/tmp/nomifun-macos-attached-t36-final-staged-shutdown.log`；未连接日常浏览器，不冒称完整Tauri host附着Chrome组合 |
| 最终实际 Desktop 五组合 v3 | 同签名5ede9352的AppKit、SIGINT、SIGTERM、parent-death、Stop→Runtime退休→AppKit五新root全部PASS；原host84306/84600/84887/85189/85477与各原pipe/PTY launcher/PTY payload/MCP共5原代际均有kernel NOTE_EXIT。正常全部退出500.16/479.55/479.75/450.58ms，强杀父死87.89ms（controller signal9／无普通receipt如实分列）。normal均exit0/cleanup=true，listener timeout0；AppKit例原matting HTTP pending→409精确shutdown原因，client未abort，worker准入另由17回归证明。原三Session/owner/event/legacy Profile/generation保留，树外sentinel存活，JWT/随机password删除。总证据 `/tmp/nomifun-lifecycle-macos-20261019-root/desktop-resources-series-v3.json`，早期32–40s慢图另保留 |
| 真实 GUI 首失败与阻塞处理 | 首basic链原form type/select/scroll/click/submit已到达，但夹具仍在旧tool text局部找PNG，现役adapter把同batch typed附件另发紧邻user媒体消息，导致 `First WK screenshot missing typed image`。原failed report与canonical事件保留，修夹具按唯一call marker/邻接batch/PNG IHDR/target尺寸严格验证，必须新root补实际字节。期间原生工具报告Mac锁定；只对原host85960请求普通AppKit收尾，五原generation已exit0/cleanup=true，Cmd-Q driver因无真实shortcut receipt正确FAIL；不把锁屏当验收。用户现已手动解锁，继续新root GUI链 |
| 真实 Cmd-Q v3 | 新root原host87456，真实Cua native `Cmd+Q` 操作回执与App路径/PID绑定；从键盘调用开始到全部5原NOTE_EXIT **399ms**，controller exit0/cleanup=true/final_verified，原listener关闭、树外sentinel存活、三Session/owner/events/Profile/generation保留，JWT/随机password已删。人工粘贴完整中文draft，经面板hide/show原值保留；原native close-button操作后两原process target身份不变。此前Cmd-W实际关闭的是Browser tab，未冒称window-close或任意网页恢复。证据新root `nomifun-lifecycle-01a15332-a068-703d-a411-908b685a627e/{cmdq-native-summary,resources-before-close,resources-after-close}.json`；basic首截图解析通过后因按钮在窄pane fold外再次FAIL，另保留并修fixture可见布局，不放宽生产actionability |
| 新真实缺陷：迟到 Runtime 退休 | 长GUI观察在Stop已收原cleanup成功证明后，仍有8次 `Original effect resource cleanup owner is unavailable`（1/2/4/8/16/32/60/60s退避）。canonical原MCP仍Unknown但local confirmed/remote not_required、settled与host_cleanup_proven均保留；原teardown成功后exact Kernel弱缓存已移除，迟到退休却重新查Weak并永久pin expected Runtime到Quit。已定位为完成交接缺陷，已用原Runtime retained teardown flight的成功证明收束，不将None视为成功、不重建/重跑、不洗白Unknown；本修复已另构建v4并通过Stop长观察和真实退出（见本小节当前行） |
| 完整 GUI WK basic／pending Cmd-Q v3 | 新联合preparer `d7e8d9b89e45e1f35a3e5c3845603f8c08c8e3d41c2492dcedef8289d98b637a`：截图只接受同tool batch紧邻媒体中的唯一call marker/typed PNG，完整decode核对target/media_type/宽高；计数按钮仅移到提交同行，未加重试/放宽生产refs。新root `nomifun-lifecycle-01a15338-850f-7037-98fa-55d66e2940b8`，真实App PID90112；原basic **PASS**，18本scenario模型请求（总19含seed）、semantic form/select/scroll/click、unsupported drag、旧doc ref拒绝、截图后原ref单次点击count=1，最终338×597 PNG已视觉核对。人工中文粘贴draft跨原Turn完成仍保留。之后仅一次新pending场景，原Confirm request/target/operation和`confirm-open`一次、UI禁用toolbar/Agent输入锁均留证，再真实Cmd-Q取得全部原5代际退出与normal cleanup=true。原失败不重放、不覆盖；脚本/GUI/typed PNG与canonical原事实各自保留 |
| 当前跨平台与未运行边界 | Windows的surface dispatch保持原User分支，但共享Slot native ACK/unlock前用途pin及本次SDK teardown证明有共通变化，旧Windows原生证据不算当前整图复验；本轮不重复Windows实施。Linux/Intel/最低macOS14/正式签名与notarization未运行。AX/TCC实际App前置拒绝及原Turn Stop已验；真实权限撤销/等待授权时取消、物理输入中Stop、SMAppService/XPC管理员多client、睡眠/唤醒/受控锁屏和系统重启退出未运行，当前日常桌面不具已授权独立受控环境，不擅自改变权限或打断系统。chooser只证明取消/关闭，未声称真实选文件、保存下载字节或活跃授权下载组合；完整App1230与全产品全量UI未跑。仅关闭精确自有view/context与原进程，不要求共享WebContent进程归零 |

### 10.6 Linux 接续（2026-10-19）

本节只记录本机 Linux 实跑；不以 Windows/macOS 结果冒充 Linux，也不以 WSLg/XWayland、私有 D-Bus 或 mock 冒充真实 Wayland。证据目录为仓库外 `/tmp/nomifun-lifecycle-linux/`（下称 E），不含 JWT、密码或用户数据。每条证据注明源码／构建；“最终图”专指下表最终构建。

| 项目 | 本轮证据与边界 |
| --- | --- |
| 同步与保护 | 初始 `main` 干净，`git pull --ff-only` Already up to date，HEAD 含 `74646fd874057b70d8b606a18ad1985196ca7e96`。用户告知远端 macOS 更新后，先把全部本地差异备份为仓库外 patch 与逐文件 SHA，只还原本轮测试生成的 ts-rs 绑定，快进至 `a7d79aea2` 再 3-way 重放；稳态采样改用上游共用 Unix `native_fds`。溯源用的父提交 detached worktree `/tmp/nomifun-lifecycle-linux/wt-084c7dc8d` 已 `git worktree remove` 并 prune。未 stash、reset、提交或推送 |
| 环境 | WSL2 内核 6.18.33.2-microsoft-standard-WSL2、Ubuntu 24.04.4 x86_64、32 核／31GiB；rustc/cargo 1.97.1、Bun 1.3.14、Node 24.4.1、Python 3.12.3；普通 uid1000，未用 sudo。WSLg 1.0.73.2（Weston），`:0` 带 XWAYLAND；`XDG_CURRENT_DESKTOP/XDG_SESSION_TYPE` 未设，logind 会话无 seat |
| Provider 适用性 | `apps/desktop/src/main.rs` 3446–3451 在非 Windows/macOS 令 `browser_resources=None`、`attached_chrome=None`：OS03、OS04 与 T15/T16/T36 在当前 Linux Desktop 不适用；本专项未新增后端 |
| 构建身份 | 规定命令 `cargo build --locked -p nomifun-desktop --bin nomifun-desktop --features nomifun-app/browser-conformance`。首图（HEAD `74646fd`，无本节修复）`b8fe5f31fd7d88ea…`；中间图 `c47b1dd2451b…`、`08fa2ca48068…`、`32a49e937752…` 已被替代，仅在引用处注明。**最终图** `e9665b56dfef0c022dbd802a0eb4804ab43be97e78ee38086f3fb6f8fa2e7c55`（936644760B，UTC 2026-10-19T15:44:31Z），源码＝HEAD `a7d79aea2`＋未提交代码差异 `git diff HEAD -- . ':!docs'` SHA256 `fa636b5feb4dd85bff5f1d085a766fcde586c9e45c2a971fca9d6f11090ff787`；构建、final6 原生与 T28 运行前后该值不变。原生报告另记共享 `scripts/lib/source-identity.mjs` 给出的完整差异 `1ee7ff63a07b…` 与 1 个未跟踪文件哈希。其后仅把该文件以 intent-to-add 注册（内容 SHA256 `909aa16a…` 与报告中的未跟踪哈希一致）并改名一个 X11 测试，当前代码差异 `bb7fb838d74e10ceb2fa12d1ce793be64c5c6d0fca1c79e3cf71bfe94454d9ee`；生产代码与最终图一致 |
| 产品修复（唯一生产代码变更） | Execution 调度竞态：回答在中断对账快照后提交时，原 Attempt 被取消、Execution 判“无可调度步骤”失败；暂停中已准入回答被当作中断调用而转为 review 阻塞。现中断对账跳过任何带待投递 write-ahead 的 Running Agent Attempt（不论是否暂停），投递候选与空转 `continue` 共用同一谓词，空转失败只按判定快照版本 CAS 提交且与 `fail_if_active` 共用租约／版本权威判断。4 个确定性回归；最终源码 Execution lib 160/160，调度模块 16 线程两轮 46/46，DB `agent_execution_repository` 53/53 |
| 测试对齐（非产品变更） | ① `pool_lifecycle` 两项（Windows 因无 sshd 跳过）：两个关闭等待者同时挂起、teardown 中途取消一个，回执在原命令结算后返回，结果为 retired/admitted-output 退休中断或退出码 130；断连无退出证明为 Lost；最终源码 3×19/19 ② 4 个仅 Unix 的进程退休夹具改为在真实 canonical Turn 内正式派发，后继 Turn 同库；每进程 reap 后 owner-released 为 0，校验器未放宽；`cleanup_runtime_session` 的 owner-released 写入因此暂无 App 级用例覆盖 ③ DB 四租约用例写入时取时 ④ `darwin_stability` 改名 `unix_process_stability` 并扩至 Linux /proc，读取失败只容忍已消失线程 |
| 最终图原生：退出与父死 | `linux-desktop-lifecycle.mjs`（复用共享资源侧车）在私有 Xvfb/Openbox 与隔离数据集运行，pidfd 绑定原 PID＋启动时间，只向核实的 App 发信号，树外同镜像 sentinel 必须存活。E/native-lifecycle/final6-*：`sigterm` exit0，App 及其 8 个原后代（2 WebKit、3 Agent 子进程、3 watchdog）≤679ms 全部退出；`sigint` exit0，App＋8 个 ≤506ms；`parent-death`（SIGKILL）-9 无协调器，8 个原后代 ≤44ms 由原 watchdog 收束；sentinel 均存活，模型请求 5，无残留原进程。均为信号退出，不是 GUI Quit |
| 最终图原生：Stop | final6-stop：Turn Stop 后 MCP 子进程及其 watchdog ≤134ms 退出，Runtime 释放、MCP Unknown 保留且不自动重放，pipe／PTY 原身份存活；随后 SIGTERM 时 App 及其余 6 个原后代 ≤359ms 退出。单个 ProcessInstance 精确 Stop 的原生证据只来自旧驱动版本在中间图 `08fa2ca48068…` 上的 final2-stop（仅 pipe 及其 watchdog ≤12ms 退出，PTY 保持）；现提交的薄驱动不含该阶段，不能以其复现 |
| 最终图原生：X11 与 T28 | `linux-computer-native.sh`（私有 :90、禁 TCP、私有 Xauthority）3/3（`nomi-computer` 源码自本节修改后未变）：焦点／截图／输入；拖拽按住期间 Stop 时原 worker 被 join，恰一次 press/release，无残留按键。原名为 stop_before_native_dispatch 的用例已按实际内容改名 `cancelled_token_is_rejected_at_entry_without_x11_input`：token 已取消，6 类输入在 `execute_authorized_controlled` 入口即返回，只证明入口守卫，不覆盖“已准入未派发”的排队输入撤回；改名后重跑 3/3。T28 默认 Minimal files-process 19/19、General desktop 86/86（E/t28-final6，会话 `01a154df-54d7…`／`01a154df-b6ad…`）；更早构建上的同两项结果已被替代 |
| 回归（源码注明） | 最终源码：Execution lib 160/160、`pool_lifecycle` 3×19/19。与最终源码只差两条调度日志及测试／验证脚本的前一源码（`32a49e93` 同源）：App lib 1207 通过／1 失败／3 原 ignored（失败为 `non_macos_computer_permissions_do_not_block_launch`，无 DISPLAY 时失败、私有显示通过、父提交同样），integration 分别 browser_workspace 21/21、desktop_agent_shutdown 2/2、native_execution_recovery 8/8、mcp_e2e 17/17，App 稳态 3/3（`native_fds` 约 53→42、registered 1→0），调度模块 2×46/46、DB 53/53，5 项边界检查 exit0。更早源码（`08fa2ca4` 同源）：AI 351/351、Companion 363/363、Session 322/322、`unix_process_stability` 1/1×3 与完整 process-runtime；拉取 macOS 更新后、调度去重前：Cron 169/169、MCP、Engine core、Browser platform/engine；拉取前：Gateway、Requirement 等其余回归。首图源码：process-runtime 原生 26/26。调度后续两次改动仅涉及 Execution 内部判定与日志，未重跑与其无依赖的套件 |
| OS 映射 | OS01/OS02 见上。OS05 X11 焦点／截图／输入与在途拖拽 Stop 通过，已准入未派发输入的撤回未覆盖，真实 Wayland 未验。OS06：单任务停止＝Turn Stop（最终图）；ProcessInstance 精确 Stop 仅有旧驱动在 `08fa2ca4` 上的证据，最终图无可复现原生证据，列为待补；管理员单客户端关闭因无 `/usr/bin/pkexec` 未运行。OS07：只验证 SIGTERM／SIGINT 退出与 SIGKILL 父死；关闭窗口到托盘、显式 GUI Quit、“重新启动应用”与系统退出入口未运行。OS08 睡眠／锁屏未执行。OS09 App 稳态与进程稳态见上。OS10 无桌面浏览器或 Playwright，WebUI 浏览器控制未执行 |
| 未修存量 | `bundled_role_inventory_matches_native_action_resource_contracts` 在父提交 `084c7dc8d` 同样失败（冻结 inventory 的浏览器 manifest 摘要早于 `ecf3b6f46`/`1b9c317ec`，与平台无关）；上述 DISPLAY 依赖用例同为存量。均非本专项改动，未顺带修改 |
| 暂缓（WSL 不可验） | 按用户要求不在 WSL 强行操作：真实 Wayland portal（仅 GTK 后端，无 RemoteDesktop/ScreenCast/Clipboard，无 gnome/kde/wlr portal，PipeWire 未运行）授权／拒绝／撤销；OS06 管理员；OS07 托盘／GUI Quit／重启／系统退出；OS08；OS10 WebUI 浏览器。需真实 GNOME/KDE 会话、polkit 管理员或桌面浏览器环境补验。另：提交的驱动不含单进程精确 Stop 阶段，该项需在共享侧车补阶段后重验 |
| 证据保留 | E 下保留 final6 原生报告、t28-final6 与回归日志；仅保留 final6 与 t28-final6 引用的 5 个 owner、5 个 X11 夹具根（无 JWT/凭据），其余带标记的一次性夹具根已删除。Git 索引：`unix_process_stability` 改名经 `git mv` 已暂存，两份新 Linux 脚本为 intent-to-add，`scripts/lib/source-identity.mjs` 同为 intent-to-add；未提交 |
