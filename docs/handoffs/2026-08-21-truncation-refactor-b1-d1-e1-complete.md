# Nomifun Coding Agent 截断重构：B1 / D1 / E1 阶段完成交接

日期：2026-08-21

分支：`fix/truncation-not-success`

提交链：

- `122646f8`：C1 + A2b，输出上限 capability 化、Refusal 终态与 reasoning usage。
- `efcc7fd2`：B1，引擎内可恢复轮次与截断工具调用事实。
- `121c04ca`：B1 剩余输出撤回、D1 用户续跑、E1 `openai.responses` 链接及相邻假成功加固。

## 阶段结论

本轮要求的 B1 剩余、D1、E1 已实现并提交。当前系统已经从“截断会被如实报成失败”推进到：

1. 被截断且可证明未完成的工具轮次可在引擎内安全重启，旧草稿会从模型上下文、持久消息、实时 UI、渠道和 AutoWork 输出中一致撤回。
2. 无法自动恢复的 `output_truncated` / `turn_requests_exhausted` 会留下持久恢复卡；用户可显式“继续执行”，后端从不可变 delivery receipt 恢复原始文本与多模态附件。
3. OpenAI 原生 Responses 协议可显式启用 `previous_response_id` 链接；服务端留存、cursor 与请求级授权使用同一双门，sidecar、压缩、health 等调用保持 `store:false`。
4. 多个直接消费 `LlmEvent` 的旁路不再把 MaxTokens、Refusal、无 Done 的 partial stream 或 post-Done 数据当成成功。

**仍未关闭的核心风险只有 A2 的零写入 EndTurn 假完成裁决。**不要对外宣称“所有假成功都已关闭”。

旧的三份设计和 `rechecks.json` 保留为历史证据，不再是当前实现规范。尤其 Responses 旧设计中的“只看 compat 开关”“不改 `LlmRequest`”“17 个 literal”等结论已经被隐私边界复核推翻；实际实现是请求级双门和 19 个真实 literal。

---

## 一、B1 剩余：被丢弃草稿的全链路撤回

### 新协议事件

新增 `AgentStreamEvent::OutputDiscarded` / `OutputDiscardedEventData { restart_attempt }`，并贯通：

- `OutputSink` trait 与 null / terminal / protocol sink；
- Nomi protocol 与 ts-rs binding；
- Conversation relay、Channel relay、Robot、AutoWork；
- 后端 held-text、stage-direction filter 与 UI message merge。

`Start` 只建立 discard checkpoint，不代表撤回。steering race-tail 会再次发 `Start` 且必须共享同一气泡，所以 B1 restart 使用独立 `OutputDiscarded` 事件。

### checkpoint 与回滚语义

Conversation relay 在 `Start` 保存经过真实 filter 后的状态，而不是 provider 原始字节长度：

- `full_text_buffer` 长度；
- 已落库 text segment 数量；
- active text id 与 buffer 前缀长度；
- stage-direction filter 状态；
- backend held text。

收到 `OutputDiscarded` 后：

- active segment 截回 checkpoint 前缀；
- checkpoint 后的新行以同 id `replace + hidden` 撤回；
- `full_text_buffer` 和 `text_segments` 同步回卷；
- active thinking 先持久化 hidden/error/done 并等待 DB ack，再关闭实时 spinner；
- 任一持久化失败都 fail closed，不能继续接收后续“成功”事件；
- checkpoint 随后更新为下一次 attempt 的起点，支持多次重启。

前端同 id text merge 已改为传播 incoming `hidden` / status envelope，因此撤回不会留下空白可见气泡；live 与 hydrate 形状一致。

### 相邻输出消费者加固

以下路径现在只允许 clean EOF 上唯一、非空的 `EndTurn` 成功；MaxTokens、Refusal、ToolUse、MaxTurns、ToolUseTruncated、ProviderRoundId、无 Done partial 与 post-Done 数据均显式失败：

- `nomifun-ai-agent/src/one_shot.rs`；
- `nomifun-ai-agent/src/factory/provider_config.rs` 的两个 text/reasoning drain；
- `nomi-agent/src/compact/auto.rs`；
- `nomi-agent/src/bootstrap.rs` 的 extract / visual locator；
- `nomifun-ai-agent/src/image_generation.rs` intent classifier。

Robot 侧也已 fail closed：

