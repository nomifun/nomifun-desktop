# Agent 执行与智能决策

AgentExecution 是一次目标执行的领域聚合；AgentSession 是唯一会话、Turn、事件、内容、效果和取消权威。Execution 组合现役 Runtime，不复制模型/工具循环、会话日志或完成合同。Canonical 边界见 [Agent Session](agent-session.zh.md)。已完成重构的映射、施工顺序和旧设计只保存在 Git 历史。

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

## 策略与有界调度

四类顶层策略保持正交：

| 策略 | 作用 |
| --- | --- |
| `delegation_policy` | `disabled` / `automatic` / `prefer_parallel` |
| `plan_gate` | 自动执行计划或等待明确审批 |
| `decision_policy` | 自动决定或请求用户 |
| `adaptation_policy` | 固定计划或允许有界重试、改派、重规划 |

Step 的自由文本 role 只作展示和路由提示；独立 typed tool policy 只能收缩原 Session 权限，不能按角色文案授予工具。Attempt 保存实际生效配置。模型池与当前 lead、模板选择在同一事务验证，有限池必须包含 lead；切换 lead 不保留失配模板。

共享上限来自 [agent_execution.rs](../../crates/backend/nomifun-common/src/agent_execution.rs)：16 个模型、64 个当前 Participant、128 个当前 Step、64 并行度、4 层委派深度。历史 revision 不计入当前工作集，也不能为满足上限而删除。保存的模板同样受可执行 Participant、模型与并发上限约束。

## 生命周期与恢复

`planning`、`awaiting_approval`、`running`、`paused`、`waiting_input` 和终态由 Engine 的状态机与 optimistic version 共同控制。Scheduler 每次派发前在事务内复核依赖、Step 版本、活动 Attempt 与重试时间；同一 Execution 只保留一个活动调度循环，同一 Session 仍串行接收 Turn。

暂停撤销调度租约并停止新派发。已经准入的 Attempt 必须从 canonical owner 取得确切结果；效果未知时保留 review 围栏，不能因暂停重新派发。未回答问题保留，恢复先处理它们。取消向精确在途 Turn 传播且永久终止聚合，不生成伪造完成正文。

迟到回执不重开终态。只有带 expected version 并通过 Engine 校验的显式 retry/adopt/add 才可重开允许的已结算结果；`cancelled` 永不重开。显式 replan 也必须走同一版本及状态边界。启动恢复继续原租约、Attempt 与收据，不能重复派发或把未知结果当作可安全重试。

## 输出、交接与取消

正常结算、重启恢复和人工采纳共用 [canonical_output.rs](../../crates/backend/nomifun-agent-execution/src/canonical_output.rs)，在同一事务读取精确 Turn 的事件窗口与终态正文。产物必须来自该 Turn 已结算的文件或发布回执，并校验根、路径、长度与摘要；不扫描目录、读取 UI 消息或相信模型自称完成。同路径的后续变更按事件顺序决定可交付状态。

Step spec 是任务输入；完成要求交给唯一 Runtime 的 typed requirements、completion 与 delivery。调度器不从自然语言文件名、格式或数量再猜一套验收规则。`request_user_input` 总是以 InputRequired 正常闭轮交还输入，原 Session 后台进程独立保留；交接不代表 Step 完成。问题即时交付；后台输出、退出及超时都不代替用户回复，不自动开启模型或重试命令。纯机器资源观察与人工交接分开，实际回复仍通过 canonical owner 和领域准入。

取消先由 canonical owner 固定精确 Operation。StopTurn write-ahead intent 保存稳定取消身份与目标，迟到 StopTurn 不能命中后继 Turn。租约丢失、取消和其他任务结算必须能继续被调度循环处理。

结果通过稳定 operation 幂等投影到 lead Session，不发送隐藏用户消息触发第二次模型总结。模型入口复用 `nomi_delegate`、`nomi_execution_get` 和 `nomi_execution_update`；HTTP 位于 `/api/agent-executions`，模板位于 `/api/agent-execution-templates`。前端通过 Link 读取关系，按 `agentExecution.changed` sequence 去重和补拉，不保存第二份执行状态。

## IDMM：Session 领域监督

[IDMM](../../crates/backend/nomifun-idmm/src/lib.rs) 通过 `IdmmSessionPort` 读取有界、已验证的 Session 观察，规则先行，仅在显式配置时调用旁路模型。它不拥有 Runtime、Session、工具权限或全局备用模型，也不注册 Agent Capability。工具阶段静默只触发安全停止记录，不自动取消正在执行的工具。

`runtime_policy.idmm` 是不可变 Agent Revision 的新会话默认值；初始化重放不覆盖已有配置。会话覆盖不回写 Agent，后续 Revision 不追改现有会话；fork 继承生效策略。状态存于受限的 `agent_session.idmm.<uuidv7>` 配置命名空间，按 Session 串行。删除 Session 清理状态；删除旁路供应商清除引用并降到规则档；孤立状态由 worker 回收。

旁路输入是尽力脱敏的有界数据上下文；只接受安全选项、长度受限的短答或 halt。权限、凭据、付款和破坏性选择不能自动批准。解析失败、越界输出或模型失败只产生失败/等待记录，不把任意文本送入工作会话。

所有自动输入通过 [app IDMM port](../../crates/backend/nomifun-app/src/router/idmm.rs) 的专用 canonical owner 写入；正式 accepted 内容保存服务端作者的 `idmm_decision`、实际规则/旁路来源、当次模型、简短依据与精确问题引用。首次投递在同一 Store 事务复核问题身份、完成序列与摘要；已回答、已清空上下文或已切换 Agent 的问题不能自动回答。普通输入拒绝保留字段和伪造 `origin=idmm`。重投仍用原 exact-input 幂等规则。

等待与失败使用 `idmm/notice-recorded` 关联原问题，不创建 Turn、修改问题或产生完成证明。UI 显示开关只影响解释展开，不改变输入、模型上下文或工作授权。全局模型故障转移在创建 binding 时冻结为有界候选路由，不因 IDMM 决定暗改活动 Session。

## 实现与验证入口

- [engine.rs](../../crates/backend/nomifun-agent-execution/src/engine.rs)、[scheduler.rs](../../crates/backend/nomifun-agent-execution/src/scheduler.rs)：命令、版本与派发。
- [conversation_effect.rs](../../crates/backend/nomifun-agent-execution/src/conversation_effect.rs)、[canonical_output.rs](../../crates/backend/nomifun-agent-execution/src/canonical_output.rs)：精确取消、输出与结算。
- [IDMM service](../../crates/backend/nomifun-idmm/src/service.rs)、[detector](../../crates/backend/nomifun-idmm/src/detector.rs)：规则、旁路与故障边界。
- [IDMM e2e](../../crates/backend/nomifun-app/tests/idmm_e2e.rs)：认证接口、默认值、幂等与 canonical 来源。

执行域变更定向验证 DAG/并行、版本竞争、重复派发、暂停恢复、未知效果、迟到回执、用户交接、取消和显式重开；IDMM 变更覆盖安全选项、严格旁路输出与问题失效。运行 `bun run check:agent-session-boundary`；Session 内容改动同时运行受影响 canonical history、reset 和 consumer 测试。本地替身不证明真实供应商验收。
