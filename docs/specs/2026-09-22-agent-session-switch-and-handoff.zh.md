# 会话内 Agent 切换与高质量任务交接实施方案

> 日期：2026-09-22
>
> 状态：已实施；本地/协议验收完成，真实 live-provider 因缺少隔离凭据未运行
>
> 范围：普通本地 Nomi AgentSession、Agent binding 切换、跨 Agent 上下文分段、任务交接、AgentExecution 交付质量
> 启动入口：[纯净会话实施启动 Prompt](2026-09-22-agent-session-switch-and-handoff-start-prompt.zh.md)

## 1. 决策摘要

本方案采用以下产品语义：

- Conversation/AgentSession 是用户持续工作的上下文容器；
- Agent 是从某个 Turn 边界开始生效的版本化执行配置；
- 用户可以在已有消息的普通会话中切换 Agent；
- 切换不删除会话、不跳回 Guid、不改变既有消息；
- 新 Agent 从下一次新 Turn 准入开始生效；
- 跨 Agent 只交付任务事实、历史消息和产物引用，不继承旧 Agent 的权限、系统提示、工具调用、运行时句柄或完成证明；
- 若用户选择“交接当前任务”，系统从 canonical 事件确定性生成任务交接包，不额外调用模型编写摘要；
- 原生 Runtime replay 仍保持 exact-binding 安全边界，不允许把另一 Agent 的工具事件伪装成当前 Agent 历史。

这是一项跨前端、HTTP contract、Control Plane、AgentSession Store、Runtime history 和 AgentExecution 的改造，不能通过恢复旧 `switchPreset` 接口或只修改前端导航完成。

## 2. 当前问题与代码事实

### 2.1 当前用户行为

`ui/src/renderer/pages/conversation/components/ChatConversation.tsx` 中的会话 Agent 选择回调会把用户导航至 `/guid`。用户因此离开当前会话，无法在保留上下文的情况下增强后续工作能力。

### 2.2 当前 canonical 约束

当前 AgentSession 将以下内容绑定在 `AgentBindingValue` 中：

- exact `preset_revision_ref`；
- exact `resolved_snapshot_ref`；
- typed resource bindings；
- `binding_version`。

`crates/backend/nomifun-agent-session/src/store.rs` 已经提供：

- `replace_session_model_binding`：同一 Session 下版本化切换模型；
- `replace_session_resource_bindings`：同一 Session 下版本化变更一种资源；
- active Turn、Remote provenance 和 compare-and-swap 防护。

因此，同一 Session 的 host-validated binding transition 已有先例，但当前没有安全的完整 Agent transition。

### 2.3 当前历史恢复边界

`crates/backend/nomifun-app/src/router/engine_session_host.rs` 只允许历史 Snapshot 与当前 binding 在 Chat route 上不同。`unified_runtime_history.rs` 会拒绝另一 Agent 的指令、工具和资源范围进入原生 replay。

这项约束必须保留。正确实现不是放宽 `historical_model_binding_compatible`，而是增加显式 Agent transition 边界：

- transition 之后、当前 Agent 的 Turn 可以原生 replay；
- transition 之前的内容只能作为 data-only 消息、可信摘要或结构化 handoff 输入；
- transition 之前的工具调用、prior task、checkpoint 和 capability authority 不得重放。

### 2.4 当前已有的任务交接原料

Runtime 已持久化：

- `AgentPlan`：步骤、状态、revision 和 `needs_replan`；
- append-only `AgentTaskRequirement`：用户需求和来源引用；
- `AgentCompletionReport`：`supported / unverified / blocked / scope_changed`、证据调用和摘要；
- `AgentWorkStatus`：工作区 epoch、命令、失败工具和活跃进程；
- `AgentPatchRecoveryState`：失败 Patch 的恢复目标；
- Context compaction 和完整 Runtime event journal。

`AgentPriorTask` 已能把这些事实作为 data-only continuation candidate，但它要求相同 exact `EngineBinding`。不要削弱该检查；跨 Agent 应使用新的 handoff contract。

### 2.5 当前 AgentExecution 交付缺口

