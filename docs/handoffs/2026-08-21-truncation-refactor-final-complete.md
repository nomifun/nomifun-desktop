# Nomifun Coding Agent 执行链路重构：最终完成交接

日期：2026-08-21

仓库：`C:\Users\rika0\code\nomifun\bak\1\nomifun-tauri`

分支：`fix/truncation-finalize`

代码基线父提交：`1f00647f fix(bindings): normalize generated TypeScript whitespace`

上一阶段交接：

- `docs/handoffs/2026-08-21-truncation-refactor-a2-finalization-handoff.md`（A2 收尾，列出唯一开放 P1）
- `docs/handoffs/2026-08-21-truncation-refactor-b1-d1-e1-complete.md`（B1 / D1 / E1）

## 一、本轮结论

上一份交接列出的**唯一开放 P1（accepted-turn 回滚未恢复隐藏 runtime authority）已关闭**。

在统一门禁复跑中另外发现并修复了一个**上一轮遗留的 P0 死锁**：provider 失败轮次会永久挂死，
而不是如实终态化。这正是上一份交接警告过的风险——最后几个并发分片合流后没有再跑统一
workspace 门禁，所以这条路径从未被执行过。

## 二、P1：accepted-turn runtime-authority 快照 / 恢复

### 设计

`CompletionEvidenceContext` 新增 `turn_authority: Option<AcceptedTurnRuntimeAuthority>`，
**与 `turn_root` 在同一个受保护步骤中捕获**（`capture_turn_root` 内部），因此：

- 二者不可能对不上；
- host race-tail 的第二次 pass 在同一函数提前返回，保留 pass A **之前**的快照；
- `turn_root.is_some()` 蕴含 `turn_authority.is_some()`，恢复函数因此不需要新的失败模式。

新增窄接口，均不克隆或替换活的进程 / shell / 共享句柄：

| 类型 | 接口 | 保留 |
| --- | --- | --- |
| `ToolConfirmer` | `authority_snapshot` / `restore_authority` | 用户显式 `[a]lways` 授权 |
| `HookEngine` | `hooks_config` / `replace_hooks_config` | `SupervisedShell` + process supervisor + cwd |
| `GoalRuntime` | `snapshot_state` / `restore_state` | 共享 `Arc<Mutex<GoalState>>` 本体（`UpdateGoalTool` 持有同一份） |

`StagnationGuard` 与 `CacheBreakDetector` 补 `#[derive(Debug, Clone)]`。

### 快照字段

`crates/agent/nomi-agent/src/engine/runtime_authority.rs`：

`model`、`thinking`、`current_reasoning_effort`、`compaction_level`、`compact_state`、
`allow_list`、`ToolConfirmerAuthority`、`HooksConfig`、`PlanState`、plan flag 值、
`GoalState`、`CacheBreakDetector`、`StagnationGuard`。

deferred ToolRegistry activation 继续复用既有 `AcceptedTurnRoot.activated_deferred_tools`，
没有另造第二套猜测。

### 恢复点

引擎内恰好 3 处，覆盖交接要求的全部形状：

1. `execute_turn_with_completion_evidence_context` 的 adjudication 分支——A2 与
   `SessionCommitFailed`；
2. 同函数 `result.is_err() && turn_started` 分支——直连 / CLI 的 provider Err；
3. `restore_uncommitted_completion_root`——host 的两个入口
   （`restore_uncommitted_completion_turn` = artifact / delivery / session commit failure，
   `restore_uncommitted_completion_attempt` = provider Err 与 cancel）。

审计确认没有第三套 root 恢复路径：`git grep` 下所有 `self.messages = root.messages`
与 `self.messages = safe_messages` 都已配对 authority 恢复。

恢复全部是内存赋值，不可失败；因此**既有的 checked session write 仍是唯一失败闸门**，
写失败照旧升级为 `HistoryRollbackFailed` / `NOMIFUN_AGENT_SESSION_INCONSISTENT` 并退休 runtime，
不会复用半恢复的 runtime。

### 明确不回滚（附证据）

- `total_usage`：供应商成本与 telemetry 必须为真。
- 已发生的 workspace / 外部副作用：本次只恢复 authority 与会话真相。
- **`ToolApprovalManager`（审计裁决）**：`auto_approved` 只由
  `approve(_, ApprovalScope::Always)` 写入，`session_mode` 只由 `set_mode` 写入，
  `add_auto_approve` 的唯一非测试调用点是 Browser 审批夹具。三者**全部**是显式人类决定，
  没有任何 skill / 模型路径可达。结论：这是用户偏好，不是 accepted-turn authority，
  **不回滚**。