- `SpokenReplyReducer` 仅 `None` / `EndTurn` 进入成功播报；MaxTokens、MaxTurnRequests、Refusal、Cancelled 清草稿并发 `Failed`。
- cold-runtime 无 stream、broadcast `Lagged`、无 terminal 的 `Closed` 不再映射为 `Done`。
- delivery completed 只按权威 receipt 的 `result_ok` / error 映射。

---

## 二、D1：持久提示与显式继续执行

### API 与原子 admission

新增：

```text
POST /api/conversations/{conversation_id}/messages/{source_message_id}/continue-truncated
Idempotency-Key: <required>
```

请求不提交用户正文。后端从 source delivery receipt 的 typed payload 恢复原始 requirement 与 files，并在同一 SQLite writer transaction 内完成：

- owner / conversation / source message 校验；
- source receipt 必须 completed、`result_ok=false`、retryable，错误码只接受 `output_truncated` 或 `turn_requests_exhausted`；
- source 必须是最新 public turn，conversation 必须 idle；
- continuation receipt 插入；
- Finished / Pending 到 Running 的 epoch CAS admission。

replay 先按 operation id 返回已有结果，再做 stale 校验；双击与跨连接并发只有一个 execution owner。

### durable payload 决定

只接受两个严格版本的 source envelope：

- `public-turn:v1`；
- `public-truncation-continue:v1`。

continuation receipt 同时保存：

- `original_delivery`：最初用户请求的不可变文本、files 等；
- `delivery`：本次实际提交的 hidden continuation。

因此“截断 → 继续 → 再截断 → 再继续”始终从同一原始多模态请求恢复，不会把 recovery prompt 层层嵌套。原始 receipt 永远保持失败，每次 continuation 有独立 receipt。

有意决定：

- 不重放 `inject_skills`，避免安装类副作用；现有 session skill 状态继续生效。
- 本次点击清空 origin / channel platform，并写 hidden user turn。
- owner-visible initial public turn 与普通 public turn 共用现有 namespace / schema，**有意允许续跑**；AutoWork、agent execution 和其他 internal namespace 严格拒绝。
- 不自动重试。工具副作用无法证明 exactly-once，恢复必须由用户显式点击。

### 恢复卡与 UI

截断 Finish 在 receipt-backed public turn 上持久化并广播 canonical tips row：

- i18n code：`OUTPUT_TRUNCATED` / `TURN_REQUESTS_EXHAUSTED`；
- `error.retryable=true`；
- recovery metadata：kind、source message id、lowercase failure code。

UI 已贯通 live transform 与 persisted normalization，且 endpoint 只信 receipt，不信卡片 metadata。专用“继续执行”按钮只在以下条件同时成立时显示：

- Nomi owner；
- error tips；
- retryable 为 true；
- tips error code 与 recovery failure code 一致；
- 非 readonly；
- conversation idle。

按钮 pending 后立即锁定；stale 409 表示已被后续轮次取代。此卡不会触发 generic edit-resubmit，因此不会 destructive rewind 已完成工具证据，也不会丢附件。

同时将 `input_tokens` / `output_tokens` / `reasoning_tokens` 写入现有 `conversation.extra.last_token_usage`，Context Usage popover 展示 output 与 reasoning；ring 仍只表达 context occupancy。

---

## 三、E1：OpenAI Responses 原生协议与安全链接

### 类型、manifest 与 resolver

新增：

- `ProviderType::OpenAIResponses` / alias `openai-responses`；
- `openai.responses` Agent/Chat/Http native-only protocol，只允许 OpenAI platform 与 bearer auth；
- `LlmRequest.retain_provider_round: bool`；
- `Message.provider_round_id: Option<String>`；
- 独立 `LlmEvent::ProviderRoundId(String)`，没有给约 208 个 `Done` 构造点加字段；
- `ProviderCompat.chain_rounds` typed boolean。

Agent 协议启动白名单与“只有四族”的契约测试已更新为五族。Chat recommendation 仍是 `openai.chat_text`，没有更改公共 URL snapshot，也没有 DB migration。

endpoint owner 对称校验：

- `openai.chat_text` 不能保存 `/responses`；
- `openai.responses` 不能保存 `/chat/completions`；
- query、fragment、trailing slash 与大小写先规范化；其他明确 custom path 仍允许；
- 不做 Responses → Chat fallback。

`chain_rounds` 只允许 `(openai.responses, Chat)` 的 boolean；保存与 resolve 都拒绝跨协议或非 bool 值，factory typed 提取后不会把它泄漏进 `extra_body`。

### 留存隐私双门

Responses 只有同时满足：

