# Agent Capability Platform v2 实施状态

> 最后更新：2026-08-29
>
> 本文件是跨任务、跨机器继续实施的唯一人工可读状态入口。机器契约、
> Gate input 与 evidence 仍分别由 canonical contract、manifest 和 ledger
> 持有；本文件不替代它们。

## 仓库与参考基线

- branch：`rf/agent-capability-platform-v2`
- base SHA：`7a2ade3c49374add25a35565265399c57729a8b9`
- current SHA：`SELF`（包含本文件的 C0 implementation commit；检出后用
  `git rev-parse HEAD` 解析）
- last verified remote SHA：`7a2ade3c49374add25a35565265399c57729a8b9`
- origin：`https://github.com/nomifun/nomifun-tauri.git`
- worktree：clean；启动时无用户未提交改动
- Git identity：`colir0 <colir0@qq.com>`
- DeepSeek Harness：
  - expected/current：`cd5ef8148158c3a752a658978873241fdf8e2bbc`
  - tag：`dsh-v0.1.2-alpha.1`
  - 状态：匹配调查基线
- Codex：
  - frozen investigation SHA：`dc2ccc6843abb09c9d297862dc10b6bd12a3935d`
  - sibling checkout SHA：`4ee04c0aa5833ac39b1763f6ea44c7bc777c83dd`
  - 状态：当前 checkout 比冻结基线前进 16 commits；冻结 SHA 仍在本地对象库中且
    是当前 HEAD 的祖先。C0/C4 必须显式 pin 冻结 SHA，不得把当前 HEAD 静默当作
    调查基线。

## 当前阶段

- 当前 boundary：`C0 Contract Closure / G0` 已闭合；下一 boundary 为
  `C1 FullAuto physical deletion`
- Review A：
  - Decision Closure：PASS，D-001～D-028（含 D-019）均已确认
  - Contract Closure：PASS
- G0 状态：PASS；machine-readable canonical sources、跨文档冲突、
  target first-party inventory、`OfficialPresetSeedManifest`、SessionEvent
  Registry、fresh-v4 schema contract、D-014/D-025～D-028 fixtures、generated
  schemas/envelopes 与 digest 已闭合
- production behavior：未开始；C1 之前不得物理删除 mode/approval，也不得接入
  production migration/seed/composition

## Contract Closure 裁决

下列内容属于已确认决策的机械闭合，不是新的产品方案：

1. canonical source 固定为三类机器源：Rust contract types、fresh-v4 schema、
   SessionEvent Registry；Markdown 只解释，不再充当第二份 schema。
2. 所有含 digest 的 artifact 使用 `payload + envelope`：digest 只覆盖 canonical
   payload，不覆盖自身字段、运行状态、evidence、日志或 summary。
3. capability search/activate 是固定内部 Runtime protocol control operation，不是
   Capability，也不进入 `initial_capabilities`。
4. 唯一持久 activation Event 为 `capability/active-set-committed`；requested/failed
   只允许作为 transient diagnostic。
5. Snapshot 固定 Runtime protocol/Profile/feature contract，不保存实际 Runtime build；
   实际 build 只存在于 `runtime/bound` Event。
6. 普通插件只获得窄 `PluginRegistrar + PluginContext`；不得获得 root
   `PluginHost`、SQLite、Registry、Session、Model、EventBus、`AppServices` 或
   `GatewayDeps`。
7. cutover archive basename 统一为
   `<canonical>.pre-v4-archive-<UTC timestamp>`。
8. parent operation marker 一旦 durable，在 rename/mkdir 失败后保持不可变并留作
   crash fence；恢复只用 exact path/ready/metadata 重试或 fail-stop。
9. fresh 初始化顺序统一为 baseline/`schema_metadata` → bundled Package
   materialization → seven-template authoring seed → ready → durable remove marker。
10. canonical error 使用大写稳定 code；缺少目录物化统一为
    `CAPABILITY_NOT_MATERIALIZED`，Snapshot 外调用统一为
    `CAPABILITY_NOT_IN_PRESET`。
11. RC→Stable 复用相同 signed release artifact/content digest；发布渠道 metadata
    不得改变 artifact bytes 或重新签名另一份制品。
12. Stable 可编辑已挂载 bundled Package 的 schema-backed config；仍不提供用户
    install/enable/disable/uninstall/SDK/market。

## Workstream Owner

- W1 Platform Foundation & Fresh-v4：主 Agent（central integration）+
  `Schrodinger`（Package contracts）+ `Dirac`（Session/Event contracts）
- W2 Codex Runtime & Providers：`Aquinas`
- W3 Product Control Plane：`Halley`
- W4 Domain Migration & Inline Demolition：`Dewey`
- W5 Shared Integration, Hard Delete & Release：主 Agent（central integration）+
  `Bernoulli`（validation contracts）

## Closed C0 Disjoint Write Sets

机器可读清单：
`docs/specs/2026-08-28-agent-capability-platform-v2/C0-WRITE-MANIFESTS.json`