- 同一裁决切分了 `ToolConfirmer`：skill 通过 `add_to_allow_list` 自授的 grant 回滚；
  交互提示里人回答 `[a]lways` 的 grant 存入独立 `user_always` 集合并保留。
  拒绝模型的假完成声明不应撤销人的决定。

### `thinking` / `compaction_level` 的审计结论

二者只被 `apply_config_update` 写入，而其唯一调用点（`nomi-cli`）在轮次进行中会
**排队**（`"set_config: queued, will apply after current response"`）并在 turn future
返回后才应用；desktop 后端完全不调用它。因此 mid-turn 不可能有用户决定与回滚交错，
纳入快照不会撤销任何人类选择——所以按"请求 authority"纳入，属 fail-safe 方向。

### `cache_detector` / `stagnation_guard` 的审计结论

`stagnation_guard` 在每个 `execute_turn_inner` 开头**无条件 reset**，因此本身不携带跨轮
authority；纳入属纵深防御，不改变行为。`cache_detector` 只影响 opt-in 的 INFO 级诊断，
但它记录的是特定 transcript 的 prompt 快照；transcript 回卷后保留旧快照会给下一次请求
误报 cache break，故一并恢复。

已知且有意的后果：`CompactState.consecutive_failures` 随轮次回卷，因此被拒绝轮次里的
autocompact 失败不计入熔断器。每轮仍受 `max_failures` 约束，无 runaway；这与
"回卷 transcript 就回卷由它派生的状态"一致。

## 三、P0（本轮新发现）：provider 失败轮次死锁

`crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`

accepted-turn 流程整轮持有 `tokio::sync::MutexGuard<AgentEngine>`，而 manager 的
`restore_uncommitted_completion_turn` / `_attempt` 取 `&self` 并**重新** `self.engine.lock().await`。
`tokio::sync::Mutex` 不可重入。

A2 分支与成功分支各自在调用前 `drop(engine)`；**`Err(e)` 分支没有**。因此每一个 provider
失败轮次都会永久挂死在 `restore_uncommitted_completion_attempt`——既不返回 Err，也不发终态，
表现为 `cargo test -p nomifun-ai-agent --lib` 中途"卡住"。

复现与定位：既有测试
`manager::nomi::agent::tests::provider_error_never_publishes_uncommitted_tool_progress`
本就覆盖这条路径，但它以**挂死**而非断言失败的方式表现，所以之前被当成编译慢。
用探针确认执行到 `Err` 分支后停在 engine 锁上。

修复：在 `Err(e)` 分支首行 `drop(engine)`，附不可重入原因注释。

同时新增有界回归测试
`a_provider_error_turn_releases_the_engine_lock_before_restoring`：用
`tokio::time::timeout` 把这一类死锁从"挂死 CI"变成"20 秒内失败"，并断言失败轮次不泄漏
engine guard、provider 不被静默重放。

审计其余 8 个 restore 调用点：全部被既有 `drop(engine)` 支配（编译器可证——其后
`self.engine.lock()` 能编过说明 drop 支配它）；cancel 分支直接在已持有的 guard 上调用引擎方法，
不重入。仅此一处破损。

## 四、变更清单

新增：

- `crates/agent/nomi-agent/src/engine/runtime_authority.rs`
- `crates/agent/nomi-agent/src/engine/runtime_authority_tests.rs`

修改：

- `crates/agent/nomi-agent/src/engine/mod.rs`：`turn_authority` 字段、原子捕获、
  `snapshot_runtime_authority` / `restore_runtime_authority`、3 个恢复点、mod 声明。
- `crates/agent/nomi-agent/src/confirm.rs`：`user_always` 溯源拆分、`ToolConfirmerAuthority`。
- `crates/agent/nomi-config/src/hooks.rs`：`hooks_config` / `replace_hooks_config`。
- `crates/agent/nomi-agent/src/goal/runtime.rs`：`snapshot_state` / `restore_state`。
- `crates/agent/nomi-agent/src/loop_guard.rs`、`cache_diagnostics.rs`：`Clone` 派生。
- `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`：P0 死锁修复 + 有界回归测试。
- `ui/src/renderer/services/i18n/i18n-keys.d.ts`：改为生成器输出（修 `check:i18n`）。

以上之外的 dirty 文件均为上一轮 A2 / artifact / consumer 收尾工作，本轮未改动其语义。

## 五、验证证据

环境（全程）：

```bash
NO_PROXY=localhost,127.0.0.1,::1
CARGO_INCREMENTAL=0
```

Cargo 全程串行，无并行。

### 变异检验（证明测试有牙）

- 临时禁用 `restore_runtime_authority`：10 个新 authority 测试中 **8 个回滚测试全部失败**，
  2 个"成功保留"测试仍通过（正确——它们断言变更被保留）。
