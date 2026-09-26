# Agent 可靠性开发交接

更新日期：2026-09-25。目标仍未完成，未证明工具成功率、执行稳定性或交付质量达到 99%。

接手后的修复与实测范围见 [V2 接手验证记录](VALIDATION-2026-09-25.zh.md)。下文“未验收”描述为原始交付快照；原 `handoff-manifest.json` 保留，不覆盖接收时的完整性证据。

## 最新交付：V2 补充实现，未进行应用编译或测试验收

用户最新要求继续实现暂停续期、核对后续跑和证据流水线，但不马上测试。本轮只做源码、必要契约生成及交接记录。
先读 `PAUSE-RESUME-V2.zh.md` 与 `EVIDENCE-PIPELINE.zh.md`；下文 V1 检查结果不适用于当前 V2 源码。

V2 新增：

- 非终态暂停：底层 Turn 保持 running，`native_pause_json` 表示暂停，head 为 paused，保留 active Turn；不是 failed，也不创建新任务。
- 所有者 API：execution/pause、execution/resume、execution/effects、execution/reconcile；绑定版本、digest、幂等键及显式预算增量。
- 复用真实 owner 回执；丢失 Runtime result 时补核对观察，不执行旧工具。认证 owner 可以针对 exact effect/invocation 提交可审计的核对证据。
- pending/unknown 人工核对保留 uncertainty → reconciled，不伪装原始成功；取消/失败任务可结清效果，但真正 terminal 不重开。
- applied steering 尾部纳入 checkpoint，未 applied 的排队；恢复后重新确认计划与工作区证据。
- SDK/Kernel/工具按新 generation 重开同一 Turn，旧 scope 必须关闭。模型/准备失败可保留暂停；清理未证明时保持 quarantine。
- 默认额度不变；owner 才能增加有硬上限的执行/日志/payload 额度，reader 同步支持。普通历史候选窗口仍保持小窗口。
- paused 协议、turn.paused 消息、只读模型进度；channel/robot 不把暂停当成功。
- 固定独立 grader 的采集、签名和聚合脚本，以及产品 fixture capture 辅助函数。没有运行评测或新增实测成功样本。

入口：contracts/native_execution.rs；session/native_pause.rs、native_effect_reconciliation.rs；runtime/reconciliation.rs；app/router/native_execution_control.rs；agent-reliability-collect.mjs；app/tests/common/native_reliability_capture.rs。
crate 路径均在 crates/backend 下，脚本在 scripts/validation 下。

SQLx migration 到 **007**，Agent Store head 为 **6**。007 摘要来自生成的 agent-store-migration-manifest.envelope.json：
`de5d31539af0cc75f26d2090546a908b39e8719665d3f8dac6eda6e0b26fa036`。
不要改旧 005/006，也不要拿聚合 schema envelope digest 替换迁移摘要。

本轮只运行 `agent-v2-contract -- write`（为生成产物构建 contracts 包）及源码交接清单生成。
没有运行 cargo check/test、验收 --check 模式、UI 测试、collector/grader 或 live runner。旧 failed 预期和新 enum 分支需下一台机器核对。
这是 API-first 交付，未增加独立图形管理面板或新布局。人工核对不等于自动验证所有外部系统；清理不确定的宿主不能清零错误标志假装修复，必要时由 owner 核对并重启后恢复。

## 当前开发方式

用户于 2026-09-25 明确要求：优先整体实现，完成开发后分阶段测试；其他电脑上的 agent 接手系统测试和完善，以降低重复开发/测试成本。
本机开发期间以必要编译、契约生成和静态检查为主，不开展新一轮真实模型评测或反复运行大型测试。
编译通过、离线测试通过和真实产品验收必须分开记录。不得把“待运行”改成“通过”。

## 接手先读

1. 仓库根 `AGENTS.md`：仅桌面 renderer，最小 880×600；本轮主要改后端，不引入移动布局。
2. 同目录 `README.zh.md`：原始问题、技术方案调研、历史失败样本及统计验收口径。
3. 本文件：当前实现入口、未完事项、验证次序。历史文档的“未完成”是当时快照，以这里的最新状态为准。
4. `git status --short` 和相关 diff。本线程包含大量未提交、未跟踪文件；这些都属于开发现场，不得重置。

工作目录：`C:\Users\MINISFORUM\code\nomifun\multi\1`，Windows PowerShell。`rg` 不可用时使用 `git grep` / `git ls-files` / `Select-String`。
不提交、不强推、不改写无关内容。不复制 API key、用户数据库、`.tmp-*` 运行日志或环境变量到交接包。

## 已有实现及代码入口