- `C0-W1-PACKAGE-CONTRACTS`：closed
- `C0-W2-RUNTIME-CONTRACTS`：closed
- `C0-W3-PRESET-REMOTE-CONTRACTS`：closed
- `C0-W4-DELETION-INVENTORY`：closed；已校正 `AgentEngine`、`ProbeDeps`、
  `ConversationAttemptRunner` 的真实声明路径/名称
- `C0-W5-VALIDATION-CONTRACTS`：closed
- `C0-W1-SESSION-EVENT-CONTRACTS`：closed

## Central File Queue

仅主 Agent 可写：

- `Cargo.toml`
- `Cargo.lock`
- `package.json`
- `crates/backend/nomifun-agent-contracts/Cargo.toml`
- `crates/backend/nomifun-agent-contracts/src/lib.rs`
- `crates/backend/nomifun-agent-contracts/src/digest.rs`
- `crates/backend/nomifun-agent-contracts/src/schema.rs`
- `crates/backend/nomifun-agent-contracts/schema/**`
- `scripts/gate-agent-v2.mjs`
- 本规格目录中的 README/DECISIONS/01/02/03/04/START/STATUS/write manifests

C0 central queue 已清空。C1 开工前必须创建新的 disjoint write manifest；不得沿用
C0 路径授权。

## Canonical Cohort Tuple

C0 尚未生成原生候选；以下字段均为 not-applicable，而不是 pass：

- candidate_source_sha：`not-applicable-before-c8`
- confirmed_decision_contract_digest：
  `b45efce157933d72671a9158ff87d4a84b5b288bc8ec6bf3688226497c6e0cf5`
- platform_validation_manifest_digest：`not-applicable-before-c8`
- runtime_release_digest：`not-applicable-before-c8`

C0 contract/golden digests：

- canonical v4 schema manifest：
  `d744f3e97d894bb2fe40a109f4e31e38c51401d42de93c9c27424eaf5dedf8fc`
- contract digest ledger：
  `a049c67f3ff18b9eb46abdea71f0994950a190b6414b1498c2f20945a18efcd1`
- official preset seed manifest：
  `c2684efb05f8540c3f61da95e6cee9f8d6f1bab7867ae405819efc568e8449d8`
- runtime protocol：
  `f1c0422f04c9de923e18c7df40d814d3c9f5b2db5f1c5fef2745e77e6d62590f`
- runtime feature inventory：
  `bc01fffa050a721debc7740405a05f53b966d4e2dc2d8b4392e321d944fca2ee`
- platform validation contract：
  `78f264e177efafceb5ca55e4642fead82fa56e5e92bce355ccc79b774126f5f9`
- runtime release fixture：
  `5a9823e6860e9f39192a0cfb9c46281f4d8ec68b1277a7e3119ab5d9366fb416`
- platform validation fixture：
  `b4015cec8f8d88ca857bc3e98babdef1937be8b82564ec9002a8620426a2f113`

## Platform Verification

- 当前 Host：Windows x64
- C0 只执行 host-independent contract/schema checks
- pending PlatformVerificationPoints：0（C1 之后触及跨平台实现时开始登记）
- affected cells：无
- native pass：0
- whole-cohort recheck：未开始

## 已运行验证

- `git status --short --branch`
- `git worktree list --porcelain`
- `git ls-remote origin refs/heads/rf/agent-capability-platform-v2`
- Git identity、兄弟仓 branch/HEAD/origin/worktree、冻结基线祖先关系核验
- `cargo metadata --no-deps --format-version 1`（只读 inventory）
- 规范 heading/MUST/contract/residual 定向扫描
- `git diff --check`（启动时 clean）
- `cargo fmt --package nomifun-agent-contracts -- --check`
- `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`
- `cargo test -p nomifun-agent-contracts`：15 passed
- `bun run gate:agent-v2 -- contract-closure`：PASS；60 个 JSON payload；
  generator check、targeted crate tests、`git diff --check` 全绿

## 未运行验证

- workspace `cargo test`：C0 不属于允许的 C6/C8-WIN-PRE/C10-WIN 节点族
- broad `bun run check`：C0 尚未触及 product/UI；只运行后续 contract 定向 Gate
- macOS/Linux native checks：当前阶段无 native candidate，不能标 pass

## Closed Slices / Commits / Evidence

- C0 六个 contract slices：closed
- 设计基线 commit：`7a2ade3c49374add25a35565265399c57729a8b9`
- C0 implementation commit：`SELF`
- C0 evidence：
  `build.noindex/agent-capability-v2/7a2ade3c49374add25a35565265399c57729a8b9/contract-closure/summary.json`
- C0 generated ledger：
  `crates/backend/nomifun-agent-contracts/contracts/generated/contract-digest-ledger.envelope.json`

## 下一批可直接执行

1. 提交并普通 push C0，核对 local/origin/`git ls-remote` SHA。
2. 创建 C1 disjoint write manifests。
3. 物理删除 FullAuto 之外的 mode/approval/confirmation Rust/API/UI/DB/Event/i18n/tests。
4. 运行受影响 crate/UI 定向检查与 route/DTO/Event residual/reachability Gate。
5. C1 闭合后按 disjoint paths进入 C2～C5。

## 真实 Blocker

- 无外部 blocker。
- Codex sibling checkout 漂移已记录；冻结 SHA 本地可读，不阻止 C0。
