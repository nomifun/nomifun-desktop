# Agent Capability Platform v2 实施状态

> 最后更新：2026-08-29
>
> 本文件是跨任务、跨机器继续实施的唯一人工可读状态入口。机器契约、
> Gate input 与 evidence 仍分别由 canonical contract、manifest 和 ledger
> 持有；本文件不替代它们。

## 仓库与参考基线

- branch：`rf/agent-capability-platform-v2`
- base SHA：`7a2ade3c49374add25a35565265399c57729a8b9`
- current implementation SHA / C6 code base：
  `1dedfedc782b81389ead27858963d0a943f4d142`
- last verified remote implementation SHA：
  `1dedfedc782b81389ead27858963d0a943f4d142`
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

- current boundary: `C6 Chat + Coding + sample.echo Triad` (CLOSED)
- next boundary: `C7 Windows continuous domain waves`
- Review A：
  - Decision Closure：PASS，D-001～D-028（含 D-019）均已确认
  - Contract Closure：PASS
- G0 状态：PASS；machine-readable canonical sources、跨文档冲突、
  target first-party inventory、`OfficialPresetSeedManifest`、SessionEvent
  Registry、fresh-v4 schema contract、D-014/D-025～D-028 fixtures、generated
  schemas/envelopes 与 digest 已闭合
- production behavior: C1 FullAuto, C2 Fresh-v4, C3 Kernel/Plugin, C4 Runtime/Model,
- C5 Preset Product and C6 Chat/Coding/sample.echo triad are closed; C7 Windows
  domain waves continue on the canonical AgentSession/Plugin/Runtime chain

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

## Closed C1 Disjoint Write Sets

机器可读清单：
`docs/specs/2026-08-28-agent-capability-platform-v2/C1-WRITE-MANIFESTS.json`

- `C1-W2-PROTOCOL-CLI`：closed
- `C1-W2-AGENT-CORE`：closed
- `C1-W1-EXECUTION-CONTRACTS-DB`：closed
- `C1-W4-BACKEND-RUNTIME-CONSUMERS`：closed
- `C1-W4-GATEWAY-REMOTE`：closed
- `C1-W3-UI-I18N`：closed

保留项：Coding review、OS permission、Channel pairing authorization、普通产品确认。
删除项：Agent mode/tool approval/plan approval/wait/confirmation 主链。

## Closed C2～C5 Disjoint Write Sets

机器可读清单：
`docs/specs/2026-08-28-agent-capability-platform-v2/C2-C5-WRITE-MANIFESTS.json`

- `C2-W1-FRESH-V4-ROOT`：closed
- `C3-W1-SESSION-STORE`：closed
- `C3-W1-KERNEL-PLUGIN`：closed
- `C4-W2-CODEX-RUNTIME`：closed
- `C4-W2-MODEL-BROKER`：closed
- `C5-W3-CONTROL-PLANE`：closed
- `C5-W3-AGENT-SETTINGS-UI`：closed

C2～C5 只建立最终 foundation/control-plane；业务域 composition demolition
仍等待 C6 三联 Gate 后的 C7 owning slice。

## Closed C6 Disjoint Write Sets

Machine-readable manifest:
`docs/specs/2026-08-28-agent-capability-platform-v2/C6-WRITE-MANIFESTS.json`

- `C6-PLATFORM-CORE`: closed
- `C6-CHAT-MINIMAL`: closed
- `C6-CODING-CODEX`: closed
- `C6-SAMPLE-ECHO`: closed
- `C6-APP-COMPOSITION`: closed
- `C6-UI-INTEGRATION`: closed

C6 exact candidate: `1dedfedc782b81389ead27858963d0a943f4d142`.
C6 closure record:
`docs/specs/2026-08-28-agent-capability-platform-v2/C6-CLOSURE.json`.
Triad gates, fully serialized workspace cargo test, focused UI/i18n, and bun build:ui passed.
The repository-wide bun check baseline failure is recorded separately.

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

C0 central queue 已清空。C1 central queue 仅包含 root Cargo/lock、Gate、状态、
`triad-core` deletion manifest 与跨 slice 冲突合流。

## Canonical Cohort Tuple

C0 尚未生成原生候选；以下字段均为 not-applicable，而不是 pass：

- candidate_source_sha：`not-applicable-before-c8`
- confirmed_decision_contract_digest：
  `b45efce157933d72671a9158ff87d4a84b5b288bc8ec6bf3688226497c6e0cf5`
- platform_validation_manifest_digest：`not-applicable-before-c8`
- runtime_release_digest：`not-applicable-before-c8`

C0 contract/golden digests：

- canonical v4 schema manifest：
  `f0a1c03696ed180db6786f781282d3a2b81dbea91ac286972b710dd7fe842ed7`
- contract digest ledger：
  `e26006ad01cd4918ce53ca430fc95aaaf36a04a160467c7432631617145e294a`
- deletion manifest set：
  `13431f76e07398c06dc9e42ccb5b70c701297451551c2fcf907c78fcab8f41ad`
- official preset seed manifest：
  `c2684efb05f8540c3f61da95e6cee9f8d6f1bab7867ae405819efc568e8449d8`
- runtime protocol：
  `f1c0422f04c9de923e18c7df40d814d3c9f5b2db5f1c5fef2745e77e6d62590f`
- runtime feature inventory：
  `bc01fffa050a721debc7740405a05f53b966d4e2dc2d8b4392e321d944fca2ee`
