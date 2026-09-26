# V2 接手验证记录

日期：2026-09-25；环境：Windows / PowerShell；本轮完成接手修复、定向离线验收和小规模真实回归，P3 完整矩阵与 P5 仍未完成。

**最新交付范围**：用户随后明确以交付为主，只要求构造数据和场景测试，不进行真实跨天等待或新的真实体验流程。已停止容器环境下载准备；之后仅补充离线构造用例和定向回归。下文真实模型结果均是这一收敛指令之前取得的历史记录，不计作离线构造测试的统计样本。

本记录仅描述接手后的实际检查。原 DEVELOPMENT-HANDOFF / PAUSE-RESUME-V2 / EVIDENCE-PIPELINE 中的 V1 冻结结果不能代替这里的 V2 证据。整体 99% 仍未证明。

## 接收与来源

- Git HEAD 为 `1348e4def003df8d15890e31554a8f1eb34c56d3`，分支 `rf/agent-capability-platform-v2`。
- 修改前执行 `node scripts/validation/agent-reliability-handoff.mjs --check`：118 个文件全部匹配。
- 保留已有未提交、未跟踪文件；未 reset、提交、推送或重写历史。
- 原 `handoff-manifest.json` 保持不变。本轮修复后的源码自然与原交付哈希不同，不重新生成清单来掩盖差异。
- 定向检查原始输出保存在本机忽略目录 `.tmp-agent-reliability-20260925/`；该目录不属于源码交付或统计样本。

## 已定位修复与补充验证

1. 应用暂停 relay 使用不存在的 generation 变量，导致编译失败；随后将终态通知绑定为对同一 Turn 的 generation 和状态进行一次原子快照读取，覆盖暂停和真正终态。
2. Session Store 的精确 `agent_turns` 列顺序检查与 canonical schema / 005→006→007 迁移顺序不一致，导致 Store 初始化失败。修正检查表，保留精确校验。
3. 人工核对把 `effect/uncertain` 和 `effect/reconciled` 写在相同 producer/key 下，触发唯一性冲突并回滚。核对结果使用独立 producer；原始 effect idempotency key、input digest、uncertainty 前驱和 owner 审计仍保留。
4. 旧隔离断言把暂停期待为 failed。改为明确检查 Turn=running、head=paused、active Turn 保留、旧 lease 被 fence、cleanup 未证明和 checkpoint 保留。非 native 的未决 effect 仍阻挡新效果，保留 reconciliation head。
5. 数据库测试的迁移目录清单漏列 007；构造旧数据库时未移除 007 的列，导致重复列错误。只修正隔离测试夹具，不更改已交付迁移文件；补测 007 只修复非终态 head、保留 checkpoint/fence/旧终态，并可重复打开。
6. 桌面会话兼容投影将 paused/reconciliation 误报为 finished，使队列可能把暂停当作任务结束。保留 nonterminal aggregate、active Turn 和禁止发送状态；paused finish 不触发完成回复后处理。
7. 应用 Journal 测试夹具还引用旧 attached/AtomicBool，已更新为 attachment/AtomicU8。
8. 新增产品测试验证 reconciliation head 经已有 Store 守卫和恢复重试进入持久隔离，保留非终态、checkpoint 及未经证明的清理状态。复核后移除了不必要的直接隔离分支，沿用原有保守流程。
9. Collector 漏查实际模型观察、runtime event 的 Turn 身份和 native build 根事件。补齐这些校验；模型尚未启动的失败可保留为失败证据，不能凭空成为完成的 live 成功样本。
10. 组合产品测试发现暂停后立刻续跑会遇到旧 Runtime admission 未释放，误触发 `EXECUTION_ATTACH_FAILED`。调度现在检查真实 local generation 占位、绑定授权 cursor，串行挂接同一 Session；已接管的重复授权不会二次派发。并发准备遇到相同授权已提交时复用精确回执。三条产品路径组合重跑通过。
11. 真实 `--model-smoke` 的协作阶段明确要求一次 `agent/delegate`，但父回复验收复用了禁止所有工具的 marker 检查，必然拒绝已执行的委派。保留首次失败；协作阶段改为必须且仅一次已结算的精确 Action，纯文本阶段仍禁止工具，子执行及父回复的精确 marker 检查不变。11 个 verifier 自身测试通过。
12. Runtime build digest 漏列 V2 的暂停、核对、恢复与分段模块。将这些源文件、相关 owner/迁移纳入既有自动摘要计算，避免仅修改遗漏模块却沿用旧恢复身份；没有手改任何 digest ledger。补充构建身份覆盖检查并重跑三条产品恢复路径，均通过。
13. Coding 冒烟补齐原始任务明确要求的回读回执与最终 marker 后缀检查，避免只凭“文件存在且有任意回复”判定任务完整。

