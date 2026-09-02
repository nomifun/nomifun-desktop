# 机器 2 启动 Prompt：Phase 1 Batch A

> 状态：**READY WHEN ALL FOUR LANES ARE ACCEPTED**
>
> 修订日期：2026-09-02
>
> 代码基线：执行时从 `origin/rf/agent-capability-platform-v2` 解析 clean HEAD，并在结果中记录实际 SHA。
>
> 分支：`rf/m2-phase1-batch-a`
>
> 权威指令：`05-system-capability-replacement-foundation.zh.md`
>
> 机器清单：`MACHINE-2-PHASE1-BATCH-A-MANIFEST.json`
>
> 结果模板：`MACHINE-2-PHASE1-BATCH-A-RESULT-TEMPLATE.json`（另有人工填写版 `.zh.md`）

只有在机器 2 能完整承担本文件的四项工作时才启动。不得只领取 SSH 或任意单项；
如果可执行项不足，机器 2 保持停用，由主机继续排期。

```text
你正在执行 NomiFun 一期 Phase 1 Batch A。目标是通过一台独立机器同时推进三个互不
冲突的 writer，共关闭或实质推进四个 TODO，而不是为单个 SSH 任务支付交接成本。

一、先读

1. AGENTS.md
2. docs/specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md
3. docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md
4. 本文件
5. docs/specs/2026-08-28-agent-capability-platform-v2/MACHINE-2-PHASE1-BATCH-A-MANIFEST.json

05 与旧设计、旧 Prompt、历史 Gate 或聊天摘要冲突时，以 05 为准。

二、Git 基线

git fetch origin --prune
git status --porcelain

如果工作树不是空的，立即停止并回传，不要在未审查的 WIP 上同步。

git rev-list --left-right --count origin/rf/m2-phase1-batch-a...origin/rf/agent-capability-platform-v2

如果上一个命令的左侧数字不是 `0`，说明机器 2 集成分支存在主机尚未审查的提交，
立即停止并回传，不要强行同步。左侧为 `0` 后，使用普通 fast-forward 更新集成分支：

git switch rf/m2-phase1-batch-a
git merge --ff-only origin/rf/agent-capability-platform-v2
git push origin rf/m2-phase1-batch-a

如果本地没有该分支，先执行：

git switch --track -c rf/m2-phase1-batch-a origin/rf/m2-phase1-batch-a

随后为每个 writer 从更新后的集成分支创建独立 worktree/子分支。
记录实际基线：

git rev-parse HEAD

要求：

- 工作树为空，实际基线 SHA 已记录；
- 不 reset、不 force-push、不改写共享历史；
- 不把其他开发分支整体 merge 进来；
- API key、token、主机凭据和私钥不得进入 Git、日志、fixture 或 Prompt。

三、内部并发结构

最多同时运行三个 writer。每个 writer 使用独立 worktree/子分支，写集不得交叉。

Writer A：Session Data

- 先执行 `SL-S2-05`，再执行 `SL-S2-06`，两项串行；
- 独占写集：
  - crates/backend/nomifun-agent-session/src/**
  - crates/backend/nomifun-agent-session/tests/**
- 禁止修改 agent-contracts、Fresh-v4 schema、Compiler、Kernel、App、Gate；
- 若真实完成必须修改上述中央文件，记录最小接线需求并停止，不自行越界。

Writer B：SSH Owner

- 执行 `SL-S3-09`；
- 独占写集：
  - crates/backend/nomifun-ssh/src/**
  - crates/backend/nomifun-ssh/tests/**
  - crates/shared/nomi-ssh/src/**
  - crates/shared/nomi-ssh/tests/**
- 不修改 Cargo.toml、Cargo.lock 或中央 host。

Writer C：Sidecar Upstream Spike

- 执行 `SL-S2-10`；
- 允许写集：
  - 新建 docs/specs/2026-08-28-agent-capability-platform-v2/SIDECAR-UPSTREAM-SPIKE.zh.md
  - 新建 scripts/validation/codex-app-server-spike.mjs
  - 新建 scripts/validation/codex-app-server-spike.test.mjs
- 不修改生产 Runtime、vendor fork、Cargo.lock、Gate 或 app composition；
- 先验证官方 app-server，再决定是否需要窄 patch，不预设自定义 RPC 必须保留。

Writer C 不得修改机器清单或结果模板。结果按
`MACHINE-2-PHASE1-BATCH-A-RESULT-TEMPLATE.zh.md` 复制填写并回传。

主机同时独占 `SL-S2-07` canonical Compiler。机器 2 禁止修改：

- crates/backend/nomifun-agent-control-plane/**
- crates/backend/nomifun-agent-kernel/**
- crates/backend/nomifun-agent-contracts/**
- crates/backend/nomifun-v4-root/**
- crates/backend/nomifun-app/**
- scripts/gate-agent-v2.mjs
- GLOBAL-CLOSURE-TODO.zh.md
- docs/specs/2026-08-28-agent-capability-platform-v2/MACHINE-2-PHASE1-BATCH-A-MANIFEST.json
- docs/specs/2026-08-28-agent-capability-platform-v2/MACHINE-2-PHASE1-BATCH-A-RESULT-TEMPLATE.json
- docs/specs/2026-08-28-agent-capability-platform-v2/MACHINE-2-PHASE1-BATCH-A-RESULT-TEMPLATE.zh.md
- Cargo.toml、Cargo.lock

四、Writer A 完成定义

`SL-S2-05`：

- SessionEvent 保持唯一语义事实；
- message projection 不再复制完整 events[]；
- 正常完成只持久化最终 assistant message；
- 中断最多保留一份有界 partial；
- 不增加第二套 event log、receipt 或兼容双写。

`SL-S2-06`：

- Effect 策略只保留 read_only、managed_effect、external_uncertain_effect；
- 本地效果依赖现有事务/CAS/原子文件操作；
- 外部 unknown result 不自动 retry；
- 删除 Session crate 内无真实消费者的通用 started/succeeded/failed/uncertain/reconciled
  扩张与重复 receipt；
- 不建立全局 EffectCoordinator。

每项独立提交，不把两个改动压成一个无法单独审查的提交。

五、Writer B 完成定义

- `ssh.fs.read`、`ssh.fs.write`、`ssh.exec`、`ssh.sudo` 有真实最小 typed owner；
- 使用现有 host book、credential authority、pool、SFTP/shell；
- path、payload、output、timeout 有界；
- exec 与 sudo credential 严格分离；
- host-key changed、disconnect、timeout、cancel 明确失败；
- write/exec/sudo 结果未知时不自动重放；
- Secret 不进入 command、outcome、Debug、Display、日志或测试。

不建设绝对原子证明、通用 uncertain 平台、中央 Effect journal 或长期旧 API 兼容层。

六、Writer C 完成定义

在明确 pinned 的官方 Codex app-server checkout 上记录并验证：

- initialize/version；
- thread create/resume；
- turn start/event；
- cancel；
- Host-managed Tool callback；
- 正常关闭与 Host 终止整棵进程树。

输出必须说明：

- upstream commit；
- 实际调用 trace；
- 哪些能力无需 patch；
- 是否缺少必要 pre-effect seam；
- 如确需 patch，只给一个最窄建议，不直接修改生产 fork；
- live model/credential 未提供时，记录一次阻塞并停止，不构造 PASS。

七、验证

Writer A：

cargo fmt -p nomifun-agent-session -- --check
cargo check --locked -p nomifun-agent-session
cargo test --locked -p nomifun-agent-session --lib -- --test-threads=1

Writer B：

cargo fmt -p nomi-ssh -p nomifun-ssh -- --check
cargo check --locked -p nomi-ssh -p nomifun-ssh
cargo test --locked -p nomi-ssh --lib
cargo test --locked -p nomifun-ssh --lib
cargo test --locked -p nomifun-ssh --tests -- --test-threads=1

Writer C：

bun test scripts/validation/codex-app-server-spike.test.mjs
bun scripts/validation/codex-app-server-spike.mjs --self-test

所有 writer：

git diff --check
git status --short

测试遇到环境/harness 障碍时只记录首个完整失败，给出人工步骤并停止重试。

八、提交与集成

建议提交顺序：

1. refactor(agent): shrink session projection
2. refactor(agent): simplify effect strategies
3. feat(ssh): add minimal agent capability owners
4. docs(runtime): record official app-server spike

各 writer 先推自己的子分支；机器 2 integration owner 按上述顺序普通 merge/cherry-pick
到 `rf/m2-phase1-batch-a`，解决冲突后运行各自定向测试。禁止 squash 掩盖任务边界。

最终：

git push -u origin rf/m2-phase1-batch-a

回传必须包含：

- base SHA、Batch 最终 SHA、每个独立提交 SHA；
- 每个 writer 的 changed paths；
- 每条验证命令与 PASS/FAIL；
- 未运行项及准确阻塞原因；
- 需要主机完成的最小接线；
- 未触碰中央 Compiler/Kernel/App/Gate/Cargo.lock 的证明；
- 相比单 SSH lane 获得的实际并发工作量。
```
