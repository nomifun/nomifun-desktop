# 分阶段验证与交付矩阵

更新：2026-09-25。用户选择开发优先，新增行为的测试执行留给后续机器/agent。
下表是验收要求，不是通过报告。历史结果仅见 `DEVELOPMENT-HANDOFF.zh.md` 的冻结记录。

## V2 新增验收范围（当前全部未执行）

先读 PAUSE-RESUME-V2.zh.md 和 EVIDENCE-PIPELINE.zh.md。本轮没有应用编译、测试或 grader 执行，V1 通过结果不可沿用。

- 007 forward migration、head=6、未决 native head 指针修复；旧 terminal 不复活。
- 非终态 pause、阻止普通发送、cancel 优先、显式 resume、并发/重复授权幂等、旧版本/digest/owner 拒绝。
- 同一 Turn 新 generation 重开 Kernel/工具/进程/Browser；旧 writer/relay 不影响新代次。
- 原回执补观察、effect/invocation 人工核对、pending → uncertainty → reconciled；核对 audit 和 input digest。
- 部分批次、未派发/不完整提案、applied steering、deferred 重入、OwnerOutcome notice 与旧错误共存。
- 清理未知、模型中断、准备失败、私有 terminal commit 丢失后的暂停；真正 canonical 终态不可重开。
- 默认/新增额度、工具调度窗口、大额度 reader、普通任务不多加载历史。
- paused 在 channel/robot/桌面消息与队列中不是成功；当前为 API-first 控制。
- collector 的 pin、grader 哈希、HMAC、mock 拒绝、缺失/失败样本、工作区一致、超时清理和统计集成。

旧 failed 断言要按非终态语义核对，不能机械放宽。新 enum、SDK mock、capture helper 需先做 P0 类型检查。

## P0：接收源码、编译和契约（不调用模型）

```text
node scripts/validation/agent-reliability-handoff.mjs --check
cargo check -p nomifun-agent-runtime -p nomifun-agent-session --tests
cargo check -p nomifun-app --lib --test native_execution_recovery
cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check
git diff --check
```

哈希不一致时先比较改动，不覆盖目标机器的其他工作。`--check` 验证源码转移完整性，不验证代码行为。
接收时 Git HEAD 也应匹配 manifest 的基线；不要用破坏性 reset 来“修复”另一台机器已有的改动。
若只是换行被转换，也需要确认来源；不要通过随意更新 manifest 来掩盖漏文件。
契约不一致时核对来源，再用已有生成器 `agent-v2-contract -- write`，不要手改 digest ledger。

## P1：状态机/迁移定向测试（不调用真实模型）

```text
cargo test -p nomifun-agent-runtime --lib
cargo test -p nomifun-agent-session --lib
cargo test -p nomifun-db --test id_schema_contract
cargo test -p nomifun-db --lib migration
cargo test -p nomifun-chat-model-broker --lib --test conformance
cargo test -p nomifun-engine-core --lib
node --test scripts/validation/agent-reliability-report.test.mjs
node --test scripts/validation/probe-stepfun-tool-schema.test.mjs
```

重点新增入口：

- runtime `segments::tests`：累计上限、无进展、重复观测不能购买续期。
- runtime `execution_segments_cross_model_windows_only_after_checkpoint_acknowledgement`：2 步窗口跨 3 段，调用号递增，写入一次。
- runtime `recovery::tests`：同一检查点连续恢复，丢弃两次未执行文本，副作用尾部不能隐藏。
- runtime `recovered_turn_does_not_repeat_a_completed_write_or_execute_the_abandoned_model_batch`：已写入与未执行提案分离。
- session `native_execution_tests`：竞争租约、旧执行者隔离、取消、未使用 claim 释放、零进度 preamble、恢复隔离和 owner 检查。
- checkpoint 测试：CAS/digest、重开数据库、失败保留、完成/取消清理；新增初始检查点后不要继续假定固定旧 revision=2。

迁移检查必须覆盖：现有数据经 005/006/007 保留、新数据库初始化、重复打开、schema manifest 一致。
SQLx migration 文件编号到 007，Agent Store migration head 是 6；两个编号体系不同。

## P2：正式应用路径（脚本 provider）

```text
cargo test -p nomifun-app --test native_execution_recovery
cargo test -p nomifun-app --lib history_display_tests
cargo test -p nomifun-app --test nomi_core_route_gap
cargo test -p nomifun-app --lib chat_broker_host::tests
```

