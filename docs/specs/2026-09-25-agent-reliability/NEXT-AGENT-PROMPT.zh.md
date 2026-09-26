# 给下一位 agent 的启动上下文

以下内容可直接交给另一台电脑上的开发/测试 agent：

---

你接手 NomiFun Agent 执行可靠性重构。用户要求先完成整体开发，再按阶段验证，以降低重复开发成本。
不要重新从零调研或重写架构；先核对已交付源码，再定位具体失败。目标是提高工具调用、长任务稳定性和独立交付质量，但目前没有证明达到 99%。

先读：

1. 根目录 `AGENTS.md`。
2. `docs/specs/2026-09-25-agent-reliability/DEVELOPMENT-HANDOFF.zh.md`。
3. 同目录 `TEST-MATRIX.zh.md` 和 `README.zh.md`。
4. `git status --short`，运行 `node scripts/validation/agent-reliability-handoff.mjs --check` 核对源码。
5. PAUSE-RESUME-V2.zh.md 和 EVIDENCE-PIPELINE.zh.md。当前是 V2 源码交付，没有运行应用编译或测试，不能把 V1 检查记录算成当前通过。

大量修改尚未提交，关键新模块/迁移可能未跟踪。不要 reset、checkout 丢改动，也不要只转移 git diff 而漏掉新文件。
未获得明确请求，不提交、强推或复制凭据/用户数据库。

主链是 Runtime → Host → Journal → canonical Agent Store。检查点在 `agent_turns`，事件仍在原来的 `agent_events`。
恢复要核对 exact owner/Session/Turn/Snapshot/build/generation/cursor/digest，并取得 lease/fence；checkpoint 不是权限。
分段保留同一个任务和递增调用号。只能在静止边界、检查点确认后续期，总量和无进展限制不清零。
无法确认的 effects 不能重试：V2 记录非终态 pause、保留 checkpoint 和 active Turn，认证 owner 核对并授权后继续。真正的旧终态不重开，完成/取消清理 checkpoint。
API 提供 pause/resume/effects/reconcile。核对观察不是新的 ToolStarted，人工回执不是原始 owner 首次成功；清理不确定的宿主保持 quarantine。
SQLx migration 到 007，Agent Store head=6。暂停状态独立保存，底层非终态 Turn 仍 running；不要改成可随意重开的 failed。
原任务、用户补充输入顺序、权限与测试禁止条款始终保留；旧验证证据不能当作当前验证。

优先按矩阵 P0 → P1 → P2 → P3 执行。修复具体失败后只重跑相关项，不默认全仓库或真实模型大批量。
P0 的 cargo check 不执行测试。交付记录中的“新增用例已写”不是“已经通过”。
collector record 会执行独立 grader，必须到获准测试阶段才调用，本轮没有运行。签名密钥只给 verifier harness，不给 Agent/app，不复用模型 API key。
如命令缺失/平台不支持，记录原因和未跑范围，不编造通过。

真实 StepFun `step-3.7-flash`：此前 coding 冒烟有过一个成功样本；collaboration 和 long-coding 未通过；最近正式应用请求 HTTP 403，未运行工具。
凭据不在交接文件里。不要恢复或反复使用旧 key；需要可用授权后通过既有隔离 runner 开展 P4，再考虑 P5。

只支持桌面 renderer，最低 880×600。不增加手机/平板布局或移动模拟测试。若改 renderer/UI 规则，运行 `bun run check:desktop-ui-boundary`。

每次交付必须区分：代码改动、实际执行的检查、通过/失败、未执行项、已知限制和下一个最小验证动作。
统计门禁不能用 mock 重复次数充数，也不能把模型自评或修复后重跑覆盖失败样本。

---
