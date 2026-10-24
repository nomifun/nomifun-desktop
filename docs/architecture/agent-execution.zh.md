# Agent 执行与智能决策

AgentExecution 是一次目标执行的领域聚合；AgentSession 是唯一会话、Turn、事件、内容、效果和取消权威。Execution 组合现役 Runtime，不复制模型/工具循环、会话日志或完成合同。Canonical 边界见 [Agent Session](agent-session.zh.md)。已完成重构的映射、施工顺序和旧设计只保存在 Git 历史。

## 生命周期约束

以下 U01–U13 是持续适用的实现与产品约束；验证范围和未验平台见[生命周期验证记录](../validation/agent-lifecycle.zh.md)。它们不增加 Agent 的运行资格。

| ID | 稳定约束 |
| --- | --- |
| U01 | 修复原 owner、准入与交接的实际故障；不能仅改显示状态、隐藏异常或叠加业务重试。 |
| U02 | 复用现役 Session、Execution、TaskTracker、原生 Supervisor 和 outbox；不新增平台 Broker、通用生命周期平台或第二账本。 |
| U03 | 保留现有工具、权限、模型选择、任意 shell、共享工作区、后台／长任务和明确配置；不要求工具支持全部暂停／强停模式才能运行。 |
| U04 | 保留用户并发和工具明确期限；不新增固定整机配额、统一任务 TTL，或按资源压力、空闲、等待时长自动杀任务。 |
| U05 | 不强制 worktree、资源／读写路径声明、只读降级或额外资源选择；不能假称完整推断任意 shell 的副作用。 |
| U06 | 父 Turn 结束、模型返回或前台切走不撤回已授权委派；取消只按真实用户授权范围生效。 |
| U07 | 缓存不能启动、恢复、重试任务、发送原始输入、新建 Attempt 或调用工具；加载失败不授权重跑。 |
| U08 | 等待、暂停、恢复和未知效果中的必要现场必须保留；idle、Finished、checkpoint 或活动计数不证明可回收。 |
| U09 | 每个 Agent 有宿主侧用户 Stop 和可见清理进度；异常状态不能隐藏入口，Stop 不等待模型、Steer 或普通业务 FIFO。 |
| U10 | 单任务、委派归属、DAG 依赖和整个 Execution 分别控制；旧控制不能命中后继，已停止范围不能被恢复或派生绕过。 |
| U11 | 原 Turn 继续与新 Attempt 重试严格分开；不以 cancel＋retry 假装继续，不重置原进度、效果或预算。合法显式预算调整和重试策略保持。 |
| U12 | 普通用户看到任务、等待原因、停止与真实清理状态；owner、代际和 checkpoint 校验不成为额外工作配置或确认门槛。 |
| U13 | 无剩余责任的重资源自动退休，不要求用户关闭历史；真正 Quit 收束应用自有资源，崩溃由原生父死覆盖。保护用户浏览器、共享服务、合法写者和持久数据。 |

## 执行域所有权

对外入口是 `AgentExecutionEngine`。Planner 产生 DAG，Router 选择 Participant，私有 Scheduler 管理 ready、租约和并发，AttemptRunner 通过现役 Session owner 执行一次 Attempt。其他领域通过 facade 命令或窄只读 guard 交互，不直接修改执行状态。

| 记录 | 责任 |
| --- | --- |
| AgentExecution | 目标、策略、版本及生命周期 |
| Participant | 本次执行可使用的不可变 Agent/模型快照 |
| Step / Dependency | 工作单元与 blocker → blocked 的 DAG |
| Attempt | 一次真实派发的最终配置、操作身份和结果 |
| ConversationExecutionLink | 同一 Session 与 Execution/Step/Attempt 的显式关系 |
| ExecutionEvent | 先持久化的状态、决定和审计事实 |
| AgentExecutionTemplate | 创建前可复用的协作方案输入，实例化时读取一次并复制 |

模板不是活动配置；Execution 不回读模板，模板更新或删除不改变已运行快照。Attempt 内委派只向同一个 Execution 原子追加 Step，复用 Participant、策略、调度与生命周期，不建立嵌套执行聚合。

身份、技术主键、逻辑引用与删除策略遵守 [ID 体系](id-system.zh.md)。Execution 的领域 Event 不替代 Session 的模型、工具和 Effect 事实。Conversation 名称始终投影同一个 Session 身份。

每个已准入 Step 都有现役 lifecycle `TaskTracker` 持有的精确宿主 job，从原预约持续负责到 Attempt 结算或已确认交接。调度循环只观察完成通知，不靠轮询 `FuturesUnordered` 才推进业务。观察者离开、暂停派发或调度换代不能丢掉输入、已返回结果及其结算责任。job 在 Queued 写入前登记；创建回包尚未绑定 Attempt ID 的短窗口由原预约版本证明归属。后继调度代可把已确认的新租约交给同一个 job 结算，不重新调用业务；registry 只是活 owner 观察，不是第二份持久执行账本。

租约观察区分 `Confirmed`、`Unconfirmed` 与 `Fenced`。续约报错或回包丢失只表示 `Unconfirmed`，不能证明另一 owner 已接管。此时暂停新派发，原 owner 保留 job，并复读真实租约 owner 与到期时间；续约已提交而 ACK 丢失可据此确认。确切的其他 owner、聚合删除或终态权威才围住旧调度代。所有准入和结算仍受原 writer 的租约、版本校验；保留工作不赋予旧租约新的权限。

runner 准备之前，Queued Attempt 已在现役 `InitialInput` outbox 保存完整原始 `SendMessageRequest`；初始 operation key 从不可变 Attempt ID 唯一派生。准备核验该确切 Queued 预约及其 Session opening 或 continuation 证明，保留原内容、来源、可见性和 channel，可以补入原授权冻结文件与 Skill，但不能丢掉原选择。typed 暂态存储错误或待准入状态即使发生在 Link 建立前，也保留同一 job、Attempt、Session 和创建 key；它既不是 Agent 业务失败，也不授权删除重建 Session。`on_linked` 在同一 writer 保存精确子 Session Link、准备后的输入和 Attempt 状态；它只表示关联建立，不表示输入已受理或模型已启动。canonical 输入 writer 在受理该原 operation、确认原队首的同一事务内核验来源租约、Link、Step／Attempt 版本及 pending 输入。事务外预读不能授予启动权限，也不能从现模板、最新任务背景或 Session 最新输入重建原意图。