- platform validation contract：
  `78f264e177efafceb5ca55e4642fead82fa56e5e92bce355ccc79b774126f5f9`
- runtime release fixture：
  `0c029dd60f53c761bce3451de66c678e95314b354e229e84fde632c70dd8b55f`
- platform validation fixture：
  `e89a51d0e11f9e9080cd1cfd860eeea4016f998b0c31eead0096257811e1a284`

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
- `cargo check --locked`（C1 affected Rust production cohort）：PASS
- C1 targeted Rust tests：Gateway 124、Public 28、AgentExecution 86、
  Conversation 537、IDMM 191、Cron/API/Channel targeted suites 全绿
- C1 Agent Core focused tests：662 passed；Browser/Config/MCP/Skills checks 全绿
- C1 targeted UI cohort：52 passed；`bun run check:i18n` PASS
- `bun run gate:agent-v2 -- c1-fullauto`：PASS
- C2～C5 foundation Rust tests：76 passed
  - Fresh-v4 10、AgentSession 9、Kernel/Plugin 12
  - Codex Runtime 31、ChatModelBroker 9、Control Plane 5
- C5 Agent Settings UI：15 passed；i18n 7050 keys / 33 modules
- `bun run typecheck`：C2～C5 changed-line diagnostics=0；repository baseline
  仍有 unrelated Arco/React typing debt
- `bun run gate:agent-v2 -- c2-c5-foundations`：PASS

## C6 Validation Record

- `bun run gate:agent-v2 -- c6-triad`: PASS; Chat 2, Coding 2, sample.echo 2, app route E2E 1, focused UI 23
- workspace `cargo test`: PASS at exact SHA `1dedfedc782b81389ead27858963d0a943f4d142` using `cargo test --locked --jobs 1 -- --test-threads=1`
  evidence: `build.noindex/agent-capability-v2/1dedfedc782b81389ead27858963d0a943f4d142/c6-workspace-fully-serialized/cargo-test.log`
- `bun run build:ui`: PASS at exact SHA `1dedfedc782b81389ead27858963d0a943f4d142`
- `bun run check:i18n`: PASS; 7074 keys / 33 modules
- focused C6 UI tests: 23 passed / 0 failed
- `bun run check`: baseline failure, exit 2; evidence: `build.noindex/agent-capability-v2/1dedfedc782b81389ead27858963d0a943f4d142/c6-ui/bun-check.log`
  Reported errors are existing React/Arco typings and matcher definitions; C6 production build and focused changed-surface checks pass.
- published legacy migrations: unchanged from C1 checkpoint
- macOS/Linux native checks: pending; Windows cannot attest other native cells

## Closed Slices / Commits / Evidence

- C0 六个 contract slices：closed
- 设计基线 commit：`7a2ade3c49374add25a35565265399c57729a8b9`
- C0 implementation commit：`84da71b7377967726552b7f80ce54ff1e4433feb`
- C0 evidence：
  `build.noindex/agent-capability-v2/84da71b7377967726552b7f80ce54ff1e4433feb/contract-closure/summary.json`
- C0 generated ledger：
  `crates/backend/nomifun-agent-contracts/contracts/generated/contract-digest-ledger.envelope.json`
- C1 implementation commit：
  `ab6166e2c33758a560e2cd7f98f6e7bc0a39aeb1`
- C1 Contract Closure evidence：
  `build.noindex/agent-capability-v2/ab6166e2c33758a560e2cd7f98f6e7bc0a39aeb1/contract-closure/summary.json`
- C1 FullAuto evidence：
  `build.noindex/agent-capability-v2/ab6166e2c33758a560e2cd7f98f6e7bc0a39aeb1/c1-fullauto/summary.json`
- C2～C5 implementation commit：
  `6e1b7338ae3c3181d14366cc3d52d30f64b45285`
- C2～C5 foundation evidence：
  `build.noindex/agent-capability-v2/6e1b7338ae3c3181d14366cc3d52d30f64b45285/c2-c5-foundations/summary.json`
- C2～C5 Contract Closure evidence：
  `build.noindex/agent-capability-v2/6e1b7338ae3c3181d14366cc3d52d30f64b45285/contract-closure/summary.json`

- C6 implementation commit: `04bb08f9`
- C6 final validated candidate: `1dedfedc782b81389ead27858963d0a943f4d142`
- C6 triad evidence: `build.noindex/agent-capability-v2/1dedfedc782b81389ead27858963d0a943f4d142/c6-triad/summary.json`
- C6 workspace evidence: `build.noindex/agent-capability-v2/1dedfedc782b81389ead27858963d0a943f4d142/c6-workspace-fully-serialized/cargo-test.log`
- C6 closure record: `docs/specs/2026-08-28-agent-capability-platform-v2/C6-CLOSURE.json`

## Next Directly Executable Batch

1. Create and freeze C7 Windows domain-wave disjoint write manifests.
2. In parallel, switch Wave 1-5 consumers to canonical PluginFactory/AgentBinding/AgentSession paths and perform same-change demolition.
3. Run each C7 slice targeted compile/test/fault/residual/reachability gates; continue Windows without feature/module/platform handoff until C7 closes and C8-WIN-PRE begins.

## Blocking Status

- No external blocker.
- Codex sibling drift is recorded; the frozen investigation SHA remains locally available and does not block C7.
- Repository-wide React/Arco typecheck baseline debt is recorded; focused UI tests and production build pass.
