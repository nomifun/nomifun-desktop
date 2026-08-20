# Nomifun Coding Agent 截断重构：B1「可恢复轮次」阶段交接

日期：2026-08-20
分支：`fix/truncation-not-success`
B1 开始前 HEAD：`122646f8`（C1 + A2b）

配套设计文档：`docs/handoffs/2026-08-20-truncation-refactor-b1-design.md`
（含全部行号重新测绘、12 个阻塞的逐项解决、以及第二轮 critique 的实质发现）

## 阶段边界

本阶段只完成 B1。**没有**开始 D1、E1。没有触碰 `openai.responses`，没有加回
wire sanity cap，没有改动 C1 的 capability ceiling 逻辑。

---

## 一、先做了什么：重新测绘

旧设计（`2026-08-20-truncation-refactor-resumable-round.md`）的**每一个行号都已失效**，
且偏移量符号不一致（常量区 +13、计数器 −29、循环体 +22），没有统一位移可用。
因此先做了 13 路只读测绘 + 1 路完备性 critique，产出 1405 个逐个打开读过的站点。

测绘推翻了旧设计的若干前提，其中影响实现的：

1. **`LlmEvent` 新变体全仓只断两处**穷尽 match（`engine/mod.rs` 流式循环、
   `image_generation.rs` image-intent 分类器），由括号配平扫描证明，不是肉眼估计。
2. **`ContentBlock` / `Message` 不 derive `PartialEq`** —— 锚点不能靠值相等定位，
   测试不能对 `Message` 用 `assert_eq!`。
3. **`append_system_resource_context` 是模块级自由函数**，不是 `AgentEngine` 方法。
4. **`limit` 在循环内部**每轮重算，不与 `let mut turn` 同处。
5. **`truncation_continuation_prompt` 所在分支被两个 tokio 测试钉住**
   （旧设计说「没有测试，可直接删」是半对半危险），且 `ScriptedProvider::stream`
   在未脚本化的 pass 上 panic。
6. **`ToolCategory::Info` 意为「无审批门」而非「无副作用」** —— companion 记忆写入、
   knowledge / skill 工具都刻意是 `Info`。
7. **`ToolCategory::Mcp` 在生产中已死**（MCP proxy 按 annotation 只返回 Info/Exec）。
8. **`TurnCompletedEventData` 是 ts-rs 导出**且其导出测试会**直接写入 `ui/`**。
9. **仓库没有 warnings gate**，所以删除唯一生产调用点只会留下 `dead_code` 警告而非编译错误。

---

## 二、相对旧设计的三个范围决定

| 决定 | 理由 |
|---|---|
| **ledger 不持久化**，降为 `execute_turn_inner` 的栈内局部量 | 重启完全发生在一次 `execute_turn_inner` 调用内，ledger 唯一消费者是同一调用中下一 pass 的 system section。持久化会引入 `host_context` 前缀契约、两处 checkpoint 快照过滤、sha256 digest fence、5 个 `Ok` + 28 个 `Err` 出口的 reap 规则、以及与 `rewind_last_turn` 的交互 —— 全部为一个尚不存在的消费者（D1）服务。critique 另外证实：`host_context` 在 conversation reset 中**不被清除**，持久 ledger 会静默跨 reset 存活。删除 sha2 依赖。 |
| **重启谓词只接受 pass 级证据**（`!cutoff.is_empty()`），不接受 turn 级证据 | 对抗复核确认：`effects_total > 0` / `has_open_plan()` 是 turn 生命期事实，会让一个「工具活已干完、末尾散文被截断」的 pass 也重启 —— 白烧两个上限重生成同一段散文（正是被删除的宿主循环造成的浪费），并且给模型一段命令它「第一个动作必须是工具调用」的 system section，可能让已完成的 Exec/Irreversible 再跑一次。 |
| **不落地宿主侧硬终态判决**，只记录四项事实 + `warn!` | 对抗复核找到一类真实误判：fork 模式 `Skill` 委托做真实文件/exec 工作，而 `Skill` 工具本身是 `Info`，所以交付物经此到达的 turn 会算出零 state-changing effect。落地硬判决还会跳过 `TurnCompleted` metrics 事件，并需要尚不存在的错误卡 i18n。「裁决一个完成声明」属于拥有 unbacked claim 的工作流；B1 的义务只是不制造该形状，并让它可测量。 |