Execution 事件先持久化，再由现役发布 owner 交付。业务准入、关联和结算只通知发布者，不等待无关 UI／WebSocket 消费；确需观察顺序的显式发布屏障仍保留。

## 策略与有界调度

三类现役顶层策略保持正交：

| 策略 | 作用 |
| --- | --- |
| `delegation_policy` | `disabled` / `automatic` / `prefer_parallel` |
| `decision_policy` | 自动决定或请求用户 |
| `adaptation_policy` | 固定计划或允许有界重试、改派、重规划 |

Step 的自由文本 role 只作展示和路由提示；独立 typed tool policy 只能收缩原 Session 权限，不能按角色文案授予工具。Attempt 保存实际生效配置。模型池与当前 lead、模板选择在同一事务验证，有限池必须包含 lead；切换 lead 不保留失配模板。

共享上限来自 [agent_execution.rs](../../crates/backend/nomifun-common/src/agent_execution.rs)：16 个模型、64 个当前 Participant、128 个当前 Step、64 并行度、4 层委派深度。历史 revision 不计入当前工作集，也不能为满足上限而删除。保存的模板同样受可执行 Participant、模型与并发上限约束。

## 原 Turn 内的子 Agent 协作

`agent/delegate` 的 `strategy=parallel` 接受有界、互不重叠的 `tasks`；每项物化为没有人为前置依赖的普通 Agent Step。`strategy=planned` 由现役 Planner 为 `goal` 产生真实依赖 DAG。自动附加汇总 Agent 的旧 `synthesize` 开关已退役，旧字段拒绝受理；用户明确配置的 synthesis Step 和模板仍按普通 DAG 执行。父 Agent 保留任务所有权，负责整合、检查和最终回答。

受理回执保存 `execution_id` 和已有确切 `step_ids`；新聚合的计划尚未物化时，Step ID 可暂缺，后续由状态查询读取。回执只证明工作已受理，不表示已启动或完成。显式协作意图不截断父 Turn 的冻结工具表，也不把每个模型步骤固定为再次调用委派工具。已经受理的命令不能换新调用身份反复提交；相同原命令的恢复仍使用原 Effect 与领域幂等事实。Attempt 内追加及同一来源 Turn 的后续工作复用原 Execution，委派归属来自原工具 Effect 和 `PlanChanged`，不能从标题、目录或 DAG 顺序猜测。

必须首次委派的判定来自当前 canonical 真人受理事件与已准入来源，不能由客户端 `origin`、子 Agent 角色或父任务背景自报。正式 human-intake 的 producer、owner、命名空间及消息身份共同校验；Host 续发不会因重复原文再次获得该要求。任务信封的 Step 解析另需既有 Execution 来源 scope 与内部输入证明。它只选择本次任务文本，既不扩权，也不禁用孩子的协作工具：用户明确要求嵌套委派或当前任务确有独立分工时仍可继续委派，原并发设置和并行偏好不变。

`agent_status` 只读取本次确切来源 Turn 所属的协作，可进一步选择确切 Step。结果包含原 Execution、Step、Attempt、子 Session、Turn 和 canonical terminal 引用；正文与产物通过原 Session 输出读者读取，不把 UI 摘要、截短展示或模型自述当作结果。`agent_message` 只向该所属范围的确切 Step 发送有幂等身份的控制消息，复用现役 Step／Session 命令，不获得其他任务权限。

普通指导、明确问题回复和 `task_control` 使用不同 typed intent。`question_reply` 必须引用状态返回的原子 Session／Turn／Question Event／request ID，并通过原父命令、归属范围、原绑定、清空边界及当前问题核验，复用同一 Attempt 的 DecisionInput outbox。正式 `reply_to` 由 Host 从原问题派生；输入保留父 Agent 来源，不伪装真人回答或许可。`task_control` 复用现役 Engine 对确切 Step 的核验、重试、停止或以已验证完成 Step 正式替代命令，保留来源所属范围、expected versions、不可变命令摘要及原回执。真人许可、未知效果、Stop 和失效来源仍不能越过。状态竞争等正常命令拒绝作为结构化工具错误返回，父任务可以继续协调；canonical 来源损坏仍是硬错误。写后确认丢失只查询原命令回执，不换 key 重发。

已受理的回答等待容量时，旧问题已被回答，不能继续作为 attention 反复唤醒父模型。尚未交付的结果保持等待；未回答问题或未解决 review 按确切决定身份呈现，不为同一状态重复唤醒模型。父 `agent_wait` 可以让出其原 native 模型名额，使原 outbox 继续交付；不修改用户的并发上限。

父 Agent 可以先做独立工作，再用 `agent_wait` 等待任一新状态或全部子工作。等待保存 typed `AwaitingChildren` 和配对的 `ChildrenWaitResolved`，保留原 Turn、operation、任务计划、已结算工具、模型步骤与预算。恢复只重挂原等待，不重发委派、创建新 Attempt 或重置预算；恢复的等待结果仍须以 `agent_status` 读取和整合。它不是用户 Pause、`InputRequired` 或一个伪造的用户消息。通知只是重读提示，状态、结果和停止围栏仍来自 canonical 事实；普通排队输入维持原 FIFO，显式 steering 在原安全边界接入。

