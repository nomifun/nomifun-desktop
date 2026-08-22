# Nomifun Coding Agent 执行链路重构：A2 最终收尾交接

日期：2026-08-21

仓库：`C:\Users\rika0\code\nomifun\bak\1\nomifun-tauri`

当前分支：`fix/truncation-finalize`

当前 HEAD：本交接文档提交；其代码基线父提交为
`1f00647f fix(bindings): normalize generated TypeScript whitespace`。

本地 / 远程 `main`：`9856516405df9be44c3f79734a0c4ec43e9a1115`

## 一、当前工作树状态

- 51 个 tracked 文件有未提交改动；没有 staged 或 untracked 文件。
- 当前 diff 约为 `8127 insertions / 621 deletions`。
- `git diff --check` 通过。
- 没有正在运行的 `cargo` / `rustc` 进程。
- runtime-authority snapshot **尚未开始编码**，因此当前树没有这项工作的半写状态。
- 最新并发分片分别有定向编译 / 测试证据，但合流后尚未再跑一次统一 `cargo check`，不能把当前树描述成“已发布就绪”。
- 不要 reset、checkout 或覆盖当前 dirty worktree；这些改动全部属于本轮 A2 / artifact / consumer 收尾。

已单独提交且稳定的生成器修复：

- `1f00647f`：统一 ts-rs TypeScript 导出文本的换行和行尾空白；二次生成幂等。

上一阶段完整交接：

- `docs/handoffs/2026-08-21-truncation-refactor-b1-d1-e1-complete.md`

## 二、本轮已经完成的实现

### 1. A2 零写入假完成裁决

- 新增 typed `CompletionAdjudication`，高精度识别“明确要求修改文件，但模型宣称完成且没有机器证据”的 EndTurn。
- 原始 requirement 来自 admission 的用户正文，不解析 RAG / KB prelude。
- requirement、steering 和证据在同一个 accepted turn 内跨 host race-tail 累加；纠正 nudge 每 accepted turn 最多一次。
- 英文 / 中文 classifier 已覆盖：明确 mutation、否定、引用、历史、模态、未落盘、取消后重加、路径空格 / Markdown / dotfile、通用 `Done`、`ready`、`now contains`、`successfully` / `as requested` 等高置信形状。
- Unicode span 使用保持字节长度的规则，避免用户输入触发 UTF-8 slice panic。
- 最终 typed A2 verdict 使用 accepted-turn 全轮 `OutputDiscarded { restart_attempt: 0 }`；中间 nudge / B1 restart 仍使用 rolling attempt checkpoint。

### 2. 机器证据闭环

- 本地 workspace：accepted-turn baseline → terminal final fingerprint；仅显式高置信目标，限制 32 个目标、16 MiB / 文件、64 MiB / 批次。
- 使用稳定 open handle、`same-file` identity、长度 / modified / SHA-256 与 canonical containment；symlink escape、非 regular file、资源不足均 fail closed。
- 终态 final state 是权威，旧 Write receipt 不能覆盖同轮 delete / revert。
- 远端 SSH：只接受相关成功 Write / Edit / ApplyPatch 的 exact receipt；opaque Bash 不猜路径。
- 任意后续失败或 opaque state-changing writer 会使旧 terminal exact receipt 失效。
- `ToolCategory::Edit` 已纳入 workspace side-effect 语义。
- Skill fork / `nomi_delegate` 使用 operation-id sidecar 传递可信 child exact targets；失败、timeout、panic、isolated worktree 均清空证据。
- `LocalDelegateTool` 不再把失败 child 或未应用 isolated patch 当父 workspace 效果。

### 3. accepted-turn 会话事务

`Session` 已增加：

- `accepted_turn_root`；
- `pending_host_terminal_root`；
- `last_interrupted_turn_source`。

实现语义：

- provider 首次 await 前原子持久化 exact root：messages、editable checkpoint、host context、deferred tool activations。
- desktop 成功路径在 artifact / terminal 可见前将 accepted root 原子 seal 为 pending host-terminal root。
- 进程崩溃且 Conversation receipt 仍 Running 时，boot healer 使用统一 strict recovery API 精确回卷。
- strict recovery 覆盖 matching accepted / pending root、同 source editable checkpoint、AlreadyRewound、pre-poll ProvenUnchanged；mismatched accepted fail closed，上一成功 turn 的 prior pending root 保留。
- 旧的宽松 `rewind_owned_turn` / 双轨 enum 和 API 已删除。
- 普通 provider Err、cancel、artifact / distill / session commit failure 都有 checked root restore；持久化失败升级专属 `NOMIFUN_AGENT_SESSION_INCONSISTENT`，runtime 退休并在 durable receipt 释放前严格恢复。
- `record_host_text_turn`、clear、rewind 使用 checked atomic session write，并在写失败时恢复内存。
- ToolSearch deferred activations随 root 精确恢复。