---

## 三、已实现

### 1. Provider：被截断的工具调用不再静默消失

新增 `LlmEvent::ToolUseTruncated { id, name, argument_bytes }` —— **永不可执行**，
永不进入引擎的 `tool_calls`，永不做 schema 校验。`id` 复用之前已发出的 `ToolUseDelta`
的 id，使 sink 能结算它已经打开的工具卡。

- **OpenAI-compatible**：`"length"` arm 不再 `clear()` 累加器；在唯一真终点
  `drain_terminal_events` 的 MaxTokens 分支 drain 成 `ToolUseTruncated`，排在 `Done` 之前。
  MaxTokens 与 Refusal 拆分：refusal 的残片不是「待续工作」，继续静默丢弃。
  **无执行不变式是结构可证的**：`finalize_structured_tool_calls`（唯一能产出
  `ToolUse` 的函数）只有一个调用点，位于 MaxTokens 永不进入的 `else if` 分支；
  `infer_terminal_from_done` 因 `finish_seen` 提前返回；`poison` 直接清空。
- **`"length"` 之后的迟到 tool fragment 决定为「安全累加」**：删除专门的 poison 子句。
  该 guard 的原始动机（累加器已清空，迟到 fragment 会凭空造伪 call）在保留累加器后消失，
  而保留它会让整个 OpenAI-compatible 网关家族（正是生产故障来源）在 `length` 上拿到硬
  `ApiError` 而非可恢复的 MaxTokens —— 相对今天的**退化**。
- **重复终帧不再双计**：`duplicate_terminal_echo` 的 `is_ok` 解析检查只能识别**完整**参数串，
  而 `length` 截断串永不解析成功，因此加了 `truncated_terminal` 子句。否则
  `argument_bytes` 会正好报成真实值的两倍。
- **Anthropic / Bedrock / Vertex**：`"max_tokens"` arm 用 `mem::take` 把 staged 调用与
  in-flight `tool_use` 块移入 `truncated_tool_calls` stash，使 `pending_tool_calls`
  **留空** —— `message_stop` 的 terminal-shape 检查要求非 `ToolUse` 终态时它为空，
  直接删 `clear()` 会让每个 max_tokens 响应变成硬协议错误。`protocol_error` 同时清 stash。
- **Gemini**：`MAX_TOKENS` + pending calls 不再是硬 `ProviderError::Parse`，
  改为 `MaxTokens` + 每个 pending call 一个 `ToolUseTruncated`。与 OpenAI/Anthropic
  已确立的政策一致（截断响应里即使语法完整的 call 也不执行），且严格优于不透明 parse 错误。
- `emitted_content` 重试分类器与引擎 TTFT gate **不需要**加入新变体：
  `ToolUseTruncated` 只在成功终点发出，从不在「partial vs empty」区分生效的流式阶段出现。

### 2. 引擎：可恢复轮次

`crates/agent/nomi-agent/src/round.rs`（新文件）：`RoundState` / `RoundLedger` /
`LedgerStep` / `LedgerEffect` / `LedgerCutoff` / `MAX_ROUND_ATTEMPTS = 3`。

- **回卷锚点不含偏移量**：`self.messages.pop()` 移除本 pass 在 `if tool_calls.is_empty()`
  之前无条件 push 的助手草稿；requirement 是 `execute_turn_inner` 的栈内 `Vec<ContentBlock>`
  clone，在 `user_content` 被 move 进 transcript **之前**捕获。两者都与自动压缩无关
  —— 压缩会整体替换消息向量并把根用户消息缩成摘要，这正是 `start_len` 方案失效的原因。