等待使用按 owner／Execution 定位的提交通知及原 Turn 输入通知，先注册等待再读状态。无关 Execution 不唤醒此等待。30 秒兜底批量比较领域与相关 Session 的 native revision；无变化时复用内容缓存，变化时读取窄快照，并增量合并最小来源／控制／结算证明索引。原工作是否仍有真实推进者另经 scoped 只读 owner 核验，不能因没有新领域事件就假定 owner 永远存在。等待观察不读取 Participant Snapshot、launch config 或完整启动 DTO；原结果正文仍在需要状态交付时由 canonical 输出读者解析。订阅、游标和缓存均为可重建观察，不承担执行、归属或交付责任。

领域取消可以早于原 native 执行真正结束。原精确取消返回成功及 cleanup ACK 后，再向已有 owner／Execution 观察者发送仅重读提示；不得因领域终态已发布就漏掉后续原生变化，也不新增领域事件、模型轮询或业务重试。通知丢失仍由原稀疏游标检查兜底。

首次等待准入、已受理等待与纯 `agent_status` 读取遇到 typed `PoolTimedOut`、SQLite `Busy`／`Locked` 时，保留原 Turn 并做可取消抢占的退避重读。该重读不新增模型步骤、重置预算、重派子任务或自动重试 `agent_message` 的写命令；损坏、I/O、关闭连接池、缺失事实和权限错误仍按其原失败处理。通知或暂态恢复不解除结果义务，分页完整消费与原持久回执继续是唯一交付依据。

数据库诊断在现役读快照和写锁准入边界观察原 operation 的等待、取得与结束阶段，稀疏记录慢操作及连接池 size／idle。它不安装 SQLx acquire／release hooks，不为诊断调用 SQLite，不修改连接健康或任务状态，也不建立诊断事实账本。

显式 `agent_wait`、自动等待与恢复等待共用每个所选 Execution 最近一次成功观察的游标。省略 `after_cursor` 不能丢掉现有水位；显式游标必须属于所选范围，其他 Execution、Step 或正文页的游标不授予权限。游标以最多 64 个 Execution 引用构成不超过 16 KiB 的 canonical 内容。来源观察覆盖完整所属头部的 Attempt／准入、终态、问题、review、控制和正式替代身份，不受正文选择影响。一个 Step 变化只使该 Step 已变化的来源与分页覆盖失效；改变范围组合不能把已消费的未变来源重新当作新消息。state 与 ready 来源摘要分开：仅已呈现 ready 来源集合减少时，只推进本地订阅，不写 `ChildrenWaitResolved` 或新增模型步骤。恢复仍受原 build／checkpoint 围栏约束。

新的真实问题、review 决定或调度暂停按原事实身份唤醒父 Agent 一次。呈现决定不等于处理完成，仍须原命令受理或原事实核验确认。已呈现的决定或 Running 工作没有精确 Step job、已受理输入、native owner 或派发 owner 可以推进时，等待返回 typed `decision_blocked`，Runtime 为技术核验停车，不重复采样模型，也不无 owner 地永久空等。派发 owner 必须确有可派发工作，仅守着 review 的循环不算推进者。它与用户 Pause、真实待输入分别表达，也不伪造真人问题。计划阶段已正式失败或取消而没有生成 Step 时，状态读取返回原聚合终态事件，不能把空集合当作成功，也不能永久等待不存在的孩子；控制器结果与普通 Agent 的 canonical Turn 输出分别读取。

结果正文按 UTF-8 字节分页，每片最多 32 KiB。`result_page` 固定不可变 Attempt／native terminal 来源、完整正文 SHA-256、长度和精确 offset；本次正文预算耗尽时，明确返回尚未阅读的首片引用，不把截断文本当作完整结果。父 Agent 只有连续覆盖从零到完整长度后才能消费该终态，单独读取末片不能解除义务。native checkpoint 只保存有界来源、摘要和覆盖范围引用，正文仍在原 canonical 事件内。选择不同正文页不改变全部所属工作及唤醒游标。

确切子结果同时报告该 Turn 的工作区 Effect 或 native process 命令是否可能改变工作区；部分或 unknown 效果同样使旧观察失效，仅原进程 owner 的精确 typed 执行前拒绝能排除未执行命令。每个新的不可变来源只推进父任务工作代次一次，并使旧 completion 证据与 scoped instructions 观察失效；重复读取同一结果不反复失效。工具策略或无关 Execution 事件增长不能证明工作区发生修改。

`workspace_change_sources` 覆盖该 Step 所有原 Attempt 及其确切 initial／问题／decision Turns，与当前结果正文分开。最新 Attempt 尚未启动就被取消，不能抹掉更早 Attempt 的真实写入。业务已终结但原 native Turn 仍运行的 Agent 保持 `delivery_pending` 等待状态，不计入可消费终态；确切 native closed 回执，或持久领域准入围栏加该原 Turn 确实不存在的证明，才解除交付等待。`started_at`、最新 Turn 猜测或显示取消不能证明零启动；native 结果关闭与重资源清理继续分开。

source 委派不生成 `lead_report_operation_id`，不扫描事件后追加 `lead_report_delivered/projection_suppressed`。变化通知只唤醒原观察者，父结果消费由确切回执与完整分页覆盖证明；这两者均不授予恢复权限。无工具来源的真实 lead report 和来源问题通知保留各自原合同。

一个调度 job 的交接或观察失败只交回该 Step／Attempt 的原回执 owner，不先等待所有兄弟任务结束。原预约仍按用户并发设置计入容量，健康 job、就绪 Step、独立控制与 Stop 继续推进。恢复通过同一个归约器核验原事实：Queued 预约保留同一 Attempt 和冻结输入；能够证明确切原输入未准入时，writer 先围住旧 sender，再允许按原 key 投递同一 pending 输入；原 Turn 仍运行则只观察，原 Turn 已终结则读取原结果。初次读取后才提交的终态要求重读，不伪造缺结果 review。缺失原意图、真实未知效果或未证明归属仍受 review 围栏保护。核验只检查原事实，不调用业务；观察失败或进入 runner 本身均不授权重投或新建 Attempt。

