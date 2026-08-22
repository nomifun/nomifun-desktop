# Nomifun Coding Agent 截断重构：C1 + A2b 阶段交接

日期：2026-08-20

分支：`fix/truncation-not-success`

C1 开始前 HEAD：`4b2fec1d`（其父提交 `00dee682` 已完成 A1/A2）

## 阶段边界

本阶段只完成并收敛：

- C1：输出上限从硬编码 `8192` 改成 task capability 的显式事实；支持协议级“省略”或“必填”。
- A2b：`content_filter` / provider refusal 不再误判成正常 `EndTurn`。
- C1 随附的 reasoning-token 可观测性。

本阶段没有开始 B1、D1、E1。尤其没有修改宿主的两次 MaxTokens 自动续跑策略，也没有实现 `openai.responses`。

## 已实现

### 1. Capability 与数据库成为唯一桌面事实源

- 新增 migration `036_provider_model_output_limit.sql`；该编号已进入主线并落库，Creative Studio 序列随后顺延为 037–046：
  - `provider_model_capabilities.output_limit INTEGER CHECK (output_limit IS NULL OR output_limit > 0)`；
  - 既有 `anthropic.messages` / `bedrock.anthropic_messages` chat 行回填 `8192`，保持升级前 wire 行为；
  - OpenAI-compatible / Gemini 保持 `NULL`，表示真正省略 wire 字段；
  - 清除既有 chat `provider_params` 中四个顶层 ceiling 别名、动态 `max_tokens_field` 指向的值，以及 `generationConfig.maxOutputTokens`。
- `output_limit` 已贯通 DB row/new row、repository upsert/change detection/clone、API、system service、gateway、model resolver、ts-rs bindings。
- 保存时拒绝 chat `provider_params` 暗路；resolver 重复校验腐坏或手改行。
- Anthropic/Bedrock authoring 缺少 `output_limit` 时在保存层拒绝；协议 manifest 暴露 `requires_output_ceiling`。

### 2. Runtime 与 wire 规则

- `LlmRequest.max_tokens: u32 -> Option<u32>`；`None` 必须是字段缺失，不是 JSON `null`。
- `Config.max_tokens` 改为 `Config.output_max_tokens: Option<u32>`；CLI/TOML 的显式 `max_tokens` 仍保留为 CLI surface，但没有默认 `8192` fallback。
- 桌面路径不再让 `~/.nomi/config.toml` 或项目 TOML 在 capability 为 `NULL` 时回填：`apply_provider_token_budget` 用 capability 值（包括 `None`）无条件覆盖。
- OpenAI-compatible / Gemini：`None` 时省略字段。
- Anthropic / Bedrock / Vertex：适配器在发请求前要求显式 ceiling，并给 CLI/TOML 与桌面各自可执行的错误文案。
- compat merge 前剥离所有顶层 ceiling 别名、OpenAI 动态字段和 Gemini 嵌套字段；typed request 是唯一 wire authority。
- `fit_context_budget` 与 compactor ceiling 解耦；`window_output_unit` 在 ceiling 缺失时仍提供非零输入 headroom。

### 3. 删除假旋钮和委托预算分叉

- 删除 `NomiBuildExtra.max_tokens` 与 UI storage 的死字段 `maxTokens`。
- 删除 `AgentInvocationInput.max_tokens`、skill fork 的 `16384`、local delegate 的 `4096`。
- 子 agent 从父 session 的 `base_config.output_max_tokens` 自然继承，并有专门回归测试。
- compact summary 预算改用同一个 window-derived unit，不再维护第二个硬编码输出预算。

### 4. Refusal 与 reasoning usage

- `nomi_types::StopReason` 新增 `Refusal`，所有穷尽 match 已收敛。
- OpenAI-compatible `content_filter` / `refusal` 和 Anthropic-family `refusal` 映射为 `Refusal`；不会再变成正常完成。
- `TokenUsage.reasoning_tokens` 贯通 OpenAI normalized/standard usage、Gemini thought usage、engine 累加、子 agent 汇总和 `TurnCompletedEventData`。
- 真 `AgentEngine` + 真 OpenAI-compatible serializer 的生产形状测试同时锁定：
  - capability ceiling 为 `None`；
  - compat 中存在四个别名与动态 key；
  - wire 上仍没有任何 ceiling key；
  - `finish_reason=length` 保持 `StopReason::MaxTokens`；
  - reasoning usage 不丢失。

### 5. 设置 UI

- 新增 `OutputLimitInput`，空值明确表示 provider default / wire omission。
- capability draft/load/save/clone 转换贯通 `outputLimit <-> output_limit`。
- manifest 声明 required 的协议缺值时禁止保存并显示中英文提示。
- 更新 ts-rs bindings、i18n 类型键和相关测试。

## 已验证

测试命令需要在本机显式设置：

```powershell
$env:NO_PROXY='localhost,127.0.0.1,::1'
$env:no_proxy=$env:NO_PROXY
```

并发 Cargo 进程偶尔会互删共享 incremental work-products；并发验证时加：

```powershell
$env:CARGO_INCREMENTAL='0'
```

通过结果：