AgentExecution 已验证 Attempt 的 `output_files`，但 `scheduler::compose_brief` 主要向下游传递自然语言 `output_summary`。并行 delegation DTO 只有：

- `name`；
- `prompt`；
- `role`；
- `tool_policy`。

它没有类型化的 acceptance criteria 或 deliverables；当前 artifact contract 仍从 step prose 保守推断。这会造成上游已产出文件、下游却不知道精确产物位置，或自然语言“完成”与机器可验证交付混淆。

## 3. 目标与非目标

### 3.1 必须达到

1. 在普通本地会话页面切换 Agent，不跳转 Guid。
2. 会话 ID、标题、消息、附件和历史 Creation 卡片保持不变。
3. 新 Agent 只从下一次新 Turn 生效。
4. 当前模型兼容时保留；不兼容时明确阻止并提供恢复路径。
5. 新 Agent 的 Skills、MCP、Capability 和资源由服务端重新解析，不接受客户端自造 Snapshot 或 binding。
6. 切换前证明当前 Turn 空闲、Runtime 可安全退出、效果已结算。
7. 跨 Agent 历史不继承旧权限、旧工具状态、旧系统提示或旧完成证据。
8. “交接当前任务”使用 deterministic handoff，而不是依赖模型重新概括整段聊天。
9. UI 明确显示 Agent transition，并说明“下一条消息起生效”。
10. AgentExecution 下游能拿到验证过的产物引用，而不只有 prose summary。

### 3.2 首期明确不做

- 不支持生成过程中排队切换 Agent；
- 不支持 Remote binding Session；
- 不支持 AgentExecution Attempt 审计会话原地切换；
- 不支持 Companion、客服、创意画布等产品身份强绑定会话；
- 不在活动 AgentExecution 期间切换 lead Agent；
- 不跨 Agent 重放 checkpoint、tool call、effect、进程或 private handle；
- 不通过额外模型调用生成权威 handoff；
- 不恢复旧 ConversationService `replace_agent_preset_snapshot` 路径；
- 不建立 Conversation/AgentSession 双写兼容层；
- 不增加手机或平板布局。

## 4. 产品交互契约

### 4.1 Agent 选择

会话页继续使用现有 `GuidAgentSelector` 视觉和 Agent catalog，但其回调改为当前会话的 switch flow。

选择新 Agent 后：

1. UI 请求 switch preview；
2. preview 显示目标 Agent、模型兼容性、能力增减、继承资源和缺失资源；
3. 若存在可交接的结构化任务状态，UI 提供两种明确模式：
   - `continue_task`：交接当前任务；
   - `context_only`：仅切换 Agent，保留聊天上下文，不导入任务账本；
4. 用户确认后提交 switch command；
5. 成功后保持当前路由和 Session ID，刷新 projection；
6. transcript 插入非对话式边界：`Agent A → Agent B；下一条消息起生效`。

服务端不得猜测 handoff mode。客户端必须显式提交；若 UI 没有提供选择，则不得默认为“继续未完成工作”。

### 4.2 活跃状态

- Turn 正在运行：禁用 Agent 选择，并提示等待当前回复结束；
- Runtime 空闲但存在未结算 effect：拒绝切换；
- 存在 pending Patch recovery：首期拒绝切换并给出具体恢复原因；
- 活动 AgentExecution：拒绝切换，要求等待、取消或在 Execution 中 replan；
- 已完成 Execution 的审计和消息继续保留。

### 4.3 模型与资源恢复

- 优先用当前会话模型解析目标 Agent；
- 当前模型缺少目标 Agent 所需 technical capability 时，preview 返回 machine-readable missing features；
- UI 复用现有模型兼容性和 Model Hub 导航，不静默换模型；
- 用户显式选择兼容模型时，可以在同一个 switch command 中原子提交 Agent + model；
- 资源只按目标 Snapshot 的 required/accepted resource kind 重新解析；
- 相同 kind/id 的资源需要重新经过 owner、operation 和 target contract 校验，禁止直接复制旧 binding JSON；
- 缺少必需资源时返回具体 kind、当前选择和设置入口。