| 层次 | 入口 | 关键职责 |
| --- | --- | --- |
| 请求重试 | `nomifun-chat-model-broker/src/retry.rs`, `broker.rs` | 原路由有界退避、Retry-After、取消、语义输出后禁止盲重放 |
| 工具协议 | `nomifun-agent-runtime/src/tool_validation.rs`, `protocol_recovery.rs`; `nomifun-engine-core/src/tool_schema.rs` | 整批参数预检、schema 反馈、拒绝执行文字伪调用 |
| 计划/交付 | runtime 的 `planning.rs`, `requirements.rs`, `completion.rs`, `workflow.rs` | 幂等计划、完整输入覆盖、当前证据约束 |
| 进度检查点 | runtime `checkpoint.rs`; session `native_checkpoint.rs` | 最新状态在 canonical `agent_turns`，与元数据事件原子提交 |
| 执行所有权 | session `native_execution.rs` | 90 秒租约、接管 CAS、fence/generation，旧执行者不可准入 |
| 运行时恢复 | runtime `recovery.rs`, `history.rs`, `turn.rs` | 从精确边界重建；抛弃未执行模型尾部；不重放工具 |
| 生产持久写入 | app `router/engine_journal.rs`, `engine_session_host.rs` | 受 fence 保护的模型、工具、检查点与终态；30 秒心跳 |
| 重启调度 | app `router/engine_recovery.rs`, `native_turn_recovery.rs` | 启动前捕获孤儿候选，租约过期后接管原输入 |
| 统计门禁 | `scripts/validation/agent-reliability-report.mjs` | 样本清单/缺失失败/独立验收/置信界检查，不生成实测证据 |

上述 crate 路径均在 `crates/backend/` 下。

## 不可破坏的约束

- 恢复不是重新发送用户任务，不能生成另一个 Turn 或重复已完成副作用。
- checkpoint 只是数据。必须校验 owner、Session、Turn、Snapshot、build、能力代次、digest、cursor，再获得执行租约。
- 取消、权限变化和不确定 effect 优先于自动继续。未知结果不能当失败后直接重试。
- 原始输入和已接受的纠正顺序保持不变；恢复后的历史验证不能冒充当前验证。
- 同一逻辑 Turn 内模型/压缩/工具身份不复用。分段只能重置有明确边界的窗口预算，不能清除累计预算或错误计数。
- 原始事件是唯一历史事实源；不引入第二个 transcript 数据库，不通过删除审计记录来腾预算。
- 不放宽真实质量验收，不擅自执行用户禁止的测试，不以模型自评证明 99%。

## 当前实现计划

1. 加入持久分段状态、检查点成功后的预算续期、总量与无进展限制。
2. 协调 Journal 窗口、流量预算、压缩操作编号以及 Session payload 的硬上限。
3. 补齐重复崩溃恢复和安全尾部校验；不确定效果走明确的核对/阻塞路径。
4. 更新生产连接和契约，编译集成；分阶段测试清单交给下一台机器执行。

这些实现现已落到源码。开发期编译/契约状态见交付末尾；新增行为的测试执行按用户要求分阶段交接。

## V1 主干实现记录（历史快照，部分边界已由 V2 覆盖）

### 同一任务内的持久分段

- runtime `segments.rs` 定义 host 选择的策略、持久分段状态、停止原因和进展摘要哈希。
- standalone `AgentTurnRequest::new` 仍是原有硬 step cap；只有生产 host 显式调用 `with_execution_segments` 且 sink 支持 checkpoint 才启用分段。
- 默认窗口仍为普通 32 / coding 64 个模型步骤；最多 16 段，累计最多 512 / 1024 个步骤。不是自动执行这么多次的目标。
- 原始输入、计划、工具历史、总计数及错误纠正额度跨段保留；有新进展才持续续期，最多容许一个无进展窗口，第二个连续无进展窗口停止。
- 新颖观测仅用于有界调度，不是质量证明。最多保存 1024 个摘要哈希，不淘汰旧项来伪装进展。
- `ExecutionSegmentRenewed` 只能出现在 checkpoint 确认之后。总量、无进展、非静止状态、持久化不可用或存储压力用 `ExecutionBudgetExhausted` 区分。
- 预算耗尽当前映射为 **TaskIncomplete / failed** 并保留最后检查点，不宣称成功。没有自动增加总预算或重新打开 terminal Turn 的接口。

### 预算/历史连续性

