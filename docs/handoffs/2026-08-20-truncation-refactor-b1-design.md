# B1 — 可恢复轮次（Resumable Round）：修正后设计

日期：2026-08-20
分支：`fix/truncation-not-success`
设计基线 HEAD：`122646f8`（C1 + A2b 已落地）

本文取代 `2026-08-20-truncation-refactor-resumable-round.md`。那份文档的**每一个行号都已失效**（C1 改动了
`llm.rs`、四个 provider、`engine/mod.rs`、`local_agent_invocation.rs`、宿主 `agent.rs`），且偏移量符号不一致
（常量区 +13、计数器 −29、循环体 +22），没有任何统一位移可用。本文所有行号均在 `122646f8` 上逐个重新读取。

---

## 0. 相对旧设计的实质性改动（不只是行号）

| 旧设计 | 重新测绘发现的事实 | 本文的处理 |
|---|---|---|
| 持久化 `nomi.round.ledger` 到 `host_context`，带 sha256 digest fence、reap 规则、`rewind_last_turn` / `record_host_text_turn` 交互 | B1 的自动重启完全发生在**一次** `execute_turn_inner` 调用内部。ledger 只用于渲染同一次调用中下一 pass 的 system section，**不需要跨进程存活**。持久化会引入 `host_context` 前缀契约、两处 checkpoint 快照过滤、digest fence、5 个 `Ok` + 28 个 `Err` 出口的 reap 规则 —— 全部为一个尚不存在的消费者（D1）服务 | **ledger 降为 `execute_turn_inner` 的栈内局部量**。删除 sha2 依赖、digest fence、`host_context` 改动、reap 规则、rewind 交互。D1 需要跨进程恢复时再加持久化，类型已就位 |
| `truncation_continuation_prompt` 「没有单元测试，可直接删」 | 函数本身无直接单测，但它所在的分支被**两个 tokio 测试钉住**：`send_message_auto_continues_after_max_tokens_before_finish`（`agent.rs:4905-4952`，断言 `provider.calls() == 2`、`finish_reason == EndTurn`）和 `send_message_does_not_auto_continue_after_max_turns`（`agent.rs:6079-6140`）。且 `ScriptedProvider::stream` 在未脚本化的 pass 上 **panic**（`agent.rs:4204`） | 第一个测试**重写**（改为断言 prose-only 截断不重启：`calls() == 1`、`finish_reason == MaxTokens`），第二个保持绿。新增一个可恢复形状的测试 |
| §5 判决 `rounds > 1 && effects_ok == 0` | 除了已知的 plan-mode / model-only 误判，测绘发现**第三类误判**：本仓库的 `ToolCategory::Info` 意为「无审批门」而非「无副作用」。companion 记忆写入（`companion_tools.rs:236-239`）、knowledge / skill 工具都刻意是 `Info`。把 Info 等同于「什么都没发生」是错的。另外 `ToolCategory::Mcp` 在生产中已死（`nomi-mcp/src/tool_proxy.rs:126-130` 只返回 Info/Exec） | 判决保留，但改为**四重机器证据合取**，并且 `retryable = true`（见 §6）。不再特判 `Mcp` |
| blocker 10 只需「不要进 `process_final_text`」 | 更严重：`process_final_text` 会**执行**截断草稿里的 cron 命令（`response_middleware.rs:324-326`），而 cron 正则是无闭合标签的裸匹配（`response_middleware.rs:47`）。半句话草稿里出现 `[CRON_LIST]` 就会触发 | 同时修两处：收窄 `failed_terminal`（原则修复），并让 `durable_completion` 对失败单调（防御未来任何第二轮路径） |
| `ContentBlock` 可用于值比较 | `ContentBlock` / `Message` **只** derive `Debug, Clone, Serialize, Deserialize`（`message.rs:10`、`:64`），全仓无手写 `PartialEq` | 锚点不能靠值相等定位；`RoundState` 不 derive `PartialEq`；测试不能对 `Message` 用 `assert_eq!` |
| `append_system_resource_context` 是 `AgentEngine` 的方法 | 它是模块级自由函数（`engine/mod.rs:252`） | 新增的同级函数也是自由函数，输入全部走参数 |
| `limit` 与 `let mut turn` 同处循环外 | `limit` 在 `loop {`（`:1331`）**内部**的 `:1335`，每轮重算 | 重启 hook 不读也不改 `limit`；不递增 `turn` |

---

## 1. Blocker 1 — 不依赖 `EditableTurnCheckpoint.start_len` 的回卷锚点

两个独立部件，都不含偏移量：

**(a) 丢弃截断草稿。** `engine/mod.rs:1909-1910` 引擎自己 push 了
`Message::now(Role::Assistant, assistant_content)`。该 push 是**无条件**的，位于
`if tool_calls.is_empty() {`（`:1912`）之前，两者之间没有任何语句改动 `self.messages`。因此在 `:1912` 分支的
**第一条语句**执行 `self.messages.pop()` 恰好移除本 pass 的草稿 —— 与压缩、历史长度、之前的轮次全部无关。

`supersede_written_draft`（`:1901`）不可能改动它：该函数只在同一轮产生文件写入时生效，而
`StopReason::MaxTokens` 携带非空 `tool_calls` 在 `:1755-1758` 已是硬协议错误，
在 `:1909` 之前就返回 `Err`。

**(b) requirement。** 在 `execute_turn_inner` 中、`self.messages.push(Message::now(Role::User, user_content))`
（`:1317`）**之前**取 `let round_requirement: Vec<ContentBlock> = user_content.clone();`。这是拥有整个循环的那个
函数的栈内局部量。

**为什么它能活过三个危险：**