## 分阶段执行记录

### P0

| 检查 | 结果 |
| --- | --- |
| 交付基线与 118 文件哈希 | 修改前通过 |
| Runtime / Session `cargo check --tests` | 通过 |
| App `cargo check --lib --test native_execution_recovery` | 首次 generation 编译错误；修复后通过；加入最终构造场景后交付复跑再次通过（`delivery-app-check.log`） |
| `agent-v2-contract -- check` | 通过；未手改或重新生成 digest ledger |
| `git diff --check` | 交付前再次通过 |

### P1

| 检查 | 已取得结果 |
| --- | --- |
| Runtime 库交付复跑 | 110 通过 / 0 失败 / 0 ignored；包含原 102 项、6 项 reconciliation 和 2 项大量压缩构造测试 |
| Runtime reconciliation 定向 | 6 项在上述 110 项内；部分批次、未知结果、不完整参数、已完成回执、steering 顺序、累计预算 |
| Session 库交付复跑 | 73 通过 / 0 失败 / 0 ignored。保留首次 65 个初始化失败及后续 4 个剩余失败的记录；修复后的整包复跑已通过 |
| Session 执行状态机定向集 | 最终 17 通过；包括真实 owner/人工核对竞争、同键/不同键授权竞争、旧 generation 通知隔离 |
| Session 真实 payload 额度边界 | 另 1 项通过；填满默认 16 MiB 后拒绝第 17 个大 payload，明确授权增至 24 MiB 后成功；累计用量、旧内容、幂等重放和当前 Turn 读取均保留 |
| DB schema contract | 首次 5 通过 / 1 旧清单失败；该清单修复后定向通过 |
| DB migration 过滤集 | 修复后 4 通过，含新增 007 升级测试 |
| DB 旧 baseline 升级和重复打开 | 1 通过 |
| Chat model broker 库 + conformance | 17 + 32 通过 |
| Engine Core 库 | 22 通过 / 1 ignored，ignored 是 opt-in Bun 进程检查 |
| report / schema probe 脚本 | 12 通过，都是合成门禁/协议解析验证 |

### P2 / P3

| 检查 | 当前结果 |
| --- | --- |
| 崩溃快照 → 正式应用启动恢复 → 真实文件校验 | 通过；一个逻辑 Turn、一次原写入、一次恢复 |
| owner pause → resume → 同一 Turn 新 generation | 单独通过，随后组合运行暴露旧 admission 释放竞争；修复后并发重复 HTTP 授权和三条恢复路径组合通过；一个 Turn、一次写入、一次授权 |
| 暂停桌面投影 / 完成后处理 / 队列门禁 | 25 个定向 UI 测试通过；不是 native UI 验收 |
| `bun run check:desktop-ui-boundary` | 通过，仍是最低 880×600 桌面 renderer |
| Collector freeze / record / aggregate 故障测试 | 23 通过；pin/HMAC/模型/build/Turn/序列/工作区/重复/缺失/超时/输出/预算/人工介入 |
| 隔离 live runner 自检 | 通过；未调用模型 |
| App history display | 5 通过；首次夹具编译错误修复后实际执行 |
| App Chat Broker Host | 15 通过 |
| Robot paused 非成功信号 | 1 通过 |
| 启动 reconciliation head 隔离 | 通过；没有恢复模型调用，保留 checkpoint 与 cleanup 未证明 |
| Nomi Core route gap | 32 项先通过；剩余旧版启动源码断言更新为 V2 调度契约后定向通过，共覆盖 33 项 |
| SDK runtime turn isolation | 1 通过，含迟到 paused 通知拒绝 |
| Channel paused 非成功信号 | 1 通过，暂停不释放排队产物 |
| Journal 窗口与清理边界 | 3 通过；真实 SQLite 提交故障注入不续窗口、不清累计量，确认后才续期，暂停前必须经过清理阶段 |
| Runtime build identity | 覆盖检查通过；摘要扩展后三条产品恢复场景再次组合通过 |
| 大量压缩构造场景 | 2 项通过；1024 次连续压缩跨分段恢复，继续使用 1025，保留溢出纠错消耗；拒绝编号缺口、重复、0 及超过 2048 总上限。显式 stalled retry 也不清累计计数 |
| 跨天状态构造场景 | 1 项通过；通过 canonical fixture writer 建立三天前的合法暂停事实并关闭/重开数据库，无真实等待、未改写既有审计事实；仍为同一 running/paused Turn，旧 producer 失效，未授权不可恢复，授权后使用新 generation |
| 多模型统计契约 | report + collector 共 34 项通过；逐 suite 固定精确模型，禁止跨模型合并，408 零失败门槛和原有失败分母不变。模型标识构造测试不代表已经调用其他供应商模型 |