```text
compat.chain_rounds() && request.retain_provider_round
```

才会：

- 发送 `store:true`；
- 读取 / 发送 `previous_response_id`；
- 接受并发出 `ProviderRoundId`。

长期 AgentEngine 是唯一生产 true 路径。autocompact、bootstrap、image generation、one-shot、title、knowledge、planner、companion、creation、robot、IDMM 等共享 provider 的请求均显式 false；provider health 也有意不复制 chain flag。false 路强制 full snapshot、`store:false`、零 cursor。

这是隐私授权边界：Responses 默认可留存，开启链接会发送 `store:true`；设置 UI 中英文文案明确说明供应商可能至少保留 30 天。`previous_response_id` 不会减少链上历史 input 的计费，只减少客户端重建与传输，UI 没有宣传省 token / 费用。

官方参考：

- <https://developers.openai.com/api/docs/guides/conversation-state>
- <https://developers.openai.com/api/docs/guides/your-data#default-usage-policies-by-endpoint>

### chain 与 wire

- parent 只看最新 assistant；没有 id 时绝不向前寻找旧 id。
- parent 后必须有非空结构 delta，且实际序列化后的 wire input 也必须非空，否则 full snapshot。
- chain request 只发 suffix，典型为 `function_call_output` 与 steering；bootstrap / stale fallback 发完整 snapshot。
- 每次都重发 `instructions`，因为它不从 previous response 继承。
- function tool schema、`function_call` 与 `function_call_output` 使用 Responses 扁平 wire；工具图片放 output content array。
- reasoning 使用 `reasoning:{effort}`，并请求 / 版本化保存 `reasoning.encrypted_content` 作为 opaque replay signature；不会把本地明文 Thinking 伪造为 native reasoning item。
- typed body 在 merge 后重新剥离 `store` / `previous_response_id` / ceiling 等影子键，`extra_body` 不能复活本地授权字段。

stale parent fallback 只在 400/404 且 bounded JSON error 同时明确指向 previous response id 和 missing / expired / deleted / not-found 语义时启用；generic endpoint 404 给出不支持 `/responses` 的诊断，绝不降级到 Chat。schema sanitize 与 stale fallback 各单调一次，最多三种 distinct body。

### 严格 SSE 状态机

新 provider 不复用 Chat parser：

- raw byte framing，完整 frame 后才 UTF-8 decode；支持拆分 UTF-8、CRLF 与 multiline data；
- named `event:` 必须与 JSON `type` 一致；sequence number 严格递增；
- output/item/content identity、index、status 与 terminal output 双向一致；
- `content_index < 128`，每个 message 的 text + refusal 跨 frame 聚合不超过 512 KiB；
- exactly one terminal；cursor 只在所有终态校验后、紧邻唯一 Done 前发出；clean EOF 前不提交 terminal events；
- 200 普通 JSON、`[DONE]` 或 clean EOF 若无合法 terminal 均 fail closed；post-terminal 数据报错。

`incomplete/max_output_tokens` 下，所有观察到或 terminal-only 的 function call 都只转成 `ToolUseTruncated`，发零 `ToolUse`、零 tool cursor，再 `Done(MaxTokens)`；因此残缺或“看起来完整”的未解决工具调用都不会执行或被链接。content filter / refusal 映射为 `StopReason::Refusal`。

---

## 四、验证与门禁

本机验证统一使用：

```powershell
$env:NO_PROXY='localhost,127.0.0.1,::1'
$env:CARGO_INCREMENTAL='0'
```

主要通过结果（全部 0 failed）：

- `cargo test -p nomi-types`：68。
- `cargo test -p nomi-config`：154。
- `cargo test -p nomi-providers`：Responses 专测 17；provider 全套 243。
- `cargo test -p nomi-agent`：全套通过，含真 provider parser 的截断重启 E2E。
- `cargo test -p nomifun-ai-agent`：全套通过。
- `cargo test -p nomifun-conversation --lib`：535；CRUD / extended / relay 集成也通过。
- `cargo test -p nomifun-idmm --lib`：201。
- `cargo test -p nomifun-model-invoke`：357，manifest / URL contract 通过。
- `cargo test -p nomifun-db`：lib 374；全部 integration binary 通过；合并后的 migration 046 为 7/7 中的一项。
- `cargo test -p nomifun-system --test agent_chat_protocol_contract`：3。
- `cargo test -p nomifun-system --test provider_routes`：11。
- `cargo test -p nomifun-terminal --lib`：130。
- workspace 尾段：`nomifun-web` 9、`nomifun-webhook` lib 9 + integration 8、`nomifun-workshop` 49。
- `bun test`：2243 passed / 0 failed。
- `bun run check`：通过；typecheck、i18n 5441 keys、theme、icons、dead CSS、process/browser boundary、agent vocabulary、help 全绿。
- 15 个触及 Rust package 的 `cargo fmt --check` 与 `git diff --check`：通过。