| 危险 | 存活原因 |
|---|---|
| `run_compaction()`（定义 `:2421`，调用 `:1364`） | 它只动 `self.messages` / `self.editable_turn` / `self.compact_state`。`round_requirement` 是局部量，ledger 也是局部量。pop 的目标是**本 pass** 在压缩之后重新建立的 `self.messages.last()`。压缩后第 2 轮的 transcript 是 `[boundary, summary, requirement]` —— 正确且合意。（`auto::autocompact` 只返回 boundary + summary 两条消息，根用户消息**物理消失**，这正是 `start_len` 方案失效的根因。） |
| 轮内错误回卷（`:1187-1197`） | `self.messages = safe_messages` 只恢复消息。重启 hook 在 pop + re-push 之后自己重设 `*safe_messages = self.messages.clone()`，所以第 2 轮的 provider 错误回卷到第 2 轮地板，绝不回到第 1 轮散文。`safe_messages` 是 wrapper（`:1161`）的局部量 —— 这也是循环不能搬到宿主的独立理由。 |
| 进程重启 | 与 B1 无关：ledger 不持久化。transcript 由 `save_session()` 持久化；requirement 的持久权威是宿主的 `turn_delivery_request_payload`，重发时重建 `image_blocks`。 |

---

## 2. Blocker 2 — 高度：循环留在引擎，且不消耗 turn

重启 hook 是 `:1912` `if tool_calls.is_empty() {` 块内的**第一条语句**，位于
`self.stagnation_guard.reset()`（`:1913`）之前。

放在 `:1913` 之前有两个理由：截断 pass 不是完成的助手响应，现存三个 hook（steering `:1929`、
goal `:1944`、spec-recheck `:1959`）都假定它是；并且 stagnation guard 不应被一次截断 pass 重置。

**这一轮不消耗 turn。** `apply_model_only_ceiling` 对每个非 owner runtime 设
`overrides.max_turns = Some(1)`（`factory/nomi.rs:49`，调用 `:138`），于是 `limit == 1`（`:1335`），
现存每个 hook 的 `turn + 1 < limit` 门都是 `1 < 1` → false。因此重启 hook **不**读 `limit`、
**不**递增 `turn`，只递增自己的 `attempt` 并 `continue`。后果：

- model-only 会话仍得到 3 个 pass（`MAX_ROUND_ATTEMPTS = 3`），不是 0 —— 无退化；
- 乘法 bug 结构性消失：宿主不再重入 `execute_turn_inner`，`let mut turn: usize = 0`（`:1324`）永不重置。
  旧路径两次宿主续跑允许 `3 × max_turns` 个 provider pass；宿主自己的注释（`agent.rs:2074-2076`）
  承认了这个机制；
- 终止条件只有 `attempt < MAX_ROUND_ATTEMPTS`，`attempt` 是 `execute_turn_inner` 的单调局部量，
  所以每个 turn 最多 2 次重启，与 `max_turns` 无关；
- `stagnation_guard`、`tool_retry_tracker`、`routed_tool_calls_seen`、`ToolEfficiencyStats`
  以及 `*_nudged` 一次性标志跨轮继续计数，不再静默重置；
- `total_usage` 每 pass 累加（`:1828` 区域），账单正确。

```rust
/// 对同一个已接受 requirement 的总尝试次数（含第一次）。3 保持今天的信封
/// （MAX_TRUNCATION_AUTO_CONTINUES = 2 → 3 个 pass → 观测到的
/// output_tokens = 24576 = 3 × 8192），同时让三个 pass 都有用。
pub const MAX_ROUND_ATTEMPTS: usize = 3;
```

---

## 3. Blocker 3 / 4 — 多模态保留，steering 永不删除

**Blocker 3：** requirement 是 `Vec<ContentBlock>`，原样 re-push，`Image` 块包含在内。
`execute_turn_inner` 明确只接受 `Text` / `Image`（`:1260-1268` 区域校验），两者都是定长字段 struct。
为保证线上每张图只有一份活体，hook 先调用既有且已测的
`self.redact_user_images_since(0)`（`:2381` 区域），把历史用户 `Image` 块替换为
`USER_IMAGE_HISTORY_PLACEHOLDER`。`abort_current_turn` 已用同一调用与同一理由。

**Blocker 4：** 机制**只移除一条助手消息，绝不移除用户消息**，所以：

- 每一条已从 inbox `drain(..)` 排空并已展示的 steer 都结构性地留在 transcript 里；
- hook 额外自己调用 `self.drain_steering()`，把截断 pass 期间到达的 steer 作为尾随
  `Role::User` 文本消息追加在 re-push 的 requirement **之后**，于下一 pass 立即送达。
  这里 drain 无条件安全（不同于 `:1929` 需要 `turn + 1 < limit`），因为 hook 不递增 `turn`，
  后面必定还有一个 provider pass；
- hook 末尾的 `save_session()` 在下一个 provider await 之前落盘。

连续两条 `Role::User` 是已支持的形状：`auto::autocompact` 返回两条连续用户消息，
steering hook 连推 N 条（`:1934-1938`），Anthropic 家族在 `compat.merge_same_role()` /
`ensure_alternation()`（`anthropic_shared.rs:447`）折叠它们。

---

## 4. Blocker 5 — OpenAI `length` 之后的迟到 tool fragment：安全累加，绝不执行

**决定：安全累加。** 不是「希望安全」，而是结构可证。

今天 `openai.rs` 在两处销毁被截断 tool call 的身份：`:1801`（`"length"` arm 的
`state.tool_calls.clear()`）和 `:730`（`drain_terminal_events` 的 `MaxTokens | Refusal` 共享 arm）。
而 `:1609-1618` 有一个专门的 guard：`length` 之后再收到非空 `tool_calls` 就 `poison`。

**无执行不变式的证明**（全部在 `openai.rs` 内，逐点验证）：

1. `finalize_structured_tool_calls`（唯一能把累加器变成 `ToolUse` 的函数，定义 `:1213`）
   **只有一个调用点**：`:737`，位于 `:731` 的 `else if` 内。而 `:729-730` 的
   `if matches!(stop_reason, StopReason::MaxTokens | StopReason::Refusal)` 先取走该分支，
   所以 MaxTokens 永远到不了 `:737`。
2. `infer_terminal_from_done`（`:761-776`，会用 `!tool_calls.is_empty()` 推断 `ToolUse`）在
   `:762-764` 因 `finish_seen` 提前返回；`"length"` arm 在 `:1781` 已设 `finish_seen = true`。
3. `poison`（`:694-699`）清空 `tool_calls`、置空 `pending_done`、设 `fatal_error`；
   `drain_terminal_events` 在 `fatal_error` 时（`:712-716`）返回空。