## 5. 目标架构

```text
Conversation Agent selector
  -> switch preview
  -> user chooses handoff mode
  -> Session operation write lock
  -> idle/effect/recovery/execution guards
  -> resolve target preset + model + snapshot
  -> reconcile typed resources
  -> deterministically export optional handoff payload
  -> terminate old Runtime with proof
  -> atomic binding/resource/active-set transition
  -> clear old runtime checkpoint
  -> refresh canonical projection + websocket event
  -> next Turn builds target Runtime
  -> target-native replay after boundary
     + data-only history/handoff before boundary
```

### 5.1 不采用隐藏 fork 作为首选实现

Store 的 fork contract 可以创建不同 child binding，但当前公开 fork 只允许复用父 binding，且 fork base 不复制完整 transcript。若为了 Agent 切换引入隐藏 child Session，还需要解决 sidebar 去重、逻辑 thread identity、父链消息渲染和 Channel/Cron 引用迁移。

当前 Store 已有 `binding_version`、模型切换和资源切换先例，因此首选同一 AgentSession 的 versioned binding transition。若后续产品决定 AgentSession 必须永远代表一个 immutable Agent segment，再单独建立 stable ConversationThread 聚合；本任务不提前引入该高成本抽象。

### 5.2 AgentHandoffEnvelopeV1

建议新增 bounded、deny-unknown-fields 的服务端类型：

```text
AgentHandoffEnvelopeV1
  schema_version
  source_agent_session_id
  source_turn_operation_id
  source_through_seq
  source_binding_ref
  target_binding_ref
  mode: continue_task | context_only
  requirements[]
  last_plan?
  historical_completion_account?
  verified_artifacts[]
  unresolved_items[]
  warnings[]
```

约束：

- 最大序列化大小建议沿用 `AgentPriorTask` 的 80 KiB 量级；
- 只从 latest exact closed Turn 导出，不允许任意挑选旧 Turn；
- requirements 保留原始 turn/message provenance；
- completion account 明确标为 historical model assessment；
- evidence call ID 只作来源索引，不能成为目标 Agent 的当前证据；
- artifact 必须来自 exact delivered Turn 的 canonical projection；
- 文件路径/引用是待重读的数据，不是内容 digest 或当前存在性证明；
- 不携带 tool arguments/results、system prompt、capability grant、connection、credential、process、browser handle 或 runtime checkpoint；
- `context_only` 不导出任务账本，只记录 transition identity。

Handoff payload 使用现有 `agent_payloads`/Session payload 机制持久化，由 transition event 引用，不新增平行存储或第二份 transcript。

### 5.3 任务 provenance

当前 `AgentInputCitation` 的 input index 是 turn-local 的，不能伪造成目标 Agent 的当前输入来源。实施时必须选择以下安全方式之一，并用测试冻结：

1. 为 handoff obligation 新增稳定的 `SessionControlRef` provenance，引用用户确认的 transition command；或
2. 将 handoff obligations 保持为独立候选，在目标首轮显式接受后，使用该首轮真实用户输入建立新的当前 requirement，并保留旧 requirement origin。

禁止生成一条用户没有输入过的虚假聊天消息来满足 citation 检查。首期若不完成 typed provenance，则 `continue_task` 只能作为 data-only 提示，不能宣称已把旧 requirements 纳入目标 Agent 的 completion gate。

### 5.4 Runtime history 分段

保持 `historical_model_binding_compatible` 的现有严格语义。

调整 `unified_runtime_history` 时采用以下规则：

- 当前 Snapshot 和 model-only compatible Snapshot：允许现有原生 replay；
- 遇到由 canonical `session/agent-binding-changed` event 证明的旧 Agent 边界：停止 native event replay；
- 边界之前使用 `project_messages` 的 data-only 用户/助手文本、合法 compaction summary 和 HandoffEnvelope；
- 任意没有合法 transition event 的 Snapshot mismatch：继续 fail closed；
- 旧 tool calls、prior task、CompletionObservation、PatchRecovery、effect state 不进入目标 Agent 当前 replay；
- 目标 Agent 完成第一轮并产生自己的计划后，后续 Turn 可恢复同 binding 的原生 replay/continuation。