### broad command 的诚实记录

`bun run test` 根脚本会执行完整 `cargo test`。本轮运行两次：

1. 第一次跑到 `nomifun-app::system_provider_e2e` 时，旧 Anthropic 保存夹具缺少 C1 现在强制要求的 `output_limit`，201 断言收到 400。补齐测试 payload 后该 binary 6/6。
2. 第二次从头跑过 App、Auth、Browser、Channel、Conversation、Creation、Cron、DB、Extension、Knowledge、MCP、Requirement、Robot、SSH 等大量全套，并通过先前失败点；到 `nomifun-system::provider_routes` 时另一个旧 Bedrock 保存夹具同样缺 `output_limit`。补齐后该 binary 11/11。

随后静态扫描确认仓内所有 `anthropic.messages` / `bedrock.anthropic_messages` 保存夹具均已显式声明 `output_limit`，并按 workspace 顺序补跑失败点后的四个 package。Terminal 首次暴露两个 Windows-only 测试夹具使用不存在的 `cmd.exe printf`；只将测试命令改为原生 `<nul set /p ... & exit /b 0`，生产代码未动，之后 Terminal 130/130，Web/Webhook/Workshop 全绿。

因此覆盖范围已闭合，但**没有把分段绿色洗成“某一次 `bun run test` 单命令 exit 0”**。若下一阶段需要发布级单命令证据，可在缓存稳定时再跑一次。

---

## 五、已知未完成项

### A2：零写入 EndTurn 的未支撑完成声明

`engine/mod.rs::unbacked_completion_claim` 仍有：

```rust
let first_write = first_write?;
```

当模型零 ToolUse、非空 EndTurn 文本声称“文件已创建”时，`first_write` 为 `None`，该函数直接返回 `None`，裁决被跳过；Conversation 仍可能记录 `result_ok=1`，磁盘没有产物。

不能用“Info 工具 = 无副作用”做粗判：fork 模式的 Skill 委托可通过 Info 类 Skill 工具完成真实文件/exec 工作。下一阶段必须先测绘可证明的 completion evidence、fork/child receipts、artifact receipts 与直接工具 effects，再设计不会误杀真实委托结果的判决。不要因 A2 未完成而回退本轮已经建立的 stop-reason truthfulness。

### ts-rs 生成器尾空格

`nomifun-ai-agent/src/protocol/events/mod.rs` 与 `nomifun-api-types/tests/ts_export.rs` 的 helper 直接比较 / 写入 `ts-rs export_to_string` 原文。带 doc comment 的 inline struct 字段会生成行尾空格，因此相关测试会反复改写：

- `ProtocolDescriptor.ts`；
- `TurnCompletedEventData.ts`；
- 新的 `OutputDiscardedEventData.ts`。

本提交前已还原 / 清理生成噪声，`git diff --check` 通过。后续应在两个 helper 中统一做 deterministic 行尾空格 normalization 并整体 regenerate；不要只反复手工修生成文件。

### 未做 live provider reproduction

没有使用真实 OpenAI / StepFun 生产凭证。Responses 由 strict local SSE fixtures 与 production serializer 覆盖；原 StepFun 故障形状由 OpenAI-compatible production-shaped regression 覆盖。

---

## 六、下一阶段建议

主线只做 A2：关闭零写入 EndTurn 假完成，并保持 fork Skill 的真实外部工作可被承认。开始前先：

1. 读本文件与 `2026-08-20-truncation-refactor-b1-complete.md` 中宿主硬裁决为何未落地。
2. 用 `rg` 枚举 `unbacked_completion_claim`、`turn_succeeded`、artifact / tool / child-agent receipts、fork Skill completion 的所有生产与测试调用面。
3. 先写生产形状回归：零工具 EndTurn 声称已写文件必须失败；fork Skill 确有 durable effect 时必须成功；普通问答不能被 artifact policy 误杀。
4. 不增加 legacy shim、dual read/write 或 fallback。
5. A2 收口后再单独修 ts-rs normalization；两项分开提交。