因此保留累加器**不可能**产出 `ToolUse`。于是：

- **删除** `:1801` 的 `state.tool_calls.clear()`（`"length"` arm）。`:1808`（`"refusal"` arm）**保留** ——
  拒答的残片不是「待续的工作」，语义上不该变成 `ToolUseTruncated`。
- **删除** `:1609-1618` 的 `length` poison 子句，并就地写下上面的不变式作为注释。理由：该 guard 的
  原始动机是「累加器已被清空，迟到 fragment 会凭空造出一个伪 call」；累加器保留后迟到 fragment 落入
  **正确的 index 键累加器**（`:1748` `get_or_create_tool` + `:1756-1763` 的 `duplicate_terminal_echo`
  去重已经在处理这一形状），动机消失。保留它反而让整个 OpenAI-compatible 网关家族（正是生产故障来源）
  在 `length` 上拿到硬 `ApiError` 而非可恢复的 MaxTokens —— 那是相对今天的**退化**。
  `:1604-1608` 的 `has_late_text` poison 与本设计无关，保留。
- 在 `:729-730` 把 MaxTokens 与 Refusal **拆开**：MaxTokens 走新的 `drain_truncated_tool_calls`，
  在 `:748` push `Done` **之前**发出 `ToolUseTruncated`（`Done` 之后的事件在
  `engine/mod.rs` 是硬协议违规）；Refusal 保持 `clear()`。

```rust
fn drain_truncated_tool_calls(state: &mut StreamState) -> Vec<LlmEvent> {
    state
        .tool_calls
        .drain(..)
        .filter(|acc| !acc.name.trim().is_empty())
        .map(|acc| LlmEvent::ToolUseTruncated {
            id: acc.id,
            name: acc.name,
            argument_bytes: acc.arguments.len(),
        })
        .collect()
}
```

只按 `name` 过滤、**不**按 `id` 过滤：`auto_tool_id` 网关不下发 id（`:1261-1267`
`maybe_tool_progress_event` 会在 `auto_tool_id` 时自己生成），按 id 过滤会丢掉这些网关的截断 call。
`ToolUseId` 是 `pub type ToolUseId = String;`（`message.rs:7`），空 id 合法。

**为什么 id 有意义：** `:1768` 的 `maybe_tool_progress_event` 在 `"length"` arm 清空**之前**就已发出
`ToolUseDelta` —— UI 已经开了一张工具卡。`ToolUseTruncated` 复用同一 id，让 sink 能结算那张卡。

**必须改写（不是删除）的既有测试：** `length_finish_never_executes_partial_structured_tool_call`
（`:2151-2171`，`assert!(state.tool_calls.is_empty())` 在 `:2163`）。改为断言累加器保留且带完整部分参数，
并且 `drain_terminal_events()` 恰好产出 `[ToolUseTruncated, Done{MaxTokens}]`、其中没有 `ToolUse`。
重命名为 `length_finish_reports_a_truncated_tool_call_and_never_executes_it`。
`:2174`（`length_finish_does_not_execute_even_complete_tool_arguments`）与 `:2195` 不断言累加器，保持绿。

---

## 5. Blocker 6 — Anthropic 家族必须 `mem::take` 到 stash

`anthropic_shared.rs:835` 是 `if terminal_is_tool_use != !state.pending_tool_calls.is_empty()`
→ `protocol_error("...terminal shape changed before message_stop")`。MaxTokens 时
`terminal_is_tool_use == false`，因此**要求 `pending_tool_calls` 为空**。直接删掉 `:783` 的 `clear()`
会让每一个 Anthropic / Bedrock / Vertex 的 max_tokens 响应变成硬协议错误。

正确写法：`"max_tokens"` arm（`:779-790`）把 staged 项 `mem::take` 出来转成 stash，
使 `pending_tool_calls` **留空**：

```rust
"max_tokens" => {
    // 被截断的响应里，即使语法完整的 call 也绝不执行；但它的身份是下一轮的
    // 事实，不能静默销毁。take 到 stash 让 pending_tool_calls 留空，
    // :835 的 terminal-shape 检查因此仍然成立。
    for staged in std::mem::take(&mut state.pending_tool_calls) {
        if let LlmEvent::ToolUse { id, name, input, .. } = staged {
            state.truncated_tool_calls.push(LlmEvent::ToolUseTruncated {
                id,
                name,
                argument_bytes: serde_json::to_string(&input).map_or(0, |s| s.len()),
            });
        }
    }
    if state.current_block_type.as_deref() == Some("tool_use") && !state.tool_name.trim().is_empty() {
        state.truncated_tool_calls.push(LlmEvent::ToolUseTruncated {
            id: std::mem::take(&mut state.tool_id),
            name: std::mem::take(&mut state.tool_name),
            argument_bytes: state.tool_input_json.len(),
        });
    }
    state.reset_current_block();
    state.pending_done = Some(LlmEvent::Done { stop_reason: StopReason::MaxTokens, usage });
    Vec::new()
}
```

`reset_current_block()`（`:784` → 定义 `:298-304`）必须保留：`message_stop` 在 `:817-821`
要求 `current_block_type.is_none()`。

`StreamState`（`:230-270`，14 字段）新增一个私有字段 `truncated_tool_calls: Vec<LlmEvent>`，
在唯一构造器 `fn new()`（`:279-296`）初始化。`impl Default`（`:272-276`）委托给 `new()`；
全仓**没有任何** `StreamState { .. }` struct 字面量（`StreamState::new()` 调用点四个：
`anthropic_shared.rs:392`、`:878`、`:1250`，`bedrock.rs:391`）。
`protocol_error`（`:306-312`）必须同时清空该 stash —— fail-closed。

`message_stop`（`:806-845`）在 `:842` 的 `std::mem::take(&mut state.pending_tool_calls)` 处一并释放
stash，排在 `:843` 的 `Done` 之前。

**必须改写的测试：** `anthropic_max_tokens_discards_even_a_complete_staged_tool_call`（`:1542` 区域，
`:1270` 有 `assert!(state.pending_tool_calls.is_empty())`）—— 仍断言无 `ToolUse` 且 `Done` 是
`MaxTokens`，但现在期望 **2** 个事件。重命名为 `..._reports_it_as_truncated`。