- **hook 位置只有一个正确窗口**，由 critique 独立证明：在 `if tool_calls.is_empty()` 之后、
  `*safe_messages` 刷新之前。放在 steering hook 之后会 pop 掉一条 steering 用户消息；
  放在 `safe_messages` 刷新之后会让回卷地板仍含截断草稿。
- **不递增 `turn`**：model-only runtime 被钉在 `max_turns = 1`，任何 `turn` 递增都会结束
  turn 而不是重试它。终止只由 `attempt < MAX_ROUND_ATTEMPTS` 界定，因此每 turn 最多 2 次重启，
  与 `max_turns` 无关 —— 旧宿主循环允许 `3 × max_turns` 个 pass。
- **steering 永不删除**：只移除一条助手消息。hook 额外自己 drain steering 并追加，
  于下一 pass 立即送达（此处 drain 无条件安全，因为重启不递增 `turn`，必定还有一个 pass）。
- **多模态保留**：requirement 原样 re-push，`Image` 块包含在内；先调用既有的
  `redact_user_images_since(0)` 保证线上每张图只有一份活体。
- **requirement 不重复发送**：re-push 只在它不是 tail 时执行。比较在 redaction **之后**
  经 `serde_json::Value` 进行（`ContentBlock` 无 `PartialEq`），这使三种形状都正确：
  纯文本且仍在 tail → 不追加；多模态 → 历史副本已是占位符，值不同 → 恢复活体载荷；
  tail 是工具结果或 steering → 值不同 → 在模型真正会行动的位置重述。
- **section 只被一个 pass 消费**（`take_section`）：section 表头断言「上一次尝试被切断、
  草稿已移除」并命令模型以工具调用开场，这只对紧随重启的那一个 pass 为真。
  留着会让它被追加到本 turn 每个剩余 pass 的 system prompt，使一个重启过一次、
  之后正常工作的模型在健康工具循环中途反复被告知草稿刚被丢弃。
- **单调计数器**：`effects_total` / `effects_ok_total` / `cutoff_state_changing_total`
  在 24 条渲染窗口裁剪**之前**递增，计数永不因边界丢失。
- **渲染有界且单行**：plan / effects / cutoff 三块各限 24 条并声明省略数量；
  标签经 `truncate_middle` + 空白折叠（`truncate_middle` 的省略标记本身含换行，
  且 `Tool::describe` 默认实现会 dump 整个 input JSON）。UTF-8 边界安全，有测试。
- **effect 分类偏向「确实发生了什么」**：`category()` 或 `category_for(input)` 任一为
  state-changing 即计数。多动作工具（browser / computer）基础类别是 Exec，
  因此会计入 `state_changing_tools_advertised`（该处无 input 可判），
  而 `category_for` 对某次只读调用可能返回 Info —— 只按 `category_for` 判定会让这类工具
  能武装判决却永远无法满足它。
- **system 通道复用既有 merge**：`context_contributor::merge_pre_turn_context`，
  不再自己写一个逐字节等价的追加函数。

`AgentResult` 新增 `rounds` / `effects_ok` / `cutoff_state_changing` /
`state_changing_tools_advertised`。**故意不进 `TurnCompletedEventData`**（ts-rs 导出，
其测试会写入 `ui/`），零 ts-rs churn。

### 3. 宿主：删除自动续跑

删除 `MAX_TRUNCATION_AUTO_CONTINUES`、`truncation_continuation_prompt`（静态英文恢复散文）、
整个 `MaxTokens` 续跑分支、`truncate_active_tool_calls_for_auto_continue`
（含其 2 个专属测试，3 处测试调用改用仍存在的 `fail_active_tool_calls`）。
`loop` 保留给 steering race-tail。

四项事实在 `drop(engine)` **之后**以 `warn!` 记录，不作终态判决（见范围决定三）。

子 agent 侧（`local_agent_invocation.rs`）保留不完整标记：委托的假「已完成」最直接地
污染父 turn 的推理，且该模式已被 MaxTokens/MaxTurns/Refusal 建立并容忍，
partial text 与 usage 都保留。四重合取门有 5 个组合测试。