父任务的异常收尾由原 Session owner 接管，其正式记录可以恢复同一收尾责任，不把 `source_continuation` 当作重新启动父任务或后台汇总的入口。子任务正式受理后具有独立生命周期；父故障不抹掉原子结果，也不默认取消孩子。只有真实原任务仍可继续且通过原控制、效果、build、预算与容量检查时，才继续原 Turn。
来源委派的子任务终态保存在原 Execution 与子 Session 链，不能由独立报告 writer 直接成为父会话的最终回答，也不自动创建隐藏结果模型 Turn。父 Turn 继续后的正常 Runtime 输出负责公开交付；Robot／Channel 使用其原输入与连接权限。无工具来源、由用户直接创建的 Execution 仍可幂等投影聚合摘要，AutoWork root 仍复用其原 lead Session，避免重复结果。查询、通知或 Session 暂时 idle 均不授权启动另一项任务。

父 Turn 后续失败或仅停止自身，不撤回已 Returned 且确切因果可验证的委派；子任务仍受原 Step、委派范围、Execution 和自身 native 准入围栏控制。真正的人类问题交接只通过已验证 `reply_to` 的递减来源链延续任务权限及结果引用，原问题终态、当前精确 binding 和 clear floor 任一不符均不继承旧范围。它不是无关新 Turn 对旧协作的通用权限。

已经消费完结果的 Execution 被删除后，其旧消费索引不成为下一次真人回答的活范围准入前提。尚待交接的已删除工作仍须通过原来源证明；父 Agent 只能读取原 `Deleted` 事件及真实持久终态，得到用户撤销、结果不可用的确切观察。该回执解除待交接义务，不声明子任务成功，不恢复普通详情／工具，不允许继续消息或重新调度。缺失、跨 owner 或损坏的删除证据仍拒绝读取。

每个 ChildAttempt 从原不可变 Snapshot 和 typed resource bindings 建立独立 Session／Runtime，不继承父 Turn 临时缩窄的 schema 呈现窗口。原受理输入及委派前已实际应用的 steering 附件、显式 Skill 选择经过确切 source／Effect／Turn／owner 核验后，进入子会话的普通输入准备；Skill 必须仍属于子 Snapshot，附件仍按其真实模型和资源权限准备。任务正文保持自己的 Step spec，原用户约束通过来源引用读取，不复制父 transcript 或把来源数据当成新授权。

明确的后续工作复用原子 Session 及经核验的冻结 binding，保留其原 canonical 历史。`message_source.command_event_id` 指向本次真实 native `ToolCallCompleted` 提案；委派归属仍是原来源，输入准备则读取发送者本次确切 accepted Turn 及该命令前已应用的 steering，不重注第一次委派的选择，也不按最新输入猜测。运行中孩子的消息从其原 StepChanged／Attempt／命令读取同一选择，完成后的续发与首次派发共用 canonical 输入准备；不得在运行中消息路径丢掉附件或显式 Skill。没有模型来源元数据的直接 UI 续发保留自己的输入，不借用旧选择。读者与 writer 共用不可变命令引用、目标、内容摘要及幂等身份的绑定证明；当前 native 代际检查只负责新命令准入。

父 Agent 同一 Turn 接收明确引导后，问题回答与录音交接读取当前工具原因果之前最后一个已应用的 typed SteeringInputs 的原始输入；没有已应用引导时才读取原 Turn 输入。尚在队列或命令之后到达的输入不能成为本次回答、展示问题或录音权限的依据。

Robot 子任务问题的通知与实际播放共用事务内的源授权核验，以原委派 Returned Effect 的真实终结事件为新通知的因果，不要求协调中的父 Turn 先结束。运行父回合须仍是原 active Turn 并通过领域准入；原已完成回合的现役 canonical 通知保留真实终态证明。Store 的原受理输入与 Effect 读者提供唯一事实校验，当前 binding、clear floor、设备、连接、确切待问 Step／Attempt 和播放 writer lease 继续约束交付；停止或过期展示不能重放。

并行容量由用户的 `max_parallel`、Participant 限制、真实 DAG 和原预约共同决定，不再因 `Full`／`ReadShell` 的工具类型把整个工作区的 Step 串行化。工具策略仍是权限上限，不能为了获得并发自动降成 ReadOnly 而撤走本来选中的能力。模型 slot 与真实资源责任分开：确切 `AwaitingChildren` 可以让出模型容量，恢复前在原 writer 内重新准入；文件、进程、Browser、Computer 和未知 Effect 的 owner 围栏仍独立生效，等待或 checkpoint 不证明它们已释放。同一 Session 的模型 Turn 仍串行，多子 Session 可以同时运行。

### 子任务重启与暂停交接

子任务的“重启任务”使用 `POST /api/agent-executions/{execution_id}/steps/{step_id}/restart` 正式命令。请求固定幂等 key、Execution／Step 版本和确切原 Attempt；受理回执与实际处理结果分别由原 `restart_requested`／`restart_resolved` 事件证明。现役 lifecycle owner 从这些事件派生未结责任，界面、连接断开及提交确认丢失不丢失原命令。没有第二张执行表或恢复账本；界面仅保存未确认的原请求草稿，不能从投影启动业务。

技术暂停保留原因、暂停与 checkpoint 身份及清理证明，通过同 writer 核验后交回原 Step review，不再折叠为普通 suspended 布尔。真人重启可通过原 native resume 接回同 Attempt／Turn；冻结的原命令、真实 User actor、原 fence／checkpoint、Stop、未知效果和容量围栏同时约束直接授权及排队后的授权。父 Agent 的控制消息不授予真人 native resume 权限。用户 Pause 保留其明确“继续”入口，恢复不增加原预算或重跑已结算效果。