- 临时移除 `drop(engine)`：新死锁回归测试在 **20.02s 内失败**（`Elapsed`），而非挂死。

两项检验后均已还原。

### 定向

- `cargo check -p nomi-agent -p nomi-cli -p nomifun-ai-agent -p nomifun-conversation -p nomifun-channel -p nomifun-cron --all-targets`：通过。
  唯一 warning 在未触及的 `nomifun-terminal/src/pty.rs`（Windows 下 `env` 未使用），为既有项。
- 新 `engine::runtime_authority_tests`：10 / 10。
- `nomifun-ai-agent` 死锁回归：1 / 1。

### 包级（全部 0 failed）

| 包 | 结果 |
| --- | --- |
| `nomi-agent`（lib + 全部 integration binary） | 636 + 245 |
| `nomi-config` | 165 |
| `nomi-types` | 73 |
| `nomi-tools` | 356 |
| `nomi-skills` | 439 |
| `nomi-cli` | 8 |
| `nomifun-ai-agent --lib` | 481 |
| `nomifun-conversation`（lib + integration） | 625 |
| `nomifun-channel`（lib + integration） | 482 |
| `nomifun-cron` | 263 |
| `nomifun-idmm --lib` | 201 |
| `nomifun-api-types` | 597 |

`nomi-types` / `nomi-tools` / `nomi-skills` / `nomi-cli` 这一组正是上一份交接中"exec session
已丢失、不可宣称通过"的那一批，本轮已如实重跑。

### 应用与 UI / 发布门禁

- `cargo test -p nomifun-app`：**815 passed / 0 failed**。已知的 loopback 轮换 flake
  本轮未出现。
- `bun test`：**2256 pass / 1 fail**，共 2257 across 405 files。
  唯一失败为 `ui/src/renderer/pages/knowledge/KnowledgeRetrievalSettingsModal.test.ts:87`
  （断言 `modalStyles.includes('height: 40px')`）。该测试与其 CSS module **均未被本分支触及**
  （`git status` 对 `ui/src/renderer/pages/knowledge/` 为空），最后修改者是上游
  `1c5f214c style(ui): unify modal visual contract`。属上游 modal 改版遗留，非本轮引入，
  **未修复**。
- `bun run check`：**全绿**（typecheck、i18n 5445 keys / 38 modules、theme、icons、
  dead CSS、process-runtime boundary、browser-platform boundary、agent vocabulary、help）。

  本轮修复了一处真实门禁失败：`check:i18n` 报
  `key sets match but file text differs (ordering/header/EOL)`——上一轮是**手工编辑**
  `i18n-keys.d.ts` 而非生成。执行 `bun run gen:i18n` 后该文件恢复为生成器输出，
  diff 收敛为预期的 4 个新 key（`NOMIFUN_AGENT_SESSION_INCONSISTENT` 与
  `USER_LLM_PROVIDER_UNBACKED_COMPLETION` 各 body/title），排序正确。

### 生成物与工作树

- `cargo test -p nomifun-api-types --test ts_export` 连跑两次：各 3 / 3，**零 binding 漂移**；
  `nomifun-ai-agent --lib` 481 项亦未改动任何 protocol binding。`1f00647f` 的生成器修复成立。
- `git diff --check`：通过。
- 全部 dirty 文件行尾审计：**0 个含 CRLF**（`.gitattributes` 要求 worktree 亦为 LF）。
- 无冲突标记、无 `eprintln!` / `dbg!` / `PROBE` / `#[ignore]` 残留。

### 合并后复验（`main` = 6668e012）

合并采用 `--no-ff`（与仓库对该分支族的既有惯例一致）。**合并结果的 tree hash 与被测分支
tip 逐字节相同（`780a7f15`）**，因此不存在内容漂移；在此基础上另外复验：

- `cargo check --workspace --all-targets`（**57 个包，含全部 test target**）：**0 error**。
  这排除了本轮新增 4 个公共 API（`ToolConfirmer::authority_snapshot/restore_authority`、
  `HookEngine::hooks_config/replace_hooks_config`、`GoalRuntime::snapshot_state/restore_state`、
  `CompletionEvidenceContext::turn_authority`）造成签名破坏的整类风险。
- `nomi-agent --lib` 636、`nomifun-ai-agent --lib` 481、新 authority 10、死锁回归 1：全通过。

### 高风险子集补跑（与 terminal-truth 路径行为耦合）

| 包 | 结果 |
| --- | --- |
| `nomi-protocol` | 66 |
| `nomi-compact` | 47 |
| `nomi-providers` | 243 |
| `nomifun-model-invoke` | 371 |
| `nomifun-terminal` | 131 |
| `nomifun-robot` | 139 |
| `nomifun-system` | 275 |
| `nomifun-db` | 756 |