### 5.5 Patch recovery 与效果

`runtime_patch_recovery` 当前要求 exact Snapshot。首期采用 fail-closed：

- latest permanent recovery state 有 pending target 时，preview/apply 均拒绝；
- latest state 已明确清空时允许切换，但不得把旧 Snapshot state 注入目标 Runtime；
- switch 前调用 hosted effect settlement 检查；
- Runtime teardown 必须返回进程树退出证明；
- teardown 成功但 DB transition 失败时，旧 binding 保持不变，下一 Turn 重建旧 Runtime，不能留下半切换状态。

后续若要允许新 Agent 接管 Patch recovery，必须单独设计跨 binding recovery transition，并验证目标 Agent 拥有同一 workspace 与必要 read/patch capability；不纳入首期。

### 5.6 Active capability generation

完整 Agent 切换会改变 active capability set，不能继续使用旧 Session 的最后一组 active IDs。

Store transition 必须：

- 计算目标 Snapshot 的 initial active contribution capabilities；
- 追加下一代 `capability/active-set-committed`；
- 保持 generation 单调递增；
- 同一事务更新 binding、resource projection 和 active-set event。

当前 `EngineKernelSession` 使用 `SessionCapabilityState::new(&compiled)` 从 generation 0 初始化。实现 Agent transition 时需要新增从 canonical committed state 初始化的路径，例如 `from_committed(compiled, generation, active_ids)`，并验证 active IDs 是目标 compiled Snapshot 的子集。不能让 Store 显示 generation N、Runtime 却从 0 发出 invocation。

## 6. API 与事件合同

### 6.1 Preview

建议新增：

```http
POST /api/agent-sessions/{agent_session_id}/agent-switch/preview
```

请求：

```json
{
  "selection": {
    "kind": "preset",
    "preset_id": "0190..."
  },
  "model": {
    "provider_id": "0190...",
    "model": "optional-explicit-model"
  }
}
```

官方模板使用：

```json
{
  "selection": {
    "kind": "template",
    "template_key": "assistant.general"
  }
}
```

响应至少包含：

- current/target Agent identity；
- current/target binding version and Snapshot refs；
- model preserved/compatible/missing features；
- retained/dropped/missing resources；
- gained/lost capabilities；
- detected handoff availability；
- blockers；
- `can_apply`。

Preview 是只读提示，不产生锁定权威；apply 必须重新解析和校验。

### 6.2 Apply

建议新增：

```http
PUT /api/agent-sessions/{agent_session_id}/agent
Idempotency-Key: <uuidv7>
```

请求：

```json
{
  "selection": {
    "kind": "preset",
    "preset_id": "0190..."
  },
  "handoff_mode": "continue_task",
  "expected_binding_version": 4,
  "model": null
}
```

响应返回更新后的 canonical Conversation projection，以及：

- transition ID；
- previous/current Agent labels；
- new binding version；
- `effective_from: "next_turn"`；
- handoff summary counts；
- warnings。

### 6.3 推荐错误码

- `AGENT_SESSION_TURN_ACTIVE`
- `AGENT_SESSION_AGENT_IS_REMOTE_FROZEN`
- `AGENT_EXECUTION_ATTEMPT_READ_ONLY`
- `AGENT_EXECUTION_ACTIVE`
- `AGENT_SESSION_EFFECTS_UNSETTLED`
- `AGENT_SESSION_HANDOFF_RECOVERY_PENDING`
- `AGENT_SESSION_MODEL_INCOMPATIBLE`
- `AGENT_SESSION_RESOURCE_REQUIRED`
- `AGENT_SESSION_BINDING_CHANGED`
- `AGENT_SESSION_AGENT_SWITCH_UNSUPPORTED`

错误 details 应提供稳定字段，例如 missing capability/resource kind、active execution ID、expected/actual binding version 和可导航的配置 section。不要把后端原始错误字符串当成 UI 分类协议。