`native_execution_recovery` 使用官方 coding Agent、API 建立的 Session、真实文件工具与本地 HTTP 脚本 provider。
它不使用真实 key。测试流程：完成写入 → 第二轮模型等待时事务快照 → 只清理原数据库 → 从快照重启 → 自动恢复 → 独立读文件断言。
应用启动会等待现有租约到期；测试只在隔离快照中将期限置零，不要对用户数据库这样做。
检查 `GET /api/agent-sessions/{agent_session_id}/execution`：运行时保留检查点，完成后不再保留，fence 正确增加，无自动 replay 授权。

## P3：故障与边界（必须逐项补齐/运行）

| 场景 | 必须独立确认的结果 | 当前交付定位 |
| --- | --- | --- |
| 两个恢复者同时接管 | 一个 winner；另一个不能取消、写入或终结 winner | Store 用例已写；应用竞争待跑 |
| 旧 producer 迟到 | 模型准入/工具准入/结果/检查点/终态都被 fence；旧取消不影响新任务 | Store 用例已写；应用待跑 |
| 接管后加载失败 | 未使用 claim 可释放；已有准入不能谎称未使用 | 用例已写 |
| claim 后、首个 Runtime 事件前崩溃 | 重用原输入，零副作用恢复，不新增 Turn | Store/应用组合待跑 |
| TurnStarted/TurnInputScope 后首个检查点前崩溃 | 只允许这两个事件，原子建立零进度 checkpoint | Store 用例已写；应用待跑 |
| 同一个 checkpoint 连续崩溃 | fence、模型号和压缩号不复用，旧文本不复活 | runtime 用例已写；应用待跑 |
| 写入完成后模型流中断 | 已完成写入一次；未准入批次零次 | 旧版本应用用例曾通过，当前版本重跑 |
| checkpoint 后出现工具准入/效果 | 不盲重放；核对不能完成则显式隔离并保留进度 | 保守分支已实现，故障点待扩展 |
| pending managed/external effect | 变为 unknown，不变成可重试 failed；继续阻挡新效果 | Store 用例已写；owner 端核对待补矩阵 |
| 用户取消与恢复并发 | 用户取消优先；不能复活取消的 Turn | Store/应用待跑 |
| 纠正输入在模型等待时入队 | 保持接受顺序、完整文本、附件与技能选择 | pending 恢复实现；应用待跑 |
| 纠正已 applied 但 checkpoint 尚未提交 | 不丢输入，不用旧 checkpoint 越过它；当前要求核对 | 保守边界，不是自动重放支持 |
| Snapshot/能力代次/build 改变 | 不自动兼容、不扩权限 | 待跑 |
| segment 边界上进程还运行 | 不持久化活句柄为可恢复证明；停止时 SDK 仍清理资源 | 待跑 |
| Journal soft/hard/累计预算 | 窗口只在 checkpoint 确认后重置；总量不清零；清理有预留 | 待跑 |
| Session payload 接近上限 | 提前停止并记录原因；不删除历史腾空间 | 待跑 |
| 无进展/重复读/反复计划 | 不无限续期；不能把哈希“新颖”当质量验收 | pure 用例已写；产品循环待跑 |
| 长 Turn 结束后再发新任务 | 历史可读取；不能被旧 4096 条/8 MiB 假上限永久卡住 | 边界统一；压力测试待跑 |
| 恢复前 UI 有半条文本 | 只关闭消息段；新输出新 ID；不合并/重复，不宣称旧任务完成 | 后端重建已实现；桌面待跑 |
| native private terminal 缺失 | 只能从 canonical failed/cancelled 派生中断；不可合成成功 | 投影逻辑已实现，待跑 |
| 本地 Windows / Linux / macOS | 文件与进程语义、数据库锁、快照和清理一致 | 跨平台待跑 |

任何检查失败都要记录原始任务、构建哈希、故障注入点、canonical event cursor、实际文件/进程结果。
不要以增加 sleep、放宽断言、删除失败记录或降低完成条件“修好测试”。

## P4：小规模真实模型回归

先恢复授权，再用既有凭据隔离 runner。此前正式应用首次请求 HTTP 403，无法据此判断 schema 修复的有效性。
只在权限确认后跑 coding、collaboration、long-coding；先小样本确认成本和错误分类，不直接跑统计批量。
现有 `nomi_core_live_provider_smoke` 中 opt-in/ignored 用例未执行不能算通过。

## P5：独立质量与统计交付

冻结任务分布、预算、模型/路由、构建、样本清单和独立产物断言，再采集全量结果。
工具成功、执行稳定、交付质量分别报告。取消、unknown、缺失样本、预算结束与恢复后成功单独记录。
`agent-reliability-report.mjs` 的门禁测试均为合成数据，不能充当实测样本。
未经独立产品样本和置信区间验收，保留“未证明 99%”，不要宣布整体目标完成。
