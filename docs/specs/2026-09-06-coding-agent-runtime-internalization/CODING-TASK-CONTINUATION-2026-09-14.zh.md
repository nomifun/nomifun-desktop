# Coding 显式跨回合任务延续（未验证）

本切片属于 CAR-06 / CAR-07。工作分支为 `rf/agent-capability-platform-v2`，
官方 Coding 构建后缀更新为 `host2-coding-loop26`；Nomi 保持 `host11`。
仅本地源码实现及局部格式化，未构建、测试、调用 Provider 或运行外部服务，未 commit/push。

## 本轮实现

此前历史重放能向模型展示旧计划，但新回合的需求账本为空，旧约束是否保留完全依赖重新提取。
现由平台从 canonical Conversation journal 提供最近闭合回合，Coding 自行生成历史任务候选。
候选包含 exact binding、回合身份、终态、最后计划／需求及最后完成说明的去证据投影。
不增加 SessionStore、不注册第三种 engine、不改变 Agent 工作台 → revision → Session 绑定。

`resume_task` 是 Coding 的内部控制工具，不是公开挂载的 runtime 或平台能力：

1. 只有宿主提供候选时才列入工具面；无候选时不能通过调用名字获得旧任务。
2. 仅最近回合，不向前寻找“某个仍有计划”的历史回合；legacy/fork 文本回退不产生候选。
3. 检查唯一 TurnStarted、operation ID、最后且唯一 terminal，以及新旧完整 EngineBinding 一致。
4. 模型须引用当前已接受用户输入中请求续接的原文；必须单调用 batch，且尚未建立新计划或观察效果／进程。
5. 一次性导入全部已登记需求，保留原 ID／描述，以当前引用为续接来源，并记录最早原始回合／需求／引用。
6. 活动步骤清空、revision 置 1、needs_replan 置 true；先写入 PlanUpdated，再发布新控制状态。
7. 之后必须 update_plan，仍须覆盖所有当前输入。历史来源字段由引擎构造，模型不能伪造或重写。

候选不会恢复旧 ToolCall、process handle、workspace epoch、CompletionObservation、provider parent、
capability generation 或成功验证状态。最后完成说明只保留 requirement IDs、disposition、rationale、
scope-change citation，明确可能已在旧回合内失效；不保留可被引用为当前证据的工具 ID。
历史范围变更可供模型理解，但不直接成为当前权限，也不能自动激活已取消工作。

## 范围变更与预算

- 本轮新建需求仍要求引用时间晚于原输入，才能报告 scope_changed。
- 从旧回合导入的需求，其原始时间在当前回合之前，因此“继续，但不再做 X”可引用同一当前输入
  明确报告范围变更。原需求仍留在账本，变更仍须最终披露，不计为原任务完成。
- 需求继续受 32 项／24 KiB 限制，带来源的完整导入超限则整体拒绝，不静默丢项。
- 历史候选序列化上限 80 KiB；它作为独立派生上下文保留，不随普通 transcript 压缩而消失。
- 压缩前现统一检查必需 instructions／任务状态／当前输入／待观察图片预算；不足时在 summary
  请求前失败，不为了压缩丢失任务数据或发送注定无法容纳的摘要请求。

## 借鉴来源及没有宣称完成的部分

阅读本地 Codex `codex-rs/core/src/session/mod.rs::record_initial_history` 中
`InitialHistory::Resumed` / `Forked` 及 `apply_rollout_reconstruction` 的调用方式：
历史由持久化 rollout 重建，恢复设置、状态与新回合入口分别处理。本实现借鉴其分层思想，
没有直接搬运 Codex 的权限、进程恢复或存储模型；需求 ledger／resume_task 是 Nomifun 的实现。

引用匹配只是来源证明，不是独立的用户意图判定器。是否续接、历史范围解释、新输入的所有约束
是否正确提取、证据是否与需求语义相关，仍依赖模型；导入只保证已登记需求不被选择性遗漏。
旧回合从未登记的需求不会被本功能自动补全。没有历史计划或超限时，不伪装成功恢复。

本功能是用户发起的新回合任务延续，不是崩溃时旧 checkpoint 自动续跑。未知进程／MCP 效果的
隔离与清理证明要求完全保留。跨启动人工恢复、非 artifact Skill 冻结接入、其他 MCP 协议及
生态 lifecycle 仍未完成；也没有据此宣称 Coding 已全面达到 Codex 的成熟度。

## 待用户允许后执行的针对性验证

未运行以下验证：最近回合选择与 exact binding 拒绝；legacy/no-plan 回退；混合 batch 无执行；
引用来自历史／错误输入时拒绝；重复续接／效果后续接拒绝；多次续接保留最早 origin；
来源伪造拒绝且已有 ID 可省略 origin 重复；预算超限原子拒绝；sink 失败不发布计划；
同条续接输入修改旧范围、不能修改本轮同输入新需求；不恢复旧证据／进程；压缩保留候选。