### 6.4 Canonical events

新增持久事件：

```text
session/agent-binding-changed v1
```

payload 至少包含：

- transition ID；
- previous/next preset revision ref；
- previous/next Snapshot ref；
- previous/next binding version；
- handoff mode；
- optional handoff payload ref/digest；
- effective-after sequence。

继续使用现有：

- `runtime/binding-discarded` 清除 checkpoint projection；
- `capability/active-set-committed` 提交新的 active set。

Event registry、generated contract、projector 和 replay tests 必须同步。建议发布用户事件：

```text
agentSession.agentChanged
```

Renderer 收到后刷新 Conversation cache、Agent label、capability controls、knowledge/creation availability 和模型状态。

## 7. Store 原子命令

新增窄方法，建议命名：

```text
replace_session_agent_binding(...)
```

Store 不负责解析 Preset、模型或资源；调用者提交已经由 host/control plane 验证的 expected/replacement binding、active IDs 和 handoff payload。

一个 DB transaction 中必须完成：

1. owner/live/remote 校验；
2. expected binding JSON CAS；
3. active Turn/head 校验；
4. binding version 恰好 `+1`；
5. replacement revision/Snapshot 确实不同；
6. 更新 `agent_sessions.agent_binding_json`；
7. 原子替换 `agent_session_resources`；
8. 写入 handoff payload（若有）；
9. 追加 `session/agent-binding-changed`；
10. 追加 `runtime/binding-discarded`；
11. 追加下一代 `capability/active-set-committed`；
12. 更新 head projection 并提交。

任何步骤失败都不得留下部分 resource、binding 或 active-set 更新。

## 8. Control Plane 与资源解析

新增“为已有 Session 解析完整目标 Agent binding”的 host-owned service：

1. template selection 先 materialize/reuse saved Preset；
2. 用当前或显式选择的 Chat model 编译 exact revision/Snapshot；
3. 通过 `official_runtime.validate_agent` 验证 executor availability；
4. 从旧 binding 中提取候选资源 selection，而不是复制 authority；
5. 只保留目标 Snapshot 接受的 resource kind；
6. 重新通过 `NomiCoreResourceBindingResolverRegistry` 解析 owner/operation/parameters；
7. 缺失 target required kind 时返回 machine-readable blocker；
8. 设置 `replacement.binding_version = current + 1`；
9. 返回 replacement、target snapshot、initial active capabilities 和 preview diff。

可以复用 `NomiCoreProductAgentResolver` 现有“按目标 required resource kind 继承 selection 后重新解析”的思路，但普通 Conversation 不能写入 product selection 表，也不能使用 product target ownership 作为 Session switch authority。

## 9. AgentExecution 高性价比改进

本任务同时交付一个独立、低风险的 quick win：

### 9.1 下游 brief 携带 verified artifacts

`compose_brief` 的每个 upstream result 增加：

- step ID/title/status；
- latest output summary；
- verified `output_files`；
- blocked/waiting/error reason（若适用）；
- 明确声明路径是历史交付引用，目标 Agent 必须按权限重新读取。

不得把 assistant prose 中出现的路径当作 artifact；只使用 Attempt settlement 保存的 verified files。

### 9.2 第二阶段 typed deliverables

在共享 `AgentDelegationTask` 中增加可选字段，并保持旧调用兼容：

```text
acceptance_criteria[]
deliverables[]
```

建议 deliverable 包含 kind、format/category、minimum_count 和 description。它应持久化为 ExecutionStep 的 typed output contract，并逐步替代 `artifact_contract.rs` 的自然语言推断。首期 quick win 不要求完成 schema 改造，但最终高质量交付目标不能长期依赖 prose heuristic。

## 10. 分阶段实施任务

### ASH-00：基线与回归锁定

- 检查分支、工作树、现有用户改动；
- 阅读本方案及相关 canonical Session/Runtime/Execution 代码；
- 为当前“选择 Agent 会跳 Guid”增加或调整结构/交互测试，先证明现状；
- 记录当前 model switch、knowledge switch、history replay 和 patch recovery 基线。