### 4. 输出撤回

- Conversation、BackendOutputSink、Channel send-once / editable 都有两套 checkpoint：
  - immutable accepted-turn first-Start root；
  - latest rolling attempt checkpoint。
- `restart_attempt == 0` 回 accepted root；正数回 rolling checkpoint。
- race-tail A / B 后最终 A2 或 host transaction rollback 会撤回整轮文本，不留下 pass A。
- B1 / provider retry 保留合法 prefix。
- Backend 在没有 Start 且没有 `turn_text` / `held_text` / active provisional tools 时，discard 是 no-op；存在未锚定 provisional state 时仍发 fail-closed legacy Error。
- active thinking、stage-direction filter、held text、持久化 text row 和 live UI 的撤回路径已闭合。

### 5. artifact 事务与 TOCTOU

- production accepted turn 的 image / file / audio / video / declared path artifact 全部 deferred；manager final lifecycle CAS 前不广播成功卡、不留下正式 artifact 文件。
- Backend declared-path baseline 限制 16 MiB / 文件、64 MiB / 批次，稳定校验 handle/path identity、长度和 modified。
- ArtifactStore 使用稳定 open-handle read；`VerifiedExistingArtifactSource(path, sha256)` 将 preflight digest 贯穿 sync / deferred / recoverable import，commit 前再次校验。
- filter / query / where / history / attempts 中的 `output_file` 不再被普通 MCP 查询误判为交付声明；真实 export `options.outputPath` 仍被验证。
- verify / cancel 失败不会在 OutputDiscarded 前清 held checkpoint。
- final CAS 只有所有 deferred sends 成功后才清 held text；partial raw publish 仍是 receipt-free provisional，并由 Prepared journal recovery 回滚。

### 6. terminal truth 消费面

- Manager A2：一次 `TurnCompleted` metrics + 一次 `Error`，零 `Finish`、零 distill、零 artifact commit。
- CLI one-shot / REPL / JSON：非 EndTurn、A2、普通 engine Err 都是 Error-only，不再附 success `StreamEnd`；rollback failure 是独立 fatal state-inconsistent，并在统一 cleanup 后退出。
- Provider health：只接受非空、无 adjudication 的 clean EndTurn；MaxTokens / MaxTurns / Refusal / ToolUse / empty EndTurn 均 unhealthy。
- Tool efficiency telemetry：MaxTokens / MaxTurns / Refusal 与 adjudication 记录为 error，保留真实 issue kind。
- Robot fallback 只有 provider / unknown-upstream 且 `retryable == Some(true)` 才自动重放；A2 是 `retryable=false`。
- Cron replay receipt：只有 `completed=true && result_ok=Some(true)` 是 Success；false 是 Error，None 或 incomplete 是 Quarantined。
- Conversation model failover 不把新的 unbacked completion code加入 provider auto-failover。
- one-shot、provider config drains、autocompact、bootstrap extract / vision、image intent 均只让 clean EndTurn 成功，拒绝 partial / no-Done / post-Done / MaxTokens / Refusal。

### 7. distillation

- memory distill 已拆为 prepare（不写盘）和 final gate 内 apply。
- 只有 artifact verify、session seal 和 terminal lifecycle gate 均成功后才写 memory。
- A2、MaxTokens、cancel、artifact reverify failure 都不会把失败 turn 写入自动记忆。

## 三、唯一确认仍开放的 P1 blocker

accepted-turn rollback 已恢复 transcript、host context 和 deferred tool activations，但**尚未恢复本轮在内存中修改的隐藏 runtime authority**。

可复现安全形状：

1. 失败 turn 先成功执行 inline Skill；
2. Skill 通过 context modifier 授予 Bash、切 model / effort / plan，或向 HookEngine 合并 hook；
3. 模型随后给出无证据完成声明，A2 拒绝并撤回 transcript；
4. 同一个 runtime 下一 turn 仍保留无可见 provenance 的自动批准权限 / hook / plan 状态，而 fresh reload 不保留，形成 live / reload 分叉。