需要新一轮的重启先精确停止原 Attempt，沿原清理队列取得真实终态与清理 ACK，才将同一逻辑 Step 重新准入；不调用留下永久取消围栏的 task Stop 来模拟重启，也不自动重置 DAG 下游或健康兄弟。未启动的原 Queued 预约保留同一输入及 key；并发已合法前进的原 Attempt 按其原事实确认，不能因旧观察缓存而永久冲突。未知效果、失效目标或需要明确重规划的下游留下可读的正式 blocked 结果。后来的任务 Stop／替代优先，旧命令不能复活它或转投后继。原父任务归属始终绑定逻辑 Step，重启不创建隐藏父回合。

InitialInput 是准入 outbox：同 writer 核实确切 accepted＋started、稳定 key、完整原输入及因果后即 ACK，仅删除这条已准入意图；业务结果仍由原 Turn owner 持有。后续 Steer／Stop 不再排在“等待业务结束才能 ACK”的原始输入后面。父任务首次看到确切技术暂停或新的重启失败回执时唤醒一次；已经核验可由真人重启的原暂停保留同一父 Turn 的外部控制订阅。普通观察循环不算推进者，真实重启 owner 与清理 owner 分开识别。相同暂停、domain review 投影或成功受理不重复唤模型，孩子恢复并交付后由原父任务继续消费结果与整合。没有合法外部控制或推进者的其他边界仍保持原 `decision_blocked` 安全停车。

## 生命周期与恢复

`planning`、`running`、`paused`、`waiting_input` 和终态由 Engine 的状态机与 optimistic version 共同控制。Scheduler 每次派发前在事务内复核依赖、Step 版本、活动 Attempt 与重试时间；同一 Execution 只保留一个活动调度循环，同一 Session 仍串行接收 Turn。

Execution 暂停只关闭新派发；已准入 Attempt 保留原 Session、Turn、活动 Link、预约和运行 owner，调度接管时也保持同一责任。暂停调度有原活 job、却没有本地完成观察者时应停车等待，不能对空通知流反复读库。暂停不冻结原生操作，不制造 review 或取消。已准入工作结算后可释放调度租约，聚合仍保持暂停，直到用户继续调度。恢复不新建已完成 Attempt，也不重放已执行效果；未回答问题继续保留。原 Turn 的安全停车与无损继续使用 Session 的 native pause、checkpoint 和明确 resume 合同，与调度暂停分别表达。

模型容量从同一原 Attempt、Turn 与 native owner 事实派生。确切 typed InputRequired 终态、无后继／活动 Turn／未结算效果且没有 review 围栏时，可以让出模型容量；确切 native child-wait 也可让出该 slot，配对结束后须重新准入。资源是否仍在执行由各原 owner 独立证明，不用 tool policy、WaitingInput 显示状态或缓存缺席代替。继续交接从原 Execution Event 读取同一 Attempt 的确切 Turn 引用。

AutoWork 原 Requirement claim 的显式取消在原 writer 内围住 Execution 创建、Attempt 派发与继续，
复核确切 requirement、generation、lead Session 和 owner。停止先于 Execution 创建或 queued Attempt
启动时不能继续准入。重启在原领域取消与下游聚合取消之间中断时，只凭该确切来源的持久取消事实
补交 System 聚合取消并沿原 owner 清理；查询失败、来源缺失或其他 generation 不能冒充此证据。
来源取消的整段查找、CAS 和清理由原 lifecycle 保留，HTTP 观察者离开不丢责任。
首次按持久来源选定的 Execution ID 随后固定；仅该原 ID 的版本确实前进时，才退避重试原取消 CAS。
并发已提交的终态仍采用原赢家；同版本冲突、数据库故障和未确认清理保持真实错误。
此重试不重新选取新 claim、创建 Attempt 或调用模型。`accepted` 表示源围栏已提交，
`execution_stopped` 还要求原执行停止及清理已确认，不能用前者代替后者。

迟到回执不重开终态。显式 retry、adopt、add 或 replan 均须原版本及状态围栏，已取消范围不能重开。允许的 retry 保留同一 Execution 和逻辑 Step，创建新 Attempt 与子 Session Link，保留旧结果和效果证明。来源所有权属于该 Step，因此经核验的原父任务仍可管理其当前 Attempt。已受理的指导、回复或停止固定原 Attempt 和 operation，迟到旧命令不能悄悄改投重试后的 Session。DAG 只表达任务顺序，不授予委派所有权。

核验、继续、重试与替代是不同决定。核验只读取和归约原 Attempt；继续需要确切活 Turn 与已受理输入；重试仍须原未知效果和版本校验。用已有完成 Step 替代时，先验证其 canonical 交付，再由原计划 writer 保存正式替代关系及允许的下游依赖改动。仅新建另一个成功 Step 不解除旧义务；其他已完成 Step、原结果及清理责任继续保留。启动恢复继续原租约、Attempt 与回执，缺失或未知结果均不授权自动业务重试。

已提交的问题回答、Steer 与 StopTurn 是 Attempt 的 write-ahead 效果，只由调度的原效果投递路径消费；中断对账不认领仍待投递的 Running Attempt。“无可调度步骤”的失败只按判定所用快照的 Execution 版本 CAS 提交；回答等并发写入在快照后提交时重新判定，不能把合法回答变成取消、失败或 review 阻塞。

用户可通过 `POST /api/agent-executions/{execution_id}/steps/{step_id}/cancel` 停止确切 Step，
命令固定 Step ID 与 expected Step version；`scope` 明确选择 `task`、`delegated_work` 或
`task_and_delegated`。旧 `include_delegated` 仅在 HTTP 解码边界映射到同一范围，不参与领域裁决。
局部停止允许无关兄弟结算或委派追加推进 Execution version，拒绝未来版本、目标 Step 变化与
已被替换的 Step；命令仍保存原 captured Execution version，用于精确重复确认。整组控制保留原聚合 CAS。
委派归属只读原 `PlanChanged` 的 caller Step/Attempt 和 `added_step_ids`，不沿 DAG 依赖边推断。
在原 SQLite writer 边界内写入 `StepChanged` 取消事件并封住所选范围的后续派发、重试和委派，
重复命令只确认原版本／范围。仅停止委派工作保留父任务的运行、结果和业务重试，独立的原 root
委派围栏阻止父任务及其后续 Attempt 再次派生，包括通过 replan 绕过。已完成父任务可停止仍存续的委派，保留其已提交结果；
确实依赖缺失产物的下游按依赖受阻结算，无关兄弟继续。