完成条件：没有生产改动，已明确写集、测试入口和当前失败/通过基线。

### ASH-01：类型、API 与事件合同

- 增加 preview/apply DTO；
- 增加 `AgentHandoffEnvelopeV1`；
- 增加 `session/agent-binding-changed` registry/schema/generated contract；
- 增加 typed error details；
- 合同测试覆盖 deny unknown fields、大小预算、ID 和 digest。

### ASH-02：Store 原子 transition

- 实现 `replace_session_agent_binding`；
- 同事务替换 resources、handoff payload、binding、checkpoint projection 和 active set；
- 覆盖 CAS、Remote、active Turn、rollback、generation、资源 FK 和 replay idempotency；
- 不修改 metadata PATCH 的窄边界。

### ASH-03：Control Plane 与 switch preview

- 解析 preset/template；
- 保留兼容模型或报告 missing features；
- 重新解析可继承资源；
- 计算 capability/resource diff；
- 验证 target Runtime；
- 检测 active Execution、unsettled effects 和 pending recovery。

### ASH-04：Runtime handoff 与历史分段

- 从 latest exact closed Turn 导出 bounded handoff；
- 增加合法 transition boundary 检查；
- 当前 Agent segment 继续 native replay；
- 旧 Agent segment 降级为 data-only messages/handoff；
- 无 transition 的 Snapshot mismatch 仍失败；
- 新 Runtime 从 canonical active generation 初始化；
- 不跨 binding 导入 `AgentPriorTask` 或 PatchRecovery。

若 typed handoff provenance 尚未完成，必须在 API/UI 中把 `continue_task` 标为 data-only handoff，不得宣称 completion gate 已继承旧 requirements。

### ASH-05：Router 与生命周期

- 加 Session operation write lock；
- apply 时重新校验 preview facts；
- Runtime teardown with proof；
- effect settlement；
- Store transition；
- projection refresh 和 websocket event；
- teardown 成功、transition 失败的恢复测试。

### ASH-06：Renderer 交互

- `ChatConversation` 不再导航 Guid；
- 复用 Agent catalog 和模型兼容组件；
- 增加 preview/confirm、handoff mode 和 capability/resource diff；
- 活跃 Turn/Execution 时明确 disabled reason；
- 成功后显示 transcript boundary 和 toast；
- 错误保留当前选择、输入草稿和 Creation draft；
- 更新中英文 locale 与 parity tests；
- 保持最小 880x600 desktop contract，不增加移动端布局。

### ASH-07：AgentExecution verified artifact quick win

- `compose_brief` 使用 latest settled Attempt 的 verified `output_files`；
- synthesis 和普通 downstream step 均覆盖；
- 测试证明 prose path 不会被当作 verified artifact；
- UI step inspector 继续展示完整输出，不把 graph summary 当权威交付。

### ASH-08：端到端验收与文档收口

- targeted Rust/UI tests；
- contract、type、build 和 desktop boundary；
- protocol E2E；
- 在可用凭据下执行 live-provider acceptance；
- 更新本方案状态和实际实现链接；
- 未进行的真实验证必须明确报告。

## 11. 关键测试矩阵

### 11.1 Store/contract

- expected binding version 正确时切换成功并 `+1`；
- stale expected binding 冲突且零部分写入；
- active Turn、Remote、foreign owner 拒绝；
- resource replacement 和 binding JSON 原子一致；
- active generation 单调且 IDs 属于 target Snapshot；
- event replay/idempotency 返回同一 transition；
- handoff payload digest/size/unknown field 校验；
- rollback 后旧 Session 可继续启动。

### 11.2 Runtime/history

- model-only 历史继续 native replay；
- legal Agent transition 后旧 segment 不 native replay；
- 旧用户/助手文本仍进入 bounded data-only context；
- 旧 tool/effect/system prompt/prior task 不进入新 Agent；
- 无 transition event 的 mismatch 失败；
- target 第一 Turn 后，同 target 的下一 Turn 可正常 continuation；
- pending Patch recovery 阻止切换；
- cleared recovery 不会污染 target Runtime。