合计 **2028 项，0 failed，无挂死**。

另补跑上一份交接点名的其余 `LlmEvent` 消费者（同一风险类别）：

| 包 | 结果 |
| --- | --- |
| `nomifun-agent-execution` | 73 |
| `nomifun-companion` | 274 |
| `nomifun-creation` | 55 |
| `nomifun-knowledge` | 272 |

合计 **674 项，0 failed**。

### 覆盖率核算（诚实记账）

- **编译**：`cargo check --workspace --all-targets` 覆盖 **57 / 57** 个包及其全部 test target，0 error。
- **运行时测试**：**26 / 57** 个包。其中：
  - **源码被改动的 12 个包：100% 覆盖**（`nomi-agent`、`nomi-cli`、`nomi-config`、`nomi-skills`、
    `nomi-tools`、`nomi-types`、`nomifun-ai-agent`、`nomifun-api-types`、`nomifun-app`、
    `nomifun-channel`、`nomifun-conversation`、`nomifun-cron`）。
  - 另 14 个未改动但与 agent / `LlmEvent` / terminal-truth 语义有行为耦合的包已覆盖。
- **未跑运行时测试的 31 个包**：源码未改动、编译干净、且两份交接文档均未将其列为耦合面。
  典型为 `nomi-browser*`、`nomi-ssh`、`nomi-mcp`、`nomi-computer`、`nomi-a11y`、`nomi-redact`、
  `nomifun-office`、`nomifun-miniapp`、`nomifun-web`、`nomifun-webhook`、`nomifun-workshop`、
  `nomifun-extension`、`nomifun-auth`、`nomifun-gateway`、`nomifun-realtime` 等外围能力包。

**这不等同于交接文档要求的完整 `bun run test` 门禁。** 残余风险已收窄为"未改动 + 编译干净 +
无已知耦合"的一类；本轮那个 P0 死锁位于**源码被改动的**包内，而该集合现已 100% 覆盖，
但这是论证，不是完整门禁的替代品。

### 关于 rustfmt 的诚实记录

**本仓库的 rustfmt 门禁是空操作，不能作为通过证据。** 根 `rustfmt.toml` 显式设置：

```toml
disable_all_formatting = true
```

并在注释中说明这是有意为之（工作区过大、未固定 toolchain 策略，裸跑 `cargo fmt`
会重写数百个文件并淹没真实改动）。因此 `cargo fmt -p ... -- --check` 对
`nomi-agent` / `nomi-config` / `nomifun-ai-agent` 三个包均以 exit 0 通过，但那只是因为
格式化被全局关闭——不代表任何格式被校验过。已用独立探针确认 rustfmt 二进制本身工作正常
（在仓库外文件上能正确报 diff）。

上一份交接把"touched-package rustfmt"列为门禁；该项应视为**不适用**，而不是通过。
新代码的排版是按周边代码手工对齐的。


## 六、已知限制与未运行项

- **rustfmt 门禁不适用**（见上）：`rustfmt.toml` 全局 `disable_all_formatting = true`。
- Windows 上 `cargo fmt --all -- --check` 另有路径长度 `os error 206` 问题；即便重新启用
  格式化也需按包执行。
- `bun test` 保留 1 项上游 modal 改版失败（`KnowledgeRetrievalSettingsModal`），非本轮引入。
- 未做 live provider 复现（无真实 OpenAI / StepFun 生产凭证）；Responses 仍由严格本地 SSE
  fixture 与生产 serializer 覆盖。
- 未以单条 `bun run test` 命令产出 exit 0 的整体证据。本轮按包串行跑完 26 / 57 个包并逐项
  记录（改动集 12 个包 100% 覆盖 + 14 个耦合包），没有把分段绿色洗成单命令绿色，也没有把
  "编译干净"说成"测试通过"。剩余 31 个包仅有编译证据。
- `nomifun-conversation` 与部分 channel / app integration binary 中单个测试耗时超过
  60s（SQLite / loopback 夹具），会打印 "has been running for over 60 seconds"。这是慢，
  不是挂；全部完成且 0 failed。真正的挂死表现为**永不完成**（见第三节）。

## 七、后续建议

- 本轮 P0 暴露的类别值得一次专门审计：任何在持有 `MutexGuard<AgentEngine>` 期间调用
  manager `&self` 异步方法的分支都可能重入。可以考虑让 manager 的 restore helper 改为接收
  `&mut AgentEngine`，把重入变成编译期错误，而不是靠调用点纪律。
- 慢测试夹具（conversation service_test）值得单独优化，目前会掩盖真实挂死。