- `cargo test -p nomi-types`：67 passed。
- `cargo test -p nomi-config`：151 passed。
- `cargo test -p nomi-providers`：219 passed。
- `cargo test -p nomi-agent --lib`：567 passed。
- `cargo test -p nomi-skills`：439 passed。
- `cargo test -p nomi-cli`：1 passed；`nomi-agent` 的 `browser-use` feature compile-check 通过。
- `cargo test -p nomifun-conversation --lib`：524 passed。
- `cargo test -p nomifun-idmm --lib`：201 passed。
- `cargo test -p nomifun-model-invoke`：354 unit + 8 manifest + 2 URL contract passed。
- `cargo test -p nomifun-db --test provider_capabilities_migration migration_46_declares_output_limits_and_removes_legacy_body_ceilings`：passed。
- `cargo test -p nomifun-db --test provider_repository`：17 passed。
- `cargo test -p nomifun-api-types --test ts_export`：2 passed。
- `cargo test -p nomifun-system provider_model`：11 passed。
- `nomifun-ai-agent`：provider protocol contract、三条 desktop token-budget case、stop-reason mapping 均 passed。
- `nomi-agent --test badcase_regression_test omitted_ceiling_cannot_be_revived_and_length_keeps_reasoning_usage`：passed。
- `bun test`：2230 passed，0 failed（404 files）。
- `bun run check`：完整通过（typecheck、i18n、theme、icons、dead CSS、边界检查等）。
- changed Rust packages 的 `cargo fmt` 与全差异 `git diff --check`：通过。

首次直接跑 `nomifun-model-invoke` 时 loopback 请求被本机代理转成 502；设置 `NO_PROXY` 后全绿。首次并发跑 conversation 时遇到共享 incremental 目录 `os error 3`；设置 `CARGO_INCREMENTAL=0` 后 524/524 全绿。这两项都是环境形状，不是源码失败。

`bun run test`（根脚本实际运行全仓 `cargo test`）完成全仓编译及绝大多数 suites；唯一失败是未触碰区域 `nomifun-app --test agent_integration_e2e::side_question_with_mock_agent` 的 companion store contract-version 初始化竞态（`expected 3, found 0`）。使用相同环境定向复跑该唯一失败后 1/1 passed，错误未复现。保留这次 broad command 的 exit 101，不把定向绿洗成“全仓命令全绿”。

## 仍然成立的边界与风险

- C1 只移除了没人声明的 `8192`。StepFun 等 optional 协议现在可能让单个 pass 使用 provider 自己更高的上限；宿主仍会最多自动续跑两次，因此单轮成本可能增加。B1 才拥有恢复/成本策略。
- A1 只保证“最终仍是 MaxTokens”的轮次不能记成功。后续 pass 若用非空文本谎称 `EndTurn` 且没有产物，仍可能成为假成功；不要在交付说明中宣称全部 false-success 路径已关闭。
- reasoning tokens 已到 `TurnCompletedEventData`，当前 conversation UI 只把 input/output 合成总 token，并未单独展示 reasoning；若要显示，和 D1 一起设计，不要私自扩 persisted schema。
- 外部 StepFun 生产凭证没有用于 live reproduction；本阶段用 production-shaped local HTTP regression 锁定 wire 与终态。

## 下一阶段：先做 B1

开始 B1 前必须重新读：

- `docs/handoffs/2026-08-20-truncation-refactor-resumable-round.md`
- `docs/handoffs/2026-08-20-truncation-refactor-rechecks.json` 的第 3、4 份复核及全部 `remaining_blockers`
- 本文件
- 原故障日志 `C:\Users\rika0\Downloads\会话01a0189e-9492-7a91-87f2-9d7cfc487fcf.md`

B1 不得直接照旧设计落代码。至少先收敛这些已验证阻塞：

1. 回卷锚点不能依赖 `EditableTurnCheckpoint.start_len`；自动压缩会在最需要恢复的长会话里清空它并替换 messages。
2. 不能把宿主重试循环整体搬进 engine；model-only 会话的 `max_turns=1` 会把现有两次恢复退化成零次。
3. requirement 必须保留为 `Vec<ContentBlock>`；不能把多模态任务降成 `String`。
4. 不能用 `truncate(start_len)` 删除已经展示且从 inbox 排空的 steering。
5. OpenAI `length` 后的 late tool fragment 当前会被 post-finish guard poison；必须明确选择“安全累加但绝不执行”或承认不可恢复，不能写一个永远跑不到的测试。
6. Anthropic 截断 tool call 必须 `mem::take` 到 truncated stash，使 `pending_tool_calls` 留空；直接删除 clear 会触发 terminal-shape protocol error。
7. no-progress 判决必须只在“本轮确实广告过 state-changing tools”时启用；plan-mode/model-only 不能因 `effects_ok=0` 被误杀。
8. `MaxTokens` 必须排除在短假完成的 hard verdict 之外，继续由 A1 映射为 retryable `output_truncated`。
9. effect 计数用不会被 24-entry render window 丢掉的 monotonic counters。
10. 检查 conversation middleware seam：MaxTokens Finish 不能经过 `process_final_text` 再开新 turn，覆盖诚实的截断 receipt。
11. terminal emission 不得在持有 engine mutex 时 `.await`。

B1 完成后再做 D1；E1 最后做，因为它再次触碰 `LlmRequest` 和协议契约。