### 11.3 API/UI

- 同一 Conversation ID、名称和旧消息保持；
- selector 不导航 `/guid`；
- 下一 Turn 使用 target preset/Snapshot；
- current model compatible 时保留；
- incompatible model、missing resource 给出可恢复提示；
- draft、附件和 creation state 在失败时不丢失；
- transition boundary 显示 old/new Agent；
- active Turn/Execution 的 disabled reason 准确；
- Remote/Attempt/product-bound Session 不显示或不启用切换。

### 11.4 AgentExecution

- upstream verified files 出现在 downstream brief；
- unrelated older Attempt 文件不进入 brief；
- assistant prose path 不进入 verified artifacts；
- synthesis 仍等待所有 upstream dependencies；
- final projection 仍实时回到 lead Conversation。

## 12. 验证命令建议

根据实际写集选择最小检查，建议至少覆盖：

```bash
cargo test -p nomifun-agent-session --lib
cargo test -p nomifun-agent-control-plane --lib
cargo test -p nomifun-agent-runtime --lib
cargo test -p nomifun-agent-execution --lib
cargo test -p nomifun-app --test nomi_core_route_gap
cargo fmt --all -- --check

cd ui
bun test src/renderer/pages/conversation
cd ..

bun run typecheck
bun run build:ui
bun run check:desktop-ui-boundary
bun run check:agent-vocabulary
bun run check:nomi-core-live-provider
git diff --check
```

最终改动跨 Store、Runtime、API 和 UI，targeted checks 全部通过后应运行 `bun run check`；若存在与任务无关的既有失败，必须给出精确命令、错误和归属，不能笼统称通过。

真实 provider 验收使用：

```bash
bun run test:nomi-core-live-provider
```

只有在凭据隔离条件满足时运行。compile-only、mock、协议 fixture 或 UI 截图不能替代真实模型验收。

## 13. 真实验收场景

至少增加一个 live smoke 场景：

1. Agent A 在普通本地 Session 中接收多约束任务；
2. Agent A 建立 requirements/plan，产生一个可验证 artifact，并保留一个明确 unverified 项；
3. Session 回到 idle；
4. 用户选择 Agent B，并选择 `continue_task`；
5. Session ID、标题和旧消息不变；
6. target binding/version/Snapshot 生效；
7. Agent B 下一 Turn 能读取原始任务、历史完成度和 verified artifact reference；
8. Agent B 明确把旧 evidence 视为历史，并重新检查当前 workspace；
9. Agent B 不具备 Agent A 已移除的能力，也无法重放旧 tool call；
10. 最终回复正确披露仍未验证的事项。

同时保留负例：pending Patch recovery、active Execution 和 missing target resource 均不能切换。

## 14. 建议修改地图

预计重点文件/目录：

- `ui/src/renderer/pages/conversation/components/ChatConversation.tsx`
- `ui/src/renderer/pages/guid/components/GuidAgentSelector.tsx`
- `ui/src/common/adapter/ipcBridge.ts`
- `ui/src/common/types/agentPlatform/`
- `ui/src/renderer/services/i18n/locales/{zh-CN,en-US}/`
- `crates/backend/nomifun-api-types/src/agent_platform.rs`
- `crates/backend/nomifun-agent-contracts/contracts/events/`
- `crates/backend/nomifun-agent-session/src/store.rs`
- `crates/backend/nomifun-agent-session/src/projector.rs`
- `crates/backend/nomifun-agent-control-plane/src/service.rs`
- `crates/backend/nomifun-app/src/router/nomi_core_session.rs`
- `crates/backend/nomifun-app/src/router/engine_session_host.rs`
- `crates/backend/nomifun-app/src/router/unified_runtime_history.rs`
- `crates/backend/nomifun-app/src/router/runtime_patch_recovery.rs`
- `crates/backend/nomifun-app/src/router/engine_kernel_session.rs`
- `crates/backend/nomifun-agent-runtime/src/task_continuation.rs`（参考，不削弱 exact-binding 约束）
- `crates/backend/nomifun-agent-execution/src/scheduler.rs`
- `crates/agent/nomi-types/src/agent.rs`（typed deliverables 第二阶段）