---

## 6. Blocker 7 / 8 — 判决的四重机器证据，以及 MaxTokens 的排除

重新测绘发现三类误判源，不止两类：

1. **plan mode**：`engine/mod.rs:1385` 只放行 `t.category() == ToolCategory::Info`；
2. **model-only**：`factory/nomi.rs:47` 把 `allowed_tools` 设为 `vec!["update_plan"]`，
   而 `UpdatePlanTool::category()` 是 `Info`（`update_plan.rs:132-134`）；
3. **`Info` ≠ 无副作用**（新发现）：本仓库的 `Info` 意为「无审批门」。companion 记忆写入
   （`companion_tools.rs:236-239`，带明确注释）、knowledge / skill 工具都刻意是 `Info`。

因此判决改为**四重合取**，每一项都是机器事实：

```rust
// 判决：本轮因「有工作在飞」而重启，state-changing 工具确实可用，
// 被切断的正是一个 state-changing call，而跨所有轮次没有任何
// state-changing call 成功过 —— 模型却宣称完成。
result.stop_reason == StopReason::EndTurn      // blocker 8：MaxTokens 留给 A1
    && result.rounds > 1                      // B1 的重启确实发生过
    && result.state_changing_tools_advertised  // blocker 7
    && result.cutoff_state_changing > 0        // 被切断的是 Edit/Exec/Irreversible
    && result.effects_ok == 0                  // 且从未有 state-changing call 成功
```

- **Blocker 8**：`StopReason::MaxTokens` 被第一条排除。上限耗尽仍由 A1 映射为
  `incomplete_stop_code(MaxTokens) → OUTPUT_TRUNCATED`（`relay_error_code.rs:100`），
  `fixed_code_retryable` 对它返回 `true`（`:48`）—— 可重试、可恢复，正是 D1 需要的。
- **Blocker 7**：`state_changing_tools_advertised` 在 `tools` 构建完成（`:1376-1393`）之后、
  `tool_authority`（`:1398`）旁边捕获。`ToolDef` 没有 category 字段（`tool.rs:36-42`），
  必须经 `self.tools.get(&def.name)`（`registry.rs:528`）查表 —— 同一循环内 `:2179` 已有先例。
  只对**确实在 `tools` 里**的名字查表（`:1394-1397` 的注释要求）。
- 不特判 `ToolCategory::Mcp`：生产中已死（`nomi-mcp/src/tool_proxy.rs:126-130` 只返回 Info/Exec）。
  判定集合是 `Edit | Exec | Irreversible`。
- **`retryable = true`**（旧设计写 `false`）。理由：残余误判只剩一类 —— 模型在重启后正确地放弃了那次写入
  并诚实汇报。把它记为**可重试**失败而非死路，把误判的最坏后果限制在「用户可以再试一次」。

**Blocker 9 — 计数用独立单调计数器。** `RoundLedger` 上声明两个 `usize`：
`effects_total` 与 `effects_ok_total`，由 `push_effect` 在 24 条渲染窗口裁剪**之前**递增。
`AgentResult.effects_ok` 携带 `effects_ok_total`。重启谓词读 `effects_total`，
不从会淘汰旧记录的展示窗口反推。

**Blocker 11 — 不在持有 engine mutex 时 await。** `agent.rs:2200` 是
`drop(engine); // no engine mutex across artifact I/O or provider work`，
而 `Ok(agent_result)` arm 从 `:2140` 一直持锁到 `:2200`。判决分支放在
**artifact 校验块结束（`:2245`）之后、`distill_job`（`:2247`）之前**：

- mutex 已释放；
- 更具体的 artifact 错误（`:2225-2244`，本设计复用的先例）优先胜出；
- 不为一个即将失败的 turn 浪费一次 distill pass。

`AgentSendError::new`（`send_error.rs:58-78`，7 参）第 7 参必须写
`Some(AgentErrorResolution::new(AgentErrorResolutionKind::ChangeModel, Some(AgentErrorResolutionTarget::ProviderSettings)))` ——
`send_error.rs:724` 的 `fn resolution` 是**私有**的，跨模块不可见（blocker 12）。

---

## 7. 重启谓词与 hook

hook 只在 `stop_reason == StopReason::MaxTokens && tool_calls.is_empty()` 时可达：
`:1755-1758` 已把 `MaxTokens` + 非空 `tool_calls` 拒为硬协议错误，且发生在 `:1909` 的 push **之前**。

```rust
let restart = stop_reason == StopReason::MaxTokens
    && round.attempt < MAX_ROUND_ATTEMPTS
    && tools_advertised
    && (!truncated_calls.is_empty()          // 一个 tool call 被字面切断
        || round.ledger.has_open_plan()       // 模型声明了带 pending/in_progress 步骤的计划
        || round.ledger.effects_total > 0);   // 本轮已经改变过状态
```

| 子句 | 事实来源 |
|---|---|
| `tools_advertised` | `let tools_advertised = !tools.is_empty();` 在 `:1398` 旁捕获，`tools` 于 `:1458` 移入 `LlmRequest` 之前 |
| `truncated_calls` | 新 `LlmEvent::ToolUseTruncated`，per-pass 局部量，与 `let mut stop_reason` 同处声明所以每 pass 重置 |
| `has_open_plan()` | 最近一次成功 `update_plan` 快照里存在 `StepStatus::Pending \| InProgress` |
| `effects_total` | 本轮已派发的 `Edit`/`Exec`/`Irreversible` 工具结果的单调计数 |

**「散文就是交付物」不再是硬错误。** 一个纯长答案没有被切断的 call、没有计划、没有状态变更，
谓词为 `false`，引擎照今天一样返回 `Ok(AgentResult { stop_reason: MaxTokens, .. })`，
durable 文本行保留，A1 把 receipt 记为 failed-but-**retryable**。**这正是生产故障的形状：
B1 不会自动重启它**，因为对同一个上限重发同一个请求只会得到同一个结果。
该形状的修复是 A1（诚实 receipt）+ C1（真实上限）+ D1（用户显式 Continue）。