### 4. Conversation seam（原 blocker 10，已独立复核为真）

`stream_relay.rs` 的 `failed_terminal` 原本只是 `Error(_) || cancelled`，于是
`Finish{MaxTokens}` 会：进入 `process_final_text` → **执行**截断草稿里的 cron 命令
（cron 匹配是前缀正则，半句话就能触发）→ 产生 `system_responses` →
`service.rs` 铸造续跑 turn → 下一轮迭代**无条件重新赋值** `durable_completion`
（声明在循环外）→ 诚实的 `output_truncated` 被销毁 → `result_ok = 1`。
这就是 A1 本该修掉的生产 bug，从续跑门重新可达；A1 只教会了两个纯函数与一个消费者
关于 `stop_reason` 的事，没有触及 receipt 的**生命周期**。

- 收窄 `failed_terminal` 纳入任何 incomplete stop reason（`:3751-3755` 的注释已把这写成政策，
  MaxTokens 只是恰好落在谓词之外）。
- 在续跑铸造前加同源谓词 `break` 作 belt-and-braces。
- **但仍携带 `final_text`**（原始未处理文本）：cron 运行历史与跨会话投递通知都用
  `result_error.or(result_text)` 渲染人类可读原因，两列同时为 NULL 会把一个截断 turn
  降级成「unknown error」。对任何消费该值的判决都安全：`incomplete_stop_code` 为 `Some`
  时 `turn_succeeded` 恒假、`map_turn_failure` 在空文本检查**之前**报出该 code、
  knowledge write-back 由同一 stop-reason 谓词把门；`final_text_msg_id` 保持 `None`。

---

## 四、对抗复核结果

5 个独立 lens 提出 **44 项**发现，每项经 3 个不同角度（正确性 / 可达性 / 是否已被别处防住）
的反驳面板审查，多数反驳即淘汰。**42 项被反驳，2 项存活** —— 且两项是同一问题的两个视角。

复核过程中直接促成的修正（在存活项之外）：
`round.attempt` 未重置导致 section 污染每个后续 pass；重启谓词接受 turn 级证据；
`append_round_context` 与 `merge_pre_turn_context` 重复；两个同值为 24 的常量；
OpenAI 重复终帧双计 `argument_bytes`；`final_text` 置空导致 cron/投递通知失去原因；
plan/cutoff 渲染无界；标签含换行；`has_open_plan` 变成死代码；
一处被编辑操作粘连的函数签名行；"point A" 命名与既有 steering 注入点冲突。

### 唯一存活的缺陷（**已知、未修、非本阶段引入**）

**截断草稿已经流出去的文本没有被撤回。**

引擎的重启只从 `self.messages`（模型上下文）里 pop 掉草稿。那段文本早已通过
`emit_text_delta` 逐字流给了 sink → `StreamRelay`。relay **没有** `AgentStreamEvent::Start`
的处理分支（`:3307` 只是个调试标签，实际落到 `_ => forward_to_websocket`），
所以它的 `active_text` 段**不会在重启边界关闭**：第 2 轮的增量追加到**同一个段 id**，
即同一条持久行、同一个 UI 气泡。于是一次**成功**的重启会产生
`<半句草稿><完整答案>` 的单条消息，`outcome.final_text` 也把这段拼接带进
receipt `result_text`、knowledge write-back 与渠道回复。

**为什么不在 B1 修**：需要一个能区分「上一段部分输出已被丢弃」与「上一段完整输出仍然有效」
的信号。复用 `Start` 会破坏 steering race-tail —— 那里第二个 Start 是
**刻意**共享同一气泡的（`agent.rs:2037-2039` 明确记录）。新增协议事件会触及 ts-rs 与 UI，
属于 D1。

**为什么可以先发**：这是**既有**行为，且 B1 严格**收窄**了它。
改动前：每个带文本的 `MaxTokens` turn 都经宿主续跑拼接。
改动后：只有「本 pass 有工具调用被切断」才重启（`!cutoff.is_empty()`）。
出现频率严格下降。