`delegated_work` 选中集合为空时仍可围住未来派生，但没有停止当前任务，界面必须明确呈现这一点。取消受理本身既不证明 native 已结束，也不解除未知效果。

原 Session Turn 的 canonical cancellation 只结算对应 Attempt/Step，不提升成整个 Execution 取消。
明确选择整组取消仍走原 `cancel_execution`，终止该协作全部未完成工作。
初始规划、手动重规划／调整及 Attempt 内的委派规划在原 Engine 中持有短期模型请求取消句柄，
登记后复读原持久来源。整组停止提交后只通知该版本及之前的请求；局部停止只通知所选来源的委派规划，
兄弟和后继不受影响。初始规划恢复保持同一在途 owner。取消本地模型流不构造替代计划、失败回退或新 Attempt，
也不代表远端供应商已经撤回计算；模型结果提交仍受原 SQLite CAS 与来源围栏约束。
任务取消受理、原生执行停止与资源退休分别确认；取消事件或 inactive link 本身不证明 Runtime 已释放。
Attempt 已创建、尚未 link 的 Session 也不能直接丢弃。原 owner 与创建收据、领域持久终局／替代裁决、
不存在任何历史 Link、输入、Turn 或 Effect，必须在同一个 canonical 删除 writer 中核验。
旧 owner 失去 lease 时，新 owner 可能正准备同一个 Queued Attempt；清理必须保留该预留，等待原领域裁决或真实 link。
Session 生命周期 owner 先确认接管清理责任，让 Runner 能继续上报原错误并由 scheduler 结算；
该确认不等于已经删除或业务成功。调用方退出、临时数据库故障都不丢失此责任。
启动时从原 Attempt 与 opening 发现尚未落围栏的清理；围栏之后复用 `deleting` 恢复，不新增账本或重跑业务。
迟到的完成退休仍持原 Runtime handle，只读该实例原有完整 teardown flight 的成功结果。
此证明可在 Kernel 弱缓存已按原 Arc 正确移除后复用，避免要求已退出的 owner 再次参与；
缺失、pending 或失败的 flight，以及 registry／缓存条目缺席都不构成成功证明。
这只结束原资源责任，不清除 canonical Unknown Effect、不授予重跑或后继准入，
也不处置仍由独立 owner 保留的用户页面、后台进程和合法回写。
原 cleanup owner 携带 Attempt 的原操作引用到 exact Session owner，不临时选择当前 Turn；
它独立于普通控制队列向宿主发出停止，不能让挂起的 Steer 延误用户 Stop。
清理 outbox 按原 Session、Execution、Step、Attempt 作为责任单元，只合并同一 Attempt 的重复
inactive Link。确认一次精确停止不能确认同 Session 的其他 Attempt；合法后继继续运行。
原目标从该 Attempt 的 effect 和原事件中的 invocation key 只读派生，软删除 Execution 后仍可
清理，不经由只返回现役聚合的查询，也不选择 Session 最新 Turn。目标无法证明时保留原责任和
可见错误，不能确认释放；有限 keyset 清理轮次仍越过失败 owner，照顾后续工作。

Step 的使命正式闭合时，原 writer 重新唤醒该 Step 已确认清理的旧终态 Attempt，
包括停止退避、后续 Attempt 最终结算、依赖跳过、采纳已验证产物和计划替换。
仍在等待合法重试时不触发这项通知；旧失败结果、retry 元数据与各 Attempt 的原调用身份保持不变。
计划替换的原事件保存确切 revision 与被替代 Step 集合，资源消费者据此验证用途结束。
退休读取明确区分精确原 Attempt 与实际 current Attempt，不把旧对象伪装成后继。
原 Browser 尚因活动后继而无法接受退休责任时，清理项继续保留；它只重试原对象的释放，
不重新派发业务，也不阻止无关 Runtime transport 退休。

## 活动工作与领域停止

桌面 `/scheduled` 保留计划任务视图，并提供活动工作视图。`GET /api/agent-activities`
仅聚合原 Session、Step/Attempt、Cron reservation、AutoWork claim、Remote 与资源 owner 的事实，
包括尚未 claim 为 Turn 的已接受输入、恢复／清理中的实例及 post-answer 知识回写。
尚未有 Attempt 的规划聚合使用明确的协作计划行，按钮标为停止整个协作；
它保留原聚合版本围栏，不作为单任务停止的替代。
分页和轮询不打开 Runtime，不读取 Message 投影作为上下文，也不发送、恢复或重试业务工作。
已保存的问题回答与 native 继续意图分别显示排队，等待原预约时无需重复回答。

`POST /api/agent-activities/stop` 携带确切领域目标，分流至原 owner。
任务、仅其委派工作、任务及其委派使用原 Step scope；Cron 本次运行和 AutoWork 本次 claim
不改未来调度配置；Remote 取消在原 request writer 内核对原 Operation、head cursor 与 binding，
canonical 新工作准入也消费同一取消围栏。后继变化不能让旧按钮选择新 Turn。
AutoWork 的唯一初始 root 从原计划首次落盘的 Step revision 识别，不按标题、角色或子任务
Session 推断。根任务的业务停止／整次运行仍交给原 Requirement claim owner；其局部 Step
入口只接受 `delegated_work`，保留根任务结果和 claim。后续委派子任务使用普通确切
Step/Attempt 目标，可单独停止或停止其所属委派，不提升为整次 AutoWork 取消。
只有原 root 复用 lead Session；委派子任务使用独立的 ChildAttempt Session，并保留原任务
投影及同一已准入的物理工作区。root 已完成且清理回执已确认后，只要本次运行仍有委派，
其停止委派入口仍由原 Step/Attempt 身份可达，不转移为任意后继目标。
`automation_control` 和根行的 `delegation_target` 仅投影原身份，不复制 Requirement 状态
或授予恢复权限；renderer 以此呈现真实可用的停止入口，原 retry/adopt/replan 禁令不变。
未 claim 的输入写原 `CancelledByOwner` 事实；原 claim 先赢时只解析其不变的确切目标。
知识回写可单独停止原 Operation 的 token，Runtime 资源按钮不得越过其尚存的结算责任。