实际实施前必须重新搜索调用链，不能把此列表当成完整写集。

## 15. 完成定义

只有同时满足以下条件，才可以宣布本修复完成：

- 会话页 Agent 切换不再导航 Guid；
- 同一 Session 的历史和 UI 上下文保持；
- target Agent 从下一 Turn 生效；
- binding/resource/active-set transition 原子且可审计；
- old Agent runtime/effect/tool authority 不越界；
- handoff facts 有 bounded、deterministic、data-only contract；
- pending recovery、active execution、Remote/Attempt 等边界 fail closed；
- AgentExecution verified artifact quick win 完成；
- targeted tests、UI/type/build/desktop boundary、contract checks 通过；
- real-provider 场景在可用条件下验证，或明确列为未验证；
- 没有覆盖用户已有工作树改动；
- 未经明确要求不 commit、不 push。

## 16. 2026-09-22 实施收口

本方案已按同一 canonical AgentSession 的 versioned binding transition 落地，没有恢复 legacy Conversation `switchPreset` / `replace_agent_preset_snapshot`，也没有增加 Conversation 双写或第二套 Runtime supervisor。

实际实施入口：

- HTTP DTO：`crates/backend/nomifun-api-types/src/agent_platform.rs`；
- handoff / transition 合同：`crates/backend/nomifun-agent-contracts/src/session.rs`、`contracts/events/session-event-registry.json`；
- Store 原子 CAS：`crates/backend/nomifun-agent-session/src/store.rs`；
- target Agent/model/resource 解析和 Router guards：`crates/backend/nomifun-agent-control-plane/src/service.rs`、`crates/backend/nomifun-app/src/router/nomi_core_session.rs`；
- Runtime boundary / committed generation：`engine_history.rs`、`engine_session_host.rs`、`engine_kernel_session.rs`、`unified_runtime_host.rs`、`nomifun-agent-kernel/src/session_capabilities.rs`、`nomifun-engine-core/src/kernel.rs`；
- 会话内交互与持久 transition marker：`ChatConversation.tsx`、`AgentSwitchDialog.tsx`、`MessageTips.tsx`；
- AgentExecution verified artifact brief：`crates/backend/nomifun-agent-execution/src/scheduler.rs`。

已冻结的首期语义：

- `continue_task` 只导入 bounded、deterministic、data-only handoff；`completion_gate_inherited=false`，不会伪造目标 Turn 的用户 citation；
- 目标 Agent 必须从真实的下一条用户消息重新建立 requirements，并重新读取/验证 workspace 与 artifact references；
- canonical transition event 是旧 Agent 历史降级为 data-only 的必要证明；无合法 event 的 Snapshot mismatch 继续 fail closed；
- model-only/resource-only 的后续窄 binding version 仍保留已经证明的 Agent boundary；
- active Turn、Remote、Attempt、active AgentExecution、unsettled effects、pending Patch recovery 和产品身份强绑定 Session 均 fail closed。

本地验证结果：

- AgentSession Store 44/44；Agent Runtime 50/50；Agent Kernel 62/62；Engine Core 17/17；AgentExecution 110/110；
- `nomi_core_route_gap` 33/33，其中真实本地 HTTP → Store → Runtime → WireMock provider 场景验证同一 Session、model-only replay、Agent transition、`continue_task`、下一 Turn target Snapshot、旧段 data-only、无旧 tool role、transition marker 和 idempotent replay；
- Conversation Renderer 849/849；TypeScript、UI build、desktop 880×600 boundary、i18n、contract generator 和 live-provider compile gate 通过；
- `bun run test:nomi-core-live-provider` 返回 `LIVE_CREDENTIAL_MISSING`，因此真实外部 provider 场景明确未验证，不能以 WireMock/compile-only 替代；
- 仓库既有且与本任务无关的 `AgentControlPlane` creative direct-creation 单测与 Agent vocabulary 检查仍失败，交付报告应保留精确证据，不应修改无关产品规则制造全绿。