```rust
if restart {
    // provider 在组织输出的中途撞到输出上限。续写一个被截断的草稿不可恢复：
    // 针对 ORIGINAL requirement 重启本轮，并把机器构建的 ledger 带过去。
    // 故意不递增 `turn` —— 一轮是本 turn 的重试，不是又一次工具循环迭代，
    // 而 model-only 会话运行在 max_turns = 1。
    let dropped = self
        .messages
        .pop()
        .expect("the assistant message pushed at :1909 is still the tail here");
    debug_assert_eq!(dropped.role, Role::Assistant);
    round.attempt += 1;
    round.ledger.cutoff = std::mem::take(&mut truncated_calls);
    self.redact_user_images_since(0);
    self.messages
        .push(Message::now(Role::User, round.requirement.clone()));
    for text in self.drain_steering() {
        self.messages
            .push(Message::now(Role::User, vec![ContentBlock::Text { text }]));
    }
    // 失败关闭地结算任何已发布但未落定的工具卡，并重置每轮引用缓冲。
    self.output.emit_stream_start(&self.current_msg_id);
    *safe_messages = self.messages.clone();   // 第 N 轮的回卷地板
    self.save_session();
    tracing::warn!(target: "nomi_agent", attempt = round.attempt, /* ... */);
    continue;
}
```

`let dropped = ... .expect(..)` 而非 `let Some(..) = .. else { /* comment */ }`：
后者的 else 块类型是 `()`，`let`-`else` 要求 `!`（blocker 12）。

`emit_stream_start` 是 host-only `truncate_active_tool_calls_for_auto_continue` 的精确替代：
`BackendOutputSink::emit_stream_start`（`backend_output_sink.rs:2807-2822`）已经调用
`fail_active_tool_calls` 并清空 `turn_text`，其注释甚至已经点名 MaxTokens 自动续跑。
它在 `OutputSink` 上是**必需**方法（`output/mod.rs:1093`），12 个实现者全部就绪。

**本设计不声称的一件事：** `stream_relay.rs` 的 `full_text_buffer` 累积整个 turn 的每个 `Text` 事件，
第 1 轮的可见散文若后续轮次干净结束仍会成为 `final_text` 的一部分。这在今天的宿主循环下同样如此，
B1 不使其退化。可见边界事件属于 D1。

---

## 8. Ledger 填充 —— 只有机器真相

**Producer A — `update_plan` 快照。** 引擎已持有原始 `ContentBlock::ToolUse { input }`，不需要宿主私有的
`parse_plan_entries`。以 `!is_error` 为门（`update_plan.rs:141` 无效参数、`:146` 空计划都返回
`ToolResult::error`，两者都不得覆盖好 ledger）。整快照语义：**替换**，不合并。
`UpdatePlanTool::category()` 是 `Info`，Producer B 不会重复计数。

**Producer B — effect 日志。** 对每个其 call 的 `category_for(input)` 属于
`Edit | Exec | Irreversible` 的结果，push `{ tool, label, ok: !is_error }`，
渲染窗口限 24 条、丢最旧；同时递增 `effects_total` / `effects_ok_total` 两个单调计数器。
成功的 `Read` 不是进展，`Write` / `Bash` 是。`label` 用
`truncate_middle(&t.describe(input), TruncationBudget::Bytes(160))` —— `TruncationBudget`
只有 `Bytes(usize)` 一个变体（`output_truncation.rs:15`），且 `Tool::describe` 的**默认实现会 dump
整个 input JSON**，所以限长是承重的。

**放置位置 —— 旧设计踩进去的借用陷阱。** `artifact_identity` 借用 `self.tools` 并在
`for result in &mut outcome.results`（`:2050` 起）整个循环内保持活跃，任何 `&mut self` 调用都是借用错误；
且该循环的 `find_map` 只绑定 `id, name`，从不绑定 `input`。两个问题都因把 ledger pass 放到循环**之后**、
紧接 `efficiency.observe_results(&outcome.results)`（`:2153`）而消失 —— 这正是同一形状的既有先例。
在那里读 `is_error` 也**更**正确：它反映了 `ToolMediaDelivery::Failed` 的调整。

**没有第三个 producer。** 不扒 transcript（那正是观测轨迹里凭空出现 `Read` 的原因），
不做自我总结 pass（又一次对同一上限的生成），不探文件系统。

**渲染的 system section**，由 `engine/mod.rs:252` 旁新增的模块级自由函数
`fn append_round_context(system: String, section: Option<String>) -> String` 追加，
在 `:1446-1449` 的 `append_system_resource_context` 之后、`:1452` 的
`cache_detector.record_request` 之前调用：

```
[resumable round 2/3] 你上一次尝试被 provider 的输出 token 上限切断。那份草稿已从你的
上下文中移除，无法续写。原始请求作为下面最后一条用户消息重述。

ALREADY DECLARED (your own plan):
  [x] scaffold the toolbox layout
  [>] write miniapp.html
  [ ] verify it opens

ALREADY DONE (observed tool effects):
  ok    Bash: mkdir -p toolbox

WHAT WAS CUT OFF:
  Write (6142 bytes of arguments streamed, NOT executed)

RULES FOR THIS ATTEMPT:
- 你的第一个动作必须是工具调用。不要用散文重述计划。
- 大文件必须拆分：先写一个小而完整的版本，再 Edit/追加。
```

走 system 通道，绝不 `Role::User`：`set_system_resource_inbox` 的文档已经陈述了这条规则
（「never into the conversation transcript as a user message」），system prompt 在循环内每 pass
重建（`:1400-1449`）。requirement 文本**不**在这里重复 —— 它是尾部用户消息。

`render_section` 必须能读到 `attempt`。签名定在 `RoundState` 上（`fn render_section(&self) -> Option<String>`，
读 `self.attempt` / `MAX_ROUND_ATTEMPTS` 并把三个正文块委托给 `self.ledger`），
调用写 `round.render_section()`（blocker 12 的第二项）。