Collector 测试在一次性临时目录中使用明确的合成 capture 与独立断言进程，不进入 live corpus。它们检验采集链的拒绝行为，不证明模型效果，也不证明 verifier 的子进程树已经清理。

### P4 / P5

当前凭据授权可用，不再是历史的 HTTP 403。通过既有隔离 runner 进行了真实 `step-3.7-flash` 请求。

| 新运行 | 结果与证据范围 |
| --- | --- |
| 首次 `--model-smoke` | 失败：`cluster.lead_reply / SESSION_UNEXPECTED_TOOL_EVIDENCE / 422`。3 个模型步骤、1 次 delegation、无参数/能力/Kernel 拒绝；由上述矛盾验收规则导致，失败日志保留为 `p4-model-smoke.log` |
| 修复验收器后的新 `--model-smoke` | 通过，覆盖模型选择、协作和 AutoWork；另存 `p4-model-smoke-verifier-fixed.log`，不覆盖首次失败 |
| 真实 `--coding-smoke` | 通过：1 次写入、1 次回读、0 tool errors，独立文件内容与回复检查通过；`p4-coding-smoke.log` |
| 真实 `--long-coding-smoke` | 通过；两个 Turn 均 completed，独立测试/不可改测试文件/README 检查通过，首次独立 ledger 检查 attempt=0 即通过，无夹具追加 repair Turn。构建摘要 `565c45cfda50f2a0807f59226e5f7b376fbd84af53811f3084ec5959169c6fd3`；`p4-long-coding-smoke.log` |
| 摘要清单扩展后的 coding | 通过；中间构建 `54ce8af7ac2971eff55232a9183ac975fa5e1ab2d2071bb3db05ea328dbbc3b1`，记录 `p4-current-build-coding.log` |
| 最终源码 build coding | 通过；实际 runtime build digest 为 `c4c5337d1326ae8e16a3dd7e9b7738db5c1c2643edafefefe273ff99dcedfef1`，1 写 / 1 读 / 0 tool errors，记录 `p4-final-build-coding.log` |
| 统计规模评测 | 未执行，未冻结有效独立样本 corpus，99% 未证明 |

注意：现有 `--model-smoke` 包含模型选择、协作与 AutoWork 多段链路，并非一个可直接计入统计的一 Session/一 Turn 独立任务。上述日志是功能诊断，不是 collector 冻结计划的签名产品样本。

模型选择/协作/AutoWork 和长 Coding 回归发生在 build digest 输入清单扩展之前。扩展后的源码通过产品恢复回归，最终源码另有实际 build digest 绑定的 Coding 通过记录；不同 build 的结果不能合并成统计样本，也不把先前的长回归冒充最终 build 的长时验收。

长 Coding 的通过只表示该夹具的最终功能断言成立，不代表所有工具尝试都成功：本次有 68 个主模型步骤、74 次压缩请求（21 次形成压缩结果）、237 次 read、13 次 exec、7 次 write、2 次完成报告记录；4 次 Kernel 拒绝，完成报告出现需求/证据/计划纠错。进程非零退出还包含原任务明确要求的失败基线，不能全部等同平台故障，也不能从 first-attempt / recovered 分项中删除。没有费用账单证据，不把请求数量换算成已确认费用。

不复用 V1 的实测结果，不把编译、脚本 provider、合成数据或模型自评计入 99%。凭据仅通过既有隔离 runner 的环境输入及一次性 stdin 传递，不写进源码、报告或命令行参数。

## 未完成范围和下一步

- P0/P1 与 P2 指定定向检查已完成；P3 已覆盖上述新增用例，不能据此声称整个故障矩阵全部验收。
- 大日志/payload 极限、完整资源进程树与 Browser 的跨 generation 行为、native 桌面交互及 Linux/macOS 尚无本轮完整验收证据。
- 后续如扩大构造验证，最小动作是补齐同一 Turn 的资源进程树/Browser owner mock 代次重开、应用层缺失回执重建与 steering 附件故障；不需要启动真实跨天体验流程。这些未覆盖项不标作本次已通过。
- P5 真实统计和隔离环境部署按用户最新要求不在本次交付范围内。没有把构造数据、脚本 provider 或短冒烟充作真实跨天体验或正式签名 corpus，99% 仍未证明。
- 长周期复杂代码工程、大量压缩、跨天状态及多模型保持为设计目标；当前用构造场景覆盖相应状态机。运行时已有授权续期、累计硬上限、取消与清理边界仍保留，不因“预算理论上无限制”而自动清零或无限续期。