必须纳入一次性 accepted-turn in-memory snapshot / restore 的字段：

- `AgentEngine.model`；
- `thinking`（审计本 turn 动态 config 路径后决定是否保留）；
- `current_reasoning_effort`；
- `compaction_level` 与 `CompactState`；
- engine `allow_list`；
- `ToolConfirmer` 完整状态：`auto_approve` + allow set；
- `HookEngine` 的 `HooksConfig`，但必须保留原 shell / process supervisor；
- `PlanState` + `plan_active_flag`；
- `GoalRuntime` 共享 `GoalState`（status / auto_continuations 等）；
- 已有 ToolRegistry deferred activation root 继续复用，不另造猜测。

明确不应回卷：

- `total_usage`：供应商成本和 telemetry 必须保持真实；
- 已经发生的 workspace / 外部工具副作用：本轮只恢复 authority 与会话真相，不伪装成可逆事务。

还需明确审计、不要擅自决定：

- `ToolApprovalManager` 中用户显式选择 “Always” 的授权，是跨失败 turn 保留的用户决定，还是 accepted-turn authority。需要依据现有产品契约单独裁决，不能与 Skill 自动 grant 混为一谈。
- `cache_detector` / `stagnation_guard` 目前可视作诊断 / 每轮状态；若代码证据显示会影响下一轮执行，应纳入 snapshot。

最小安全实现建议：

1. 在 `CompletionEvidenceContext` 第一次 capture accepted root 时同时捕获一次 `AcceptedTurnRuntimeAuthority`；host race-tail 不得覆盖。
2. 给 `ToolConfirmer`、`HookEngine`、`GoalRuntime` 增加窄的 snapshot / replace API；不要 clone HookEngine shell 或进程 authority。
3. 所有 exact root rollback 同时恢复 runtime snapshot：A2、host delivery / artifact / session failure、cancel、provider Err。
4. clean success commit 只丢弃 snapshot，并保留本轮合法 modifier / hook / goal 更新。
5. restore 失败必须走现有 AgentSessionInconsistent / runtime retirement，不能继续复用半恢复 runtime。

必测矩阵：

- Skill grant Bash + hook + model + effort + plan 后 A2：live runtime 精确回 root，fresh reload 等价。
- Goal status / auto-continuations 在拒绝后恢复；success 后保留。
- provider Err、cancel、artifact / session commit failure 同样恢复 authority。
- host race-tail 只捕获一次 root；pass A 修改 authority、pass B 失败时恢复到 pass A 之前。
- clean success 保留所有合法变更。
- 用户显式 approval 的语义测试按审计裁决补齐。

除这项外，最后一轮独立审计没有确认其他仍可复现的 P0 / P1。

## 四、最近验证证据

环境：

```powershell
$env:NO_PROXY='localhost,127.0.0.1,::1'
$env:CARGO_INCREMENTAL='0'
```

最后已证明的 core 节点：

- `cargo check -p nomi-agent -p nomifun-ai-agent -p nomifun-conversation`：通过。
- `nomi-agent` completion evidence：32 / 32。
- `nomi-agent` session：18 / 18。
- `nomi-agent` tool efficiency：5 / 5。
- `nomifun-ai-agent` Nomi session persistence：10 / 10。
- A2 two-pass race-tail full-discard、boot orphan heal / retry / quarantine：定向通过。
- CLI terminal classifier：8 / 8。
- Provider health / termination guard：定向通过。
- Cron receipt truth table：1 / 1（覆盖 6 种形状）。
- Channel accepted / rolling checkpoint：新 2 项 + 既有 2 项通过。
- Backend no-Start discard、accepted / rolling checkpoint、artifact baseline / TOCTOU / deferred lifecycle：各定向通过。
- 当前 `git diff --check`：通过。

注意：

- Channel / Backend 最后分片合入后没有再跑统一 workspace check。
- 之前启动的 `cargo test -p nomi-types -p nomi-tools -p nomi-skills -p nomi-cli` 的 exec session 已丢失，无法证明最终结果；不要把它写成通过，应重跑。
- Windows 上 `cargo fmt --all -- --check` 会因路径长度报 `os error 206`；使用 touched packages 的 `cargo fmt -p ... -- --check` 替代，并报告该平台限制。

## 五、下一 Agent 的执行顺序