**新类型**（`crates/agent/nomi-agent/src/round.rs`，新文件；`lib.rs` 在 `requirement_tools:21` 与
`ssh_backend:22` 之间加 `pub mod round;`）。`RoundState` 持
`requirement: Vec<ContentBlock>`、`attempt: usize`、`ledger: RoundLedger`，
**不** derive `PartialEq` —— `ContentBlock` 不 derive 它（`message.rs:10`）。
`RoundLedger` 只持 `String` / `usize` / `bool` / `StepStatus`。
**不需要 `sha2`**（无 digest fence）、**不需要 serde 持久化**（in-memory）。

---

## 9. Blocker 10 — conversation middleware seam

已独立复核确认：`stream_relay.rs:3723` 的
`let failed_terminal = matches!(event, AgentStreamEvent::Error(_)) || cancelled;`
对 `Finish{MaxTokens}` 为 `false`，于是完整链条是：

1. `:3750` 的 `if failed_terminal` 不取 → `:3761` `process_final_text` 在截断草稿上运行，
   **并执行**其中的 cron 命令（`response_middleware.rs:324-326`；`:47` 的 cron 正则是无闭合标签的裸匹配，
   半句话里出现 `[CRON_LIST]` 就触发）；
2. `:3855-3856` → `outcome.system_responses` 非空；
3. `service.rs:9038-9045` → 诚实 receipt 已赋值（`result_ok = false`、`output_truncated`、retryable）；
4. `service.rs:9058` 的 `if outcome.system_responses.is_empty()` 为 **false**，
   于是包含正确 write-back 门（`:9066-9067`）和 `break`（`:9087`）的块被**整块跳过**；
5. `service.rs:9099-9114` → `continuation_count += 1`、新 msg_id、`pending_send = Some(..)`；
6. 下一次迭代干净结束 → `:9038` `turn_succeeded == true`、`:9044` `map_turn_failure == None`；
7. `durable_completion` 声明在循环**外**（`:8654`）且被**无条件**重新赋值 → 诚实的
   `output_truncated` 被销毁；
8. `:9117` 取到幸存者 → 写 `result_ok = 1`、`result_error_code = NULL`。

这就是 A1 本该修掉的那个生产 bug，从续跑门重新可达。A1 只教会了两个纯函数和一个消费者
（write-back 门）关于 `stop_reason` 的事，没有触及 receipt 的**生命周期**。

**两处都修：**

**(A) 原则修复 —— 收窄 `failed_terminal`（`stream_relay.rs:3723`）。**
把任何 incomplete stop reason 纳入：

```rust
let failed_terminal = matches!(event, AgentStreamEvent::Error(_))
    || cancelled
    || matches!(event, AgentStreamEvent::Finish(data)
        if crate::relay_error_code::incomplete_stop_code(data.stop_reason).is_some());
```

`:3751-3755` 的注释已经把这写成政策（「do not strip/execute embedded cron commands, emit
continuations, or expose it as final-text/writeback material」）—— MaxTokens 只是恰好落在谓词之外。
副作用：`outcome.final_text` 变为 `None`。receipt 仍诚实，因为 `map_turn_failure`（`:147-149`）
在空文本检查**之前**返回 incomplete code；用户可见的散文仍以独立 durable message row 存在。

**(B) 防御性修复 —— 让 `durable_completion` 对失败单调。** 在 `service.rs:9099` 之前加一道与
write-back 门同源的谓词，使任何未来的第二轮路径都不能覆盖诚实的失败：

```rust
if relay_error_code::incomplete_stop_code(outcome.stop_reason).is_some() {
    break;
}
```

(A) 使 (B) 不可达；(B) 保留为 belt-and-braces，且修掉「让模型续写它被切断的工作」这一语义荒谬。

---

## 10. 被删除的东西

| 位置 | 删除内容 |
|---|---|
| `manager/nomi/agent.rs:728-731` | `const MAX_TRUNCATION_AUTO_CONTINUES: usize = 2;` 及其文档 |
| `manager/nomi/agent.rs:733-746` | `fn truncation_continuation_prompt(..)` —— 静态英文恢复散文，全删 |
| `manager/nomi/agent.rs:2009` | `let mut truncation_auto_continues = 0usize;` |
| `manager/nomi/agent.rs:2071-2106` | 整个宿主 `MaxTokens` 续跑分支：计数、`truncate_active_tool_calls_for_auto_continue` 调用、`run_content = vec![prompt]`、`continue`、以及 `warn!` 后落到干净 `Finish` 的 fall-through。`loop` 保留给 steering race-tail（`:2036-2070`） |
| `capability/backend_output_sink.rs` | `fn truncate_active_tool_calls_for_auto_continue` + 其 2 个专属测试；另有 3 处测试调用行需要摘掉 |
| `openai.rs:1801` | `"length"` arm 的 `state.tool_calls.clear()` |
| `openai.rs:1609-1618` | `length` 之后 tool fragment 的 poison 子句 |
| `gemini.rs:763-767` | `ProviderError::Parse("Gemini stopped at MAX_TOKENS with an uncommitted function call")` arm |

无兼容 shim、无恢复 append-prose 的配置开关、无 ledger 双写、宿主高度无第二个重试循环。

**Gemini（`:762-767`、`terminal_events` `:784-810`）：** Gemini 的 `pending_calls` 持**完整解析**的
call（`input: Value`），所以 `MAX_TOKENS` + pending 意味着完整 call 已到达且随后撞上上限。
今天这是硬 `ProviderError::Parse`。改为 `StopReason::MaxTokens` + 每个 pending call 一个
`ToolUseTruncated`（`argument_bytes` 取 `serde_json::to_string(&input)` 的长度）。
这与 OpenAI/Anthropic 已确立的政策一致（`openai.rs:1797-1800`、`anthropic_shared.rs:780-782`
的注释：截断响应里即使语法完整的 call 也不执行），且严格优于一个不透明的 parse 错误。

---

## 11. 编译爆炸半径（逐个 grep 验证）

**新 `LlmEvent` 变体 —— 全仓恰好 2 个无 catch-all 的 match 会断：**
- `engine/mod.rs:1535`（闭合于 `:1742`）—— 引擎流式循环
- `image_generation.rs:185`（闭合于 `:215`）—— image-intent 分类器（生产代码，非测试）