**D1 的精确接缝**：`stream_relay.rs` 的 `active_text` 段生命周期与 `full_text_buffer`
（`finalize` 用它构造 `outcome.final_text`）。同仓已有一个把 `Start` 理解为
「新一轮使先前文本失效」的先例：`robot_wiring.rs:816` 的
`AgentStreamEvent::Start(_) => { self.candidate.clear(); ... }`。

---

## 五、已验证

本机需显式设置（loopback 请求否则被本地代理转成 502）：

```powershell
$env:NO_PROXY='localhost,127.0.0.1,::1'
$env:no_proxy=$env:NO_PROXY
```

并发 Cargo 进程会互删共享 incremental work-products（`os error 3`）：

```powershell
$env:CARGO_INCREMENTAL='0'
```

通过结果（全部 0 failed）：

| 命令 | 结果 |
|---|---|
| `cargo test -p nomi-providers` | 225（lib 183 + anthropic 16 + gemini 6 + openai 20） |
| `cargo test -p nomi-agent` | 703（lib 588 + 20 个集成 binary） |
| `cargo test -p nomi-agent --test badcase_regression_test` | 9（新增 3 个 B1 回归） |
| `cargo test -p nomifun-ai-agent` | 460（lib 447 + 6 个 binary） |
| `cargo test -p nomifun-conversation` | 611（lib 525 + 4 个 binary） |
| `cargo test -p nomifun-idmm` | 201 |
| `cargo test -p nomi-types` | 72 |
| `cargo test -p nomi-config` | 163 |
| `cargo test -p nomi-skills` | 439 |
| `cargo test -p nomifun-api-types` | 594 |
| `bun test` | 2230 passed / 404 files |
| `bun run check` | 全绿（typecheck、i18n 5425 键、theme、icons、dead CSS、三个边界检查、help 对齐） |
| `cargo clippy -p nomi-types -p nomi-providers -p nomi-agent -p nomifun-api-types --all-targets` | B1 触及的文件零告警 |
| `cargo fmt`（6 个改动 crate）+ `git diff --check` | 通过 |

### 环境形状，不是源码失败

`cargo test -p nomifun-ai-agent`（经 `protocol/events/mod.rs` 的
`export_binding_if_changed`）会把 `ui/src/common/protocolBindings/ProtocolDescriptor.ts` 与
`TurnCompletedEventData.ts` 重新生成，且生成器在 `boolean,` / `number,` 后**多输出一个尾空格**，
而已提交版本没有。这两个字段（`requires_output_ceiling`、`reasoning_tokens`）都是 C1 的，
已在 `122646f8` 提交。因此这是 **C1 遗留的生成器/格式化不一致**，与 B1 无关
（B1 不新增任何 ts-rs 类型）。每次跑完该 suite 需要
`git checkout -- ui/src/common/protocolBindings/{ProtocolDescriptor,TurnCompletedEventData}.ts`，
否则 `git diff --check` 会因尾空格失败。**值得单独修生成器或提交格式化后的版本。**

### 未运行

- `nomifun-app` 全 lib suite：未触碰，且有已知的「每次轮换一个测试」loopback flake。
- StepFun 生产凭证未用于 live 复现；本阶段用 production-shaped local HTTP 回归
  （`badcase_regression_test.rs` 用真 `AgentEngine` + 真 OpenAI-compatible 解析器 + wiremock）
  锁定 wire 与终态。

---

## 六、B1 在观测故障上的行为

provider `openai-compatible`、model `step-3.7-flash`、上限 8192、
`output_tokens = 24576 = 3 × 8192`、零工具调用、`result_ok = 1`、磁盘上什么都没有。

1. pass 1：`finish_reason: "length"` → `Done{MaxTokens}`。从未开始任何 call，
   `drain_truncated_tool_calls` 产出空。
2. 重启谓词：`cutoff` 为空 → **false**。不重启。
   **第 2、3 个 8192-token pass 不再发生**，`output_tokens` 从 24576 降到 8192。