1. 先读本文件和上一阶段 B1 / D1 / E1 交接；检查 `git status`，保留全部 dirty changes。
2. 完成 runtime-authority snapshot / restore 和上述定向测试。
3. 让独立审计只复核这个新 seam；不要重新无限扩展 NLP classifier，除非出现确定 P0 / P1 反例。
4. 串行跑最小门禁，避免共享 Cargo incremental 竞态：

```powershell
$env:NO_PROXY='localhost,127.0.0.1,::1'
$env:CARGO_INCREMENTAL='0'

cargo check -p nomi-agent -p nomi-cli -p nomifun-ai-agent -p nomifun-conversation -p nomifun-channel -p nomifun-cron
cargo test -p nomi-agent --lib
cargo test -p nomifun-ai-agent --lib
cargo test -p nomifun-conversation --lib
cargo test -p nomifun-channel
cargo test -p nomifun-cron
cargo test -p nomifun-idmm --lib
cargo test -p nomifun-app
cargo test -p nomifun-api-types
bun test
bun run check
```

5. 发布前运行根 `bun run test`（它会跑完整 Cargo suite）；若出现已知 loopback proxy 502，确认 `NO_PROXY` 生效后只定向复跑失败项，不能把环境故障伪装成源码失败。
6. 二次运行 ts-rs export tests，确认 protocol bindings 没有 diff 漂移。
7. 运行 touched-package rustfmt、`git diff --check`、`git status`，检查无临时 debug / ignored test / 冲突标记。
8. 更新本文件为 final complete（或新建 final-complete 文档），明确测试计数和任何平台未运行项。
9. 审查 staged 文件后提交 `fix/truncation-finalize`。
10. 按用户已经明确给出的授权执行发布：

```powershell
git fetch origin
git switch main
git merge --ff-only origin/main
git merge fix/truncation-finalize
# 解决冲突后重跑 overlap / release gates
git push origin main
```

禁止 force-push、reset dirty worktree 或改写共享历史。

## 六、可直接复制的新会话启动 Prompt

```text
ultracode

继续并最终发布 Nomifun Coding Agent 执行链路重构。仓库：
C:\Users\rika0\code\nomifun\bak\1\nomifun-tauri

请先完整阅读：
1) docs/handoffs/2026-08-21-truncation-refactor-a2-finalization-handoff.md
2) docs/handoffs/2026-08-21-truncation-refactor-b1-d1-e1-complete.md
3) 根目录 AGENTS.md

当前分支 fix/truncation-finalize，HEAD 是本交接文档提交，其父提交/代码基线是 1f00647f；本地和 origin/main 当前为 98565164。工作树有约 51 个 tracked 未提交文件，全部是本轮合法工作；禁止 reset、checkout 覆盖或丢弃它们。当前没有半写的 runtime-authority snapshot，也没有 Cargo/Rust 进程。

目标不是重新设计，而是完成唯一确认仍开放的 P1：accepted-turn 回滚必须同时恢复隐藏 runtime authority。第一次 accepted root capture 时建立跨 race-tail 只捕获一次的 in-memory snapshot，至少覆盖 model、thinking（先审计动态路径）、current_reasoning_effort、compaction_level/CompactState、engine allow_list、ToolConfirmer完整权限、HookEngine HooksConfig（保留shell/supervisor）、PlanState/plan flag、GoalRuntime共享状态；deferred ToolRegistry activation继续用现有root。total_usage和真实外部副作用不能回滚。单独裁决 ToolApprovalManager 用户Always授权，不要把用户显式决定和Skill自动grant混为一谈。

所有 exact root rollback（A2、provider Err、cancel、artifact/delivery/session failure）都要恢复 authority；clean success丢弃snapshot并保留合法变更。补 live/fresh reload一致性、Skill grant Bash+hook+plan/model/effort后A2、goal、race-tail、cancel/Err、success保留回归。任何恢复失败都必须走现有 AgentSessionInconsistent quarantine。

完成后先做一次独立、只读 P0/P1 审计，只复核新的 authority seam 和现有 terminal transaction；不要无限扩展启发式 classifier。随后按交接文档串行跑 targeted + broad Rust/UI/release gates。NO_PROXY=localhost,127.0.0.1,::1，CARGO_INCREMENTAL=0；不要并行 Cargo。

所有门禁通过后：更新最终交接文档，审查staged文件，提交当前分支；fetch origin，更新本地main并合并fix/truncation-finalize，解决冲突后重跑重叠门禁，最后push origin main。用户已经明确授权合并和推送main；禁止force-push或改写共享历史。不要留下未完成项目；若出现新的确定P0/P1，修复并回归后再发布。
```