证明方式不是肉眼：穷尽 match 必须点名 `ThinkingSignature`；对全部 `.rs` 做括号配平扫描后，
70 个提及 `LlmEvent` 的 match 中只有这两个是无 catch-all 且 scrutinee 为 `LlmEvent`。
`LlmEvent` **不是** `#[non_exhaustive]`，只 derive `Debug, Clone`。

**新 `AgentResult` 字段 —— 恰好 6 个字面量，全部穷尽（4 字段）：**
`engine/mod.rs:1288, 1338, 1981, 2293, 2308`（生产）+ `local_agent_invocation.rs:1343`（测试）。
**不存在任何解构模式**（全仓 19 处 `AgentResult` 引用，非字面量非定义的全是类型位置）。
`AgentResult` 只 derive `Debug`。

**新 `AgentErrorCode` 变体：** `agent_error.rs:14-54`（37 变体，`SCREAMING_SNAKE_CASE`），纯追加。
- **不是** ts-rs 导出（文件内零 `ts_rs` / `TS` / `#[ts(` token）；UI 侧
  `companionError.ts` 有 `default:` 兜底，`chatLib.ts` 只校验 ownership/resolution 集合 → **无 UI 改动**。
- 全仓**不存在**对 `AgentErrorCode` 的穷尽 `match`。
- `agent_error_code_token`（`relay_error_code.rs:167-172`）是 serde 驱动 → token 免费得到。
- `is_provider_fault`（`model_failover.rs`）是 `matches!` 列表 → 新变体正确地**不是** provider fault，无需改码。
- **无 DB migration**：`result_error_code` 无枚举 CHECK。

**新 `anthropic_shared::StreamState` 字段：** 唯一构造器 `new()`（`:279-296`），`Default` 委托（`:272-276`），
全仓无 struct 字面量。

**刻意规避、因此验证为不受影响：**
- *无新 `AgentEngine` 字段*（否则会断 `engine/mod.rs:639`、`:718` 以及 5 个测试 helper，均无 `..Default::default()`）。
- *无 `Session` 改动*（全仓恰好 5 个穷尽字面量，`Session` 无 `Default`）。**无 `host_context` 改动**（ledger 不持久化）。
- *无 `StopReason` 变体*（否则断 `ToolEfficiencyStats::terminal_dimensions` 的 `match` `:425-431`）。
- *无 `AgentError` 变体*（`terminal_dimensions` 的 `match error` `:438-444` 也是穷尽的）。
- *无 `TokenUsage` 字段*、*无 `TurnStopReason` 变体* → 无 ts-rs churn。

**必须改写（不是删除）的既有测试：**
- `openai.rs:2163` `assert!(state.tool_calls.is_empty())`（`fn` 在 `:2151`）
- `anthropic_shared.rs` 的 `anthropic_max_tokens_discards_even_a_complete_staged_tool_call`
- `manager/nomi/agent.rs:4905-4952` `send_message_auto_continues_after_max_tokens_before_finish`
  → 改写为 prose-only 截断不重启（`calls() == 1`、`MaxTokens`）
- `manager/nomi/agent.rs:6079-6140` `send_message_does_not_auto_continue_after_max_turns` → 应保持绿
- `backend_output_sink.rs` 3 处调用被删方法的测试行 + 2 个专属测试
- `local_agent_invocation.rs:1343` 的 `AgentResult` 字面量
- `stream_relay.rs:8224` `a_turn_truncated_by_the_output_ceiling_is_never_adjudicated_as_success`（A1 的，需复核 (A) 是否改变其断言）

---

## 12. 本设计在观测故障上的行为

provider `openai-compatible`，model `step-3.7-flash`，上限 8192，`output_tokens = 24576 = 3 × 8192`，
零工具调用，`result_ok = 1`，磁盘上什么都没有。

1. **pass 1**：`finish_reason: "length"` → `pending_done = Done{MaxTokens}`。`state.tool_calls` 本来就空
   （从未开始任何 call），`drain_truncated_tool_calls` 产出空。引擎：`stop_reason = MaxTokens`，
   `tool_calls.is_empty()`，助手消息 push 于 `:1909`。
2. **重启谓词**：`tools_advertised = true`，但 `truncated_calls` 空、无 `update_plan`、
   `effects_total == 0` → **`false`**。不重启。引擎返回
   `Ok(AgentResult { text: <散文>, stop_reason: MaxTokens, rounds: 1, effects_ok: 0, .. })`。
   **第 2、3 个 8192-token pass 不再发生**：`output_tokens` 从 24576 降到 8192。
3. **宿主**：被删分支不再追加 `truncation_continuation_prompt`、不再吞掉结果。判决的第一条
   （`EndTurn`）与第二条（`rounds > 1`）都为假，不触发。
   `map_engine_stop_reason(MaxTokens) → TurnStopReason::MaxTokens`（`:784-794`）。
4. **Receipt**：A1 的 `incomplete_stop_code(Some(MaxTokens)) = Some(OUTPUT_TRUNCATED)` →
   `turn_succeeded` 为 **false** → `result_ok = 0`、`result_error_code = "output_truncated"`、
   `result_error_retryable = 1`。§9(A) 使 `process_final_text` 不再在截断草稿上运行，
   于是 **不会**再有续跑 turn 覆盖这个 receipt，**也不会**从半句话里执行 cron 命令。
   **「`result_ok = 1` 而磁盘上什么都没有」彻底消失。**
5. **真正的反事实**（本工作流存在的理由）：若模型在耗尽空间前调用过 `update_plan` 和 `mkdir` ——
   远更常见的形状 —— 谓词为 `true`：引擎 pop 掉 8192-token 草稿（因此它离开了 **provider 请求**），
   在尾部原样 re-push 原始 requirement，把 `[resumable round 2/3]` + 已声明计划 +
   `ok Bash: mkdir -p toolbox` 注入 **system** prompt，重设回卷地板，带着「已完成什么」的 ledger
   跑 pass 2 —— 而不是让模型续写一个写了一半的字符串。

---

## 13. 不宣称的事

- **EndTurn-with-false-claim 未关闭。** A1 只保证「最终仍是 MaxTokens」的轮次不能记成功。
  §6 的判决只覆盖「B1 自己重启过、且被切断的正是一个 state-changing call」这一窄形状，
  它是防止 B1 的重启机制制造假成功，不是通用的假完成检测。通用形状归 A2。