全部停止先用 `GET /api/agent-activities/stop-targets` 在一次 SQLite read snapshot 中捕获有限目标，
再并发分发原领域命令。该瞬态快照不持久化为第二账本；已捕获目标和命令 key 在失败重试时保留，
不重新选择后继，也不宣称跨领域原子完成。列表区分受理、执行停止及每类资源释放；
父 Turn 已结束但应用自有后台进程仍在运行时，同样捕获原进程 owner、generation 与创建调用；
原创建 Turn 必须存在于该 SQLite 截止点。用户 Browser 工作区及真实跨 Turn SSH 上下文保留逐实例资源控制。
原生证明缺失时继续显示未确认。清理重试只唤醒原 outbox／资源退休 owner，不重跑 Agent。
Runtime、进程、Browser、SSH、知识回写和 Effect 分项观察，任一 owner 的缺席都不能证明其他域释放。

## 输出、交接与取消

正常结算、重启恢复和人工采纳共用 [canonical_output.rs](../../crates/backend/nomifun-agent-execution/src/canonical_output.rs)，在同一事务读取精确 Turn 的事件窗口与终态正文。产物必须来自该 Turn 已结算的文件或发布回执，并校验根、路径、长度与摘要；不扫描目录、读取 UI 消息或相信模型自称完成。同路径的后续变更按事件顺序决定可交付状态。

Step spec 是任务输入；完成要求交给唯一 Runtime 的 typed requirements、completion 与 delivery。调度器不从自然语言文件名、格式或数量再猜一套验收规则。`request_user_input` 总是以 InputRequired 正常闭轮交还输入，原 Session 后台进程独立保留；交接不代表 Step 完成。问题即时交付；后台输出、退出及超时都不代替用户回复，不自动开启模型或重试命令。纯机器资源观察与人工交接分开，实际回复仍通过 canonical owner 和领域准入。

活动父 Turn 可以把真实物理入口跟进作为 steering 接入，发布正常中间回复并保留原子等待。执行问题的回答读取本次确切工具因果界前已实际应用的原输入，保留该次录音的 presentation ID、socket 与受理序号；不借最初请求、尚未应用的排队输入或较晚的播放证明获得回答权限。

取消先由 canonical owner 固定精确 Operation。StopTurn write-ahead intent 保存稳定取消身份与目标，迟到 StopTurn 不能命中后继 Turn。初始输入、问题回答、StopTurn／Steer 均复用原 Attempt write-ahead 效果；常规投递由现役 TaskTracker 持有并按原队列保序。慢调用和失败只保留本项，不阻挡其他 Attempt 结算。原调用在观察者离开或调度代退出后仍由该 owner 收束；确认时复读队列，只移除确切原命令，保留并发追加。

仅有持久 pending 项不表示 Session 已接管或输入已受理：准入、commit 和回调必须持续托管到精确 Session ACK。取得 ACK 后，shutdown 才可关闭纯结果观察，仍保留原输入、outbox 与结算责任；不能取消准入 Future 或伪造终态。用户直接停止的持久围栏与宿主取消信号独立于业务 FIFO，能够抢占挂起的 Steer；迟到回执不能重新打开停止范围。原 Turn 取消只结算确切 Attempt／Step，不升级为整组取消。

工具来源的子任务结果通过原父 Turn 的协作读者返回，由父 Runtime 整合后交付；用户直接创建且无工具来源的 Execution 保留稳定 operation 的摘要投影。两者不通过隐藏用户消息触发另一条总结流水线。Gateway 管理入口复用 `nomi_delegate`、`nomi_execution_get` 和 `nomi_execution_update`；原生父 Turn 使用 `agent_status`、`agent_wait` 和 `agent_message` 控制自己的孩子。HTTP 位于 `/api/agent-executions`，模板位于 `/api/agent-execution-templates`。前端通过 Link 读取关系，按 `agentExecution.changed` sequence 去重和补拉，不保存第二份执行状态。

聊天过程只显示已接受委派与真实任务终结记录，不再附加当前任务列表、运行／等待行、动态进度或常驻控制。开始记录来自原 PlanChanged 提交的任务；它只表示已委派，不把 write-ahead ToolStarted 或 Attempt running 当作真实启动。完成来自 Step 的业务结算，单次 Attempt 结束、循环继续、重试或父 Turn 结束均不能替代。正常结算、恢复收养及采用结果在原领域事件写事务保留必要的 typed 变更引用和父 Session 游标锚点，不新增生命周期表或展示日志。停止接受和清理观察留在原控制详情，不伪装为成功完成。

展示以“操作名称 · 摘要”的紧凑行承载委派。同一 Execution、原请求及工具调用／Effect 下，相邻同类记录可收拢为一行；正文、思考、工具、不同来源和不同终态均打断合并，可信锚点与仅来源顺序也不能混用。收拢仅发生在共享前端投影，保留每条原记录及其 Attempt 身份，不写回消息或领域状态。数量表示组内已提交记录数，不表示实时完成进度或不同任务数。批量默认显示数量，展开后平铺任务名并可分别打开原详情；单条直接打开，常态不附远端箭头、模型副行或状态尾标。异步详情只应用于当前会话的最后一次查看选择；返回主对话或切换会话后，旧响应不能重新打开或替换详情。