3. 宿主：被删分支不再追加英文续跑 prompt、不再吞掉结果。
4. Receipt：`incomplete_stop_code(MaxTokens) = OUTPUT_TRUNCATED` →
   `result_ok = 0`、`result_error_code = "output_truncated"`、`result_error_retryable = 1`、
   `result_text` = 用户已看到的散文（未经 middleware）。
   收窄后的 `failed_terminal` 使 `process_final_text` **不再**在截断草稿上运行 ——
   既不会从半句话里执行 cron 命令，也不会铸造覆盖该 receipt 的续跑 turn。
   **「`result_ok = 1` 而磁盘上什么都没有」彻底消失。**
5. 反事实（本工作流存在的理由）：若模型在耗尽空间前正在写 `Write` ——
   `!cutoff.is_empty()` 为真：引擎 pop 掉草稿（它因此离开 provider 请求），
   在尾部原样重述原始 requirement，把 `[resumable round 2/3]` + 已声明计划 +
   `ok Bash: mkdir -p toolbox` + `Write (6142 bytes ... NOT executed)` 注入
   **system** prompt，重设回卷地板，带着 ledger 跑 pass 2。
   已由 `a_truncated_tool_call_restarts_against_the_original_requirement`
   （真 provider 解析器）与宿主侧同名测试端到端锁定。

---

## 七、明确不宣称

- **EndTurn-with-false-claim 未关闭。** A1 只保证「最终仍是 MaxTokens」的轮次不能记成功。
  B1 只记录四项事实、不裁决。通用假完成形状归 A2。
- **成功重启的可见文本仍含被丢弃的草稿**（第四节存活缺陷）。频率严格低于改动前，
  但未关闭；D1 的第一项。
- **distillation 未按 stop reason 把门。** `agent.rs` 的 `messages_transcript()` 覆盖整个
  消息向量，一个截断或被拒的 turn 仍会写基于文件的记忆。既有行为，B1 未改；
  conversation 侧的 knowledge write-back **是**按 stop reason 把门的。
  pop 掉草稿实际上**改善**了 distill 输入（截断散文不再进入），但 requirement 会出现两次。
- **`LedgerCutoff.state_changing` 只能来自 `category()`**：被截断的调用没有可解析的 input。
  一个被截断的只读 browser 动作会被记为 state-changing。仅影响观测与子 agent 门。
- **`cache_detector.record_request` 每个重启轮重跑**，可能为每个截断 turn 多产一条
  cache-break 诊断。纯诊断。
- **`tool_retry_tracker` 是循环外的**，重启轮共享它。纯截断轮不注册任何 id
  （截断调用从不进入 `tool_calls`），所以重启后重发同名调用是首次注册。
  只有当同一 `execute_turn_inner` 里更早轮次已完成过工具调用、且 provider 铸造确定性 id
  时才会碰撞 —— 与今天的多轮工具循环行为相同。

---

## 八、下一步

**D1 之前必须先读**本文件第四节（存活缺陷）与第五节（ts-rs 生成器不一致）。

D1 的建议顺序：

1. **可见边界事件**（关闭第四节的存活缺陷）。接缝：`stream_relay.rs` 的 `active_text`
   段生命周期 + `full_text_buffer`。需要一个区分「部分输出已丢弃」与「完整输出仍有效」
   的信号，因为 `Start` 已被 steering race-tail 用作「刻意共享同一气泡」。
2. **Continue 动作**：`output_truncated` 已经是 retryable 且**特指可恢复**的 code
   （`relay_error_code.rs` 的常量文档已这么写）。若 Continue 需要复用 ledger 跨进程，
   届时为 `RoundState` 加持久化并由那个真实消费者定义 fence 语义 —— 类型已就位。
3. **reasoning tokens 展示**：已到 `TurnCompletedEventData`，当前 UI 只合成总 token。
   若要单独展示，和 D1 一起设计，不要私自扩 persisted schema。

E1 最后做，因为它再次触碰 `LlmRequest` 与协议契约。