- Journal 每窗口 3200 条 / 4 MiB，提前在 2400 条或 2 MiB 请求轮换；轮换不删除事件。
- 12 MiB 累计 native 数据开始停止新模型工作，普通写入硬限 16 MiB、清理总预留至 20 MiB；总记录硬限 64000。
- 恢复/单 Turn 历史读取 24 MiB，历史候选窗口 32 MiB；常量在 contracts `session.rs`，避免 writer 与 reader 各设不相容限制。
- Session payload 16 MiB 硬限未放宽，剩余 4 MiB 时停止自动推进以留收尾空间。极端超大单批次仍会受硬限拒绝，必须做压力矩阵。
- 压缩调用编号累计递增，压缩窗口可以续期，但累计上限 2048。重启不复用 `:compact:N`。
- 规则读取 `agent-instructions:N` 重启后沿用新编号，不复用旧回执；原来的总读取上限 4096 仍保留。
- 模型准入热路径只读相关 Turn；资源请求验证 exact canonical 模型 claim，不从任意历史 JSON 中找到 operation_id 就视为授权。
- 恢复读取当前 Turn 及原输入，不加载所有旧任务。一般完整 Session 历史的进一步分页优化不在本轮宣称完成范围。

### 恢复/隔离/收尾

- Runtime 起始即保存初始 checkpoint；仅有 `TurnStarted` / `TurnInputScope` 而没有 checkpoint 的崩溃可由 host 原子补零进度边界。
- 同一个 checkpoint 可重复接管；丢弃的模型尾部不会被闭合历史重放重新加入。调用 ID、fence、模型步骤和压缩号继续递增。
- 恢复后重新检查 workspace/规则，旧验证证据和进程句柄不复活。
- 30 秒心跳维护 90 秒 lease。journal 与 SDK 的取消双向连接，旧 producer 被 fencing 后可停止其 SDK 工作。
- preclaim/attached/abandoned 使用原子状态转换；等待 driver attachment 有 30 秒截止；未使用 claim 加载失败可释放。
- 恢复失败重试后通过 session `native_recovery.rs` 原子隔离：pending effect → unknown、提升 fence、记录 `runtime/execution-recovery-blocked`、写 failed terminal，保留 checkpoint。
- Cron 不抢先终结由 native 恢复调度负责的 Turn。短暂数据库读取失败不会直接遗弃恢复 job。
- 旧的半条 assistant 文本按原内容关闭消息段，新模型使用新 step message ID；这不是任务成功事件。
- private Runtime terminal 缺失时，历史只可根据 canonical failed/cancelled 事实派生中断，不能合成成功。

### 诊断入口

`GET /api/agent-sessions/{agent_session_id}/execution`，沿用原 API 的认证 owner。
返回最新 Turn 的状态、lease/fence/generation、checkpoint 是否保留、pending/unknown effect 数量和 payload 用量；不返回 key、原始工具输出、checkpoint 正文或 holder ID。
`automatic_replay_authorized` 永远是 false：诊断响应本身不能成为执行许可。

## V1 当时的缺口与限制（前两项已有 V2 补充流程）

1. checkpoint 后出现工具准入、已应用纠正或其他非安全尾部时，不做通用自动重放；当前进入 owner 核对/隔离。没有为所有领域 owner 实现业务级自动对账。
2. 总预算/存储上限不是无限续期；非静止进程也不能通过换段伪装成已持久恢复。当前未增加可从 UI 续费预算/解除隔离的控制面。
3. 跨 build/Snapshot/能力代次恢复仍要求精确兼容，不做任意迁移或扩权。
4. 新增应用边界、桌面消息恢复、多小时真实任务和跨平台矩阵尚未完成行为验收。
5. 真实模型权限、协作/长 coding 效果、独立交付质量与统计规模证据仍待完成。因此整体 99% 目标未完成。

这些限制不得在测试交付时改写成“自动恢复已覆盖所有失败”。详见 `TEST-MATRIX.zh.md`。

## 已有验证证据（冻结记录）

- 2026-09-25：`cargo test -p nomifun-app --test native_execution_recovery -- --nocapture`，1 passed，0 failed。
  实际应用/官方 coding Agent/真实文件工具/本地脚本 provider；运行中数据库快照 → 新应用启动 → 自动续跑；写入准入一次，恢复一次，完成并独立校验文件。
  测试最初未注册，已在 `nomifun-app/Cargo.toml` 注册后真实运行。此结果只覆盖当时版本，不覆盖后续未跑的改动。
- runtime 中 `recovered_turn_does_not_repeat_a_completed_write_or_execute_the_abandoned_model_batch` 已通过，覆盖完成写入与未执行尾部的区别。
- Store 的租约/竞争接管测试曾通过；最近全部改动尚未重新跑完整定向套件。
- 更早的 broker、工具预检、计划、检查点、schema、统计门禁结果见 `README.zh.md`。不能累加这些测试数量作为产品成功率样本。
- 真实 StepFun coding 冒烟曾有一轮通过；协作及长 coding 未通过。最近正式应用首次模型请求 HTTP 403，未运行工具；已停止凭据重试。