`GET /api/conversations/{session}/delegation-records` 按 owner 和已加载 accepted input 窗口读取原 Execution 事件，包括不再由最新 linked execution 指向的历史协作。Session 与 Execution 序号各自排序，不能相互比较；原 source 决定 Turn 归属，父流锚点只决定可证位置。正文分段是原子阅读单元，落在段内的异步锚点排在该段之后，不按文字内容或时间戳伪造文本切口。后继 Turn 期间才完成的任务仍归原请求。分页固定各 Execution 原事件上界，异步完成不依赖父 Session 新增消息；重连按稳定记录身份补读。缺少历史父流锚点时仅保持可信来源分组，不按时间戳重建伪造顺序。

开始记录可打开当前 Step 的原宿主控制；完成记录锁定确切 Attempt 和该次子任务的 Session，不随最新重试替换。历史查看与当前任务操作分开，切到当前任务须显式选择。单任务、委派范围、整个 Execution 的停止仍由现役控制组件和领域 owner 处理；停止请求、执行停止和资源释放分别读取真实证明。详情通过原 Activity 投影按 Execution／Step／Attempt 三个身份精确观察，不扫描全局活动列表寻找目标，不把原 AutoWork Attempt 的清理扩大为停止整次自动运行。展开记录不启动或恢复 Agent，也不改变并发、调度、重试或 Runtime 退休。

终态详情复用原 retirement facts 解析最终继续执行记录。实时队列退休后，不退回初始 Turn，也不猜测最新 Turn；缺少或损坏的原引用保持未确认。结算事件仅在有 canonical terminal 证明时附带原操作与终态事件引用，无证明时字段缺省，不写显式 null。它们仍属于原领域结算事件，不产生第二份清理账本。

## IDMM：Session 领域监督

[IDMM](../../crates/backend/nomifun-idmm/src/lib.rs) 通过 `IdmmSessionPort` 读取有界、已验证的 Session 观察，规则先行，仅在显式配置时调用旁路模型。它不拥有 Runtime、Session、工具权限或全局备用模型，也不注册 Agent Capability。工具阶段静默只触发安全停止记录，不自动取消正在执行的工具。

`runtime_policy.idmm` 是不可变 Agent Revision 的新会话默认值；初始化重放不覆盖已有配置。会话覆盖不回写 Agent，后续 Revision 不追改现有会话；fork 继承生效策略。状态存于受限的 `agent_session.idmm.<uuidv7>` 配置命名空间，按 Session 串行。删除 Session 清理状态；删除旁路供应商清除引用并降到规则档；孤立状态由 worker 回收。

旁路输入是尽力脱敏的有界数据上下文；只接受安全选项、长度受限的短答或 halt。权限、凭据、付款和破坏性选择不能自动批准。解析失败、越界输出或模型失败只产生失败/等待记录，不把任意文本送入工作会话。

所有自动输入通过 [app IDMM port](../../crates/backend/nomifun-app/src/router/idmm.rs) 的专用 canonical owner 写入；正式 accepted 内容保存服务端作者的 `idmm_decision`、实际规则/旁路来源、当次模型、简短依据与精确问题引用。首次投递在同一 Store 事务复核问题身份、完成序列与摘要；已回答、已清空上下文或已切换 Agent 的问题不能自动回答。普通输入拒绝保留字段和伪造 `origin=idmm`。重投仍用原 exact-input 幂等规则。

等待与失败使用 `idmm/notice-recorded` 关联原问题，不创建 Turn、修改问题或产生完成证明。UI 显示开关只影响解释展开，不改变输入、模型上下文或工作授权。全局模型故障转移在创建 binding 时冻结为有界候选路由，不因 IDMM 决定暗改活动 Session。

## 实现与验证入口

- [engine.rs](../../crates/backend/nomifun-agent-execution/src/engine.rs)、[scheduler.rs](../../crates/backend/nomifun-agent-execution/src/scheduler.rs)：命令、版本与派发。
- [conversation_effect.rs](../../crates/backend/nomifun-agent-execution/src/conversation_effect.rs)、[canonical_output.rs](../../crates/backend/nomifun-agent-execution/src/canonical_output.rs)：精确取消、输出与结算。
- [agent_activities.rs](../../crates/backend/nomifun-app/src/router/agent_activities.rs)、[活动 DTO](../../crates/backend/nomifun-api-types/src/agent_activity.rs)：只读跨域发现、瞬态停止快照与原 owner 命令分流。
- [桌面活动控制 browser smoke](../../scripts/validation/agent-activities-browser.mjs)：传入本轮已编译的 `nomifun-app` lib test 可执行文件 `--backend-test` 和已安装 Playwright 的 `--playwright-module` 绝对路径。默认使用已安装 Edge；macOS 可用 `--browser-executable` 或 `NOMIFUN_CHROME_BINARY` 显式指定已安装 Chrome 的绝对可执行文件路径，不下载浏览器。脚本只在带随机 sentinel 的新临时目录启动显式 ignored `activity_browser_fixture`，使用真实 Store、HTTP 和原输入准备 owner 的屏障；1280×800 与 880×600 的全新 browser context 点击工作入口、单项停止和全部停止，分别核对受理／原 owner 收束，保存临时证据后关闭本次浏览器、Vite 与后端。报告记录所选浏览器路径或 channel 及实际版本。任何模型请求均使验收失败；它不证明供应商或其他平台原生验收。
- [IDMM service](../../crates/backend/nomifun-idmm/src/service.rs)、[detector](../../crates/backend/nomifun-idmm/src/detector.rs)：规则、旁路与故障边界。
- [IDMM e2e](../../crates/backend/nomifun-app/tests/idmm_e2e.rs)：认证接口、默认值、幂等与 canonical 来源。

执行域变更定向验证 DAG/并行、版本竞争、重复派发、暂停恢复、未知效果、迟到回执、用户交接、取消和显式重开；IDMM 变更覆盖安全选项、严格旁路输出与问题失效。运行 `bun run check:agent-session-boundary`；Session 内容改动同时运行受影响 canonical history、reset 和 consumer 测试。本地替身不证明真实供应商验收。