- **ledger 不跨进程存活。** 这是有意的范围决定（§0）。D1 若需要用户发起的 Continue 复用 ledger，
  届时再加持久化 —— 类型已就位，且那时才有真实消费者来定义 fence 语义。
- **§9(A) 会让截断 turn 的 receipt `result_text` 变为 NULL。** 可见散文仍在独立 message row 中。
  若 D1 需要 receipt 上的截断文本，需单独设计，不要靠 `final_text` 这条已判定为
  「未完成响应」的通道。
- **distillation 会看到重启后的 transcript。** `agent.rs:2194` 的 `engine.messages_transcript()`
  覆盖整个消息向量。pop 掉草稿实际上**改善**了这一点（截断散文不再进入 distill），
  但 requirement 会出现两次。未做处理，记录在此。

---

## 14. 完备性复核补充（第二轮 critique 的实质发现）

**(1) hook 位置被独立证明只有一个正确窗口：`:1912` 与 `:1918` 之间。** 两个危险各自排除一侧：
- 放在 `:1929` steering hook **之后**：此时 `self.messages.last()` 是一条 **steering 用户消息**，
  `pop()` 会删错东西（这正是 blocker 4 的具体机制）；
- 放在 `:1918` **之后**：`*safe_messages` 快照里**仍然含有截断草稿**，后续某轮失败时会回卷到
  一个包含该草稿的 transcript。

本设计把 hook 作为 `:1912` 块的第一条语句（`:1913` 的 `stagnation_guard.reset()` 之前），落在窗口内。

**(2) `user_content` 在 `:1318` 被 move，不是 `:1317`。** 签名 `:1251-1260`，`user_content: Vec<ContentBlock>`
在 `:1253` 按**值**接收；`:1318` 的 `self.messages.push(Message::now(Role::User, user_content))` 移走它。
`:1318` 之后 requirement 只存在于 `self.messages[..]` 里。因此 clone **必须**在 `:1318` 之前捕获。
准入门在 `:1261-1269`（判定在 `:1264`）只放行 `Text | Image`，所以 re-push 的 clone 原样满足该门。

**(3) `rounds` / `effects_ok` 不进 `TurnCompletedEventData`。** 该结构在
`protocol/events/mod.rs:104-124` 是 **ts-rs 导出**（derive 在 `:101`，`#[ts(export_to = ...)]` 在 `:102`，
由 `:171` 的 `export_binding_if_changed::<TurnCompletedEventData>` 生成），而导出测试在形状变化时
**直接写入 `ui/`**，会改动工作树而不是干净失败。因此两个计数只留在 `AgentResult` 与宿主的 `info!` 日志里，
不上协议 —— 零 ts-rs churn，`bun` 侧无需改动。

**(4) 仓库没有 warnings gate**（根 `Cargo.toml` 无 `[workspace.lints]`，
`nomifun-ai-agent/src/lib.rs` 无 `#![deny(..)]`，AGENTS.md 也不提 `-D warnings`）。
所以删掉 `truncate_active_tool_calls_for_auto_continue` 的唯一生产调用点后，该方法只会变成
`dead_code` **警告**而非编译错误 —— 没有任何东西强制清理。因此必须**显式**删除方法本体
（`backend_output_sink.rs:2267`）、它的 2 个专属测试，以及 3 处仍会调用它的测试行
（`:3068`、`:3095`、`:3125`、`:4781`、`:4864` 中属于这两类的行）。

**(5) Blocker 9 的前提是一个幻影，但结论仍然正确。** 仓库里**不存在**任何既有的 24 条渲染窗口
（全仓 `= 24` 的 9 处命中全部无关）。那个窗口是**旧设计自己提议**的 effect 向量上界，
blocker 9 说的是「不要从这个会丢记录的窗口反推计数」。本设计用 `effects_total` /
`effects_ok_total` 两个单调 `usize` 计数器，在裁剪之前递增，因此按构造满足。
在引擎内已有同形先例：`ToolEfficiencyStats`（`engine/mod.rs:362-375`）的 9 个 `usize`
字段全部用 `saturating_add` 递增。

**(6) Blocker 12 的 `render_section` 子项按字面不可执行**：全仓不存在名为 `render_section` 的函数 ——
它是旧设计**提议**的新函数。真正的编译危险是所有者与参数：旧设计把它声明在 `RoundLedger` 上却
用 `round.render_section()`（`round: RoundState`）调用，而渲染 `[resumable round 2/3]` 表头需要
`attempt`，那是 `RoundState` 的字段。本设计把它定义在 `RoundState` 上（`fn render_section(&self) -> Option<String>`，
读 `self.attempt`，三个正文块委托给 `self.ledger`），子项因此消解。

**(7) 已知残余风险，不在 B1 修复范围，记录备查：**
- `tool_retry_tracker` 在 `:1325` 是**循环外**局部量，`assign`（`:1771-1777`）对一个 root turn 内
  重复的 `tool_use_id` 报硬 `ApiError`。B1 的重启不递增 `turn`，因此新轮共享该 tracker。
  截断的 call **从不进入** `tool_calls`（它是 `ToolUseTruncated`），所以纯截断轮不注册任何 id，
  重启后重发同名调用是首次注册 —— 安全。只有当同一 `execute_turn_inner` 里更早的轮次已完成过
  工具调用、且 provider 铸造确定性 id 时才会碰撞，而这与今天的多轮工具循环行为相同，B1 未使其恶化。
- `cache_detector.record_request`（`:1452`）在每个重启轮重跑。重启会为同一逻辑 turn 产生
  实质不同的消息向量 —— 正是 `CacheBreakDetector` 要标记的形状。可能出现每个截断 turn 一条
  多余的 cache-break 诊断。纯诊断，无功能影响。
- 草稿组装时 thinking 状态是 **move 而非 clone**（`:1883-1888` 移走 `thinking_text` 与
  `thinking_signature`；`:1889-1893` 的 `assistant_text` 是 clone，所以它能活到 `:1982`）。
  pop 掉草稿会丢弃该轮的 reasoning signature。这是有意的：截断草稿整体不可续写。