## 后续分阶段测试入口

开发集成完成后，在干净的测试副本中按以下顺序运行，不要直接跑所有平台的大套件：

1. 编译/契约：`cargo check -p nomifun-app`；`cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`；`git diff --check`。
2. 状态机：`cargo test -p nomifun-agent-session --lib`；`cargo test -p nomifun-agent-runtime --lib`。
3. 数据迁移：`cargo test -p nomifun-db --test id_schema_contract`；`cargo test -p nomifun-db --lib migration`。
4. 应用路径：`cargo test -p nomifun-app --test native_execution_recovery`；`cargo test -p nomifun-app --lib history_display_tests`。
5. 故障矩阵：连续恢复、租约竞争、取消与迟到 producer、待处理 steering、初次 claim 崩溃、窗口/总预算、未知 effect。逐项记录未测/通过/失败与运行版本。
6. 获得可用凭据后，通过既有凭据隔离 runner 做小规模 coding / collaboration / long-coding，不要把 key 放进命令行、文件或报告。
7. 冻结任务分布与独立产物验收，再进行统计规模评测。未经这些步骤不得标记整体完成。

## 跨电脑转移注意事项

`git diff` 不包含未跟踪文件。本轮关键模块和迁移有不少未跟踪文件，必须同时转移 `git ls-files --others --exclude-standard` 中属于本次开发的源码/文档。
不建议转移 build.noindex、node_modules、target、用户数据或任何凭据；在新机器重建。已有同名文件先比较，不覆盖其他人的工作。
当前 SQLx 文件到 `007_native_pause_resume.sql`；Agent Store migration head 为 6，两个编号体系不同。
生成契约使用现有 `agent-v2-contract -- write`，不得手改 digest ledger 来通过检查。

源码清单生成/核对：

```text
node scripts/validation/agent-reliability-handoff.mjs --write
node scripts/validation/agent-reliability-handoff.mjs --check
```

`handoff-manifest.json` 只保存路径、字节数、SHA-256 和基线 commit，不打包源文件正文或数据，不是测试结果。
白名单覆盖本次后端模块、迁移、生成契约、文档与验证脚本，包含未跟踪源文件。转移清单中的 present 文件及 manifest 本身；deleted 条目只用于比较，脚本不会替你删除文件。
接手提示可直接使用 `NEXT-AGENT-PROMPT.zh.md`。源码再次变化后应由交付者重新生成清单，而不是由接收者跳过差异。

清单接收检查同时要求 Git HEAD 与交付基线一致；其他基线应先显式比较/集成，不使用 reset 丢弃目标机器的工作。
迁移 006 的 `schema_metadata.canonical_schema_manifest_digest` 来自 `digest_payload(agent_store_schema_manifest_payload())`，当前是
`6c3ea4f8d5d3e12cbc36ebb48d7d86ef79d1794630c1acbab0e646799de92d96`。
它不是 `contracts/generated/canonical-agent-store-schema-manifest.envelope.json` 的聚合 envelope digest；两者不同是正常的，不要相互替换。

## 上一轮 V1 检查记录（2026-09-25；不是当前 V2 的检查结果）

| 实际执行的检查 | 结果与范围 |
| --- | --- |
| `cargo check -p nomifun-agent-runtime -p nomifun-agent-session --tests` | 通过；类型检查库与测试目标，没有运行测试 |
| `cargo check -p nomifun-app --lib --test native_execution_recovery` | 通过；包含当前生产接线及恢复 fixture 的类型检查，没有运行 fixture |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write` | 已生成最新 schema/API/event/ledger 等契约产物 |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check` | 通过 |
| `node --check scripts/validation/agent-reliability-handoff.mjs` | 通过语法检查；传输校验命令另行输出清单结果 |
| `git diff --check` | 通过 |

app 仍有既有的 11 个 dead_code 警告（主要是非默认 Computer Role 和 hosted-effect helper）；本轮未做无关清理。
仓库 `rustfmt.toml` 设置 `disable_all_formatting=true`，本轮未改变该策略，也未宣称全仓库格式验收。
当前新代码的单元/产品/故障矩阵/真实模型测试均按用户要求留给分阶段验收；唯一应用恢复执行通过记录是本轮开发改动之前的冻结结果。
没有提交、强推或导出凭据、生产用户数据；哈希清单本身不含源码正文。
