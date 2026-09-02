# Agent Capability Platform v2 实施状态

> 最后更新：2026-09-02
>
> 本文件是跨任务、跨机器继续实施的唯一人工可读状态入口。机器契约、
> Gate input 与 evidence 仍分别由 canonical contract、manifest 和 ledger
> 持有；本文件不替代它们。

## 仓库与参考基线

- branch：`rf/agent-capability-platform-v2`
- base SHA：`7a2ade3c49374add25a35565265399c57729a8b9`
- last clean C8 execution SHA（Windows 合流前历史 evidence）：
  `b849e2ac7c3356468f86064a47180f5442a8e0a6`
- 本轮合流前已核对的 remote SHA：
  `45655e80d3e4fd534e2903d2059a75748af6c5a4`
- origin：`https://github.com/nomifun/nomifun-desktop.git`
- current implementation checkpoint：以本分支完成合流后的 clean Git HEAD 为准；
  C8 evidence 只接受 clean source checkpoint，历史 SHA 不得冒充当前候选。
- Git identity：`colir0 <colir0@qq.com>`
- DeepSeek Harness：
  - expected/current：`cd5ef8148158c3a752a658978873241fdf8e2bbc`
  - tag：`dsh-v0.1.2-alpha.1`
  - 状态：匹配调查基线
- Codex：
  - frozen investigation SHA：`dc2ccc6843abb09c9d297862dc10b6bd12a3935d`
  - sibling checkout：Windows 工作区存在 `../codex`，冻结 SHA 可达。
  - 状态：冻结 upstream 不包含 NomiFun 所需的 `runtime/hello`、
    `runtime/session/dispose` 和 `native_action/start`；实际 sidecar 仍需按
    release input 单独提供和验证，普通 `codex.exe` 不得冒充。

## 当前阶段

- current boundary: `C8-WIN-PRE Windows pre-candidate`
  （上一 clean Gate 已完成并定向修复，正式全量证据仍未闭合）
- next boundary: `C8-MERGE` after five native cells and D-027 zero evidence
- Review A：
  - Decision Closure：PASS，D-001～D-028（含 D-019）均已确认
  - Contract Closure：PASS
- G0 状态：PASS；machine-readable canonical sources、跨文档冲突、
  target first-party inventory、`OfficialPresetSeedManifest`、SessionEvent
  Registry、fresh-v4 schema contract、D-014/D-025～D-028 fixtures、generated
  schemas/envelopes 与 digest 已闭合
- production behavior: C1 FullAuto、C2 Fresh-v4、C3 Kernel/Plugin、
  C4 Runtime/Model、C5 Preset Product 与 C6 Chat/Coding/sample.echo 三联已闭合。
  C7 的五个 Domain Wave 已闭合 registration/inventory 与 typed host boundary，
  但 Fresh-v4 的业务 action 可执行性仍不完整，不能据此宣称七模板/all-scene 已闭合。

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

C0 contract/golden digests（历史基线；不是当前 C8 cohort tuple）：

- canonical v4 schema manifest：
  `e28723d7fc524cfdd351c6fc8cc17b8a48d8fd1f5be16a7aebd395ce669f98ff`
- contract digest ledger：
  `7b8e1941df5340a7fb59c61dc96a5871575c89c952c4919271fe7f32fe8bb8d4`
- deletion manifest set：
  `0fb2e5abf2638e3c3352d4549ddb6a46b5a29ee7d62d35aedd9319a8fb5feecb`
- official preset seed manifest：
  `c2684efb05f8540c3f61da95e6cee9f8d6f1bab7867ae405819efc568e8449d8`
- runtime protocol：
  `f1c0422f04c9de923e18c7df40d814d3c9f5b2db5f1c5fef2745e77e6d62590f`
- runtime feature inventory：
  `bc01fffa050a721debc7740405a05f53b966d4e2dc2d8b4392e321d944fca2ee`
- platform validation contract：
  `78f264e177efafceb5ca55e4642fead82fa56e5e92bce355ccc79b774126f5f9`
- runtime release fixture：
  `b9dce00732f6d1c45cb20fc30e7a286518d505d7faeb2d94b6cc70d9e107289d`
- platform validation fixture：
  `70f23b52f309aeb0938ad86c987958d3f1a05e6c367263c3b73a3038e1ca2ed2`

## Platform Verification

- 当前 Host：macOS arm64（Darwin，`sysctl.proc_translated=0`）
- 当前执行上下文：C8-WIN-PRE tuple 刷新后的 macOS arm64 工程定向验证；
  不是 C8-MA native candidate
- pending PlatformVerificationPoints：4（macOS arm64/x64、Linux Desktop x64、Linux Headless x64）
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

以下 evidence 路径是历史 run 的引用；`build.noindex/` 属于 ignored
本地产物，当前 checkout 不保证这些文件存在。它们未被本轮重新生成，不作为
当前 C8/native 验证依据。

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

## C7 Closure Record

- C7 implementation candidate: `c5e1015f948cf1fd12ee3bc7dbea2d297f3e0ab6`
- C7 manifest: `docs/specs/2026-08-28-agent-capability-platform-v2/C7-WRITE-MANIFESTS.json`
- C7 closure record: `docs/specs/2026-08-28-agent-capability-platform-v2/C7-CLOSURE.json`
- C7 gate historical claim: PASS
  `build.noindex/agent-capability-v2/c5e1015f948cf1fd12ee3bc7dbea2d297f3e0ab6/c7-domain-waves/summary.json`
- 上述 C7 summary 当前不在本 checkout；不能把该历史声明当作当前
  `0bacc9ab...` source 的可复验 evidence。
- Five domain registration waves plus model-media publish exactly 26 packages and 137 capabilities.
- Focused domain tests: PASS; Fresh-v4 root tests: PASS; production host tests: PASS.
- Windows startup smoke: PASS for absent and pre-created empty roots; canonical `/api/capabilities` returned 137 entries.
- Workspace `cargo test` remains intentionally deferred to `C8-WIN-PRE`.
- macOS and Linux native verification remains `pending_native_verification`.
- Known forward work: 在最终 clean source 上重新冻结/对账 C8 tuple，完成真实
  provider/credential/sidecar lifecycle 验证，并通过 global legacy residual
  gate；不能把已有 provider factory 的存在解释为 live provider 已验证。

## Next Directly Executable Batch

1. 在最终 clean source `0bacc9ab...`（及其后续 status-only 文档提交完成后）
   重新生成并对账 `PlatformValidationManifest`/runtime tuple；不要直接从历史
   C7 candidate `c5e1015f...` 继承 tuple。
2. 在新的 frozen tuple 上运行 Windows whole-candidate preflight，包括 workspace
   Cargo validation、UI check/build、七模板/resource coverage、
   lifecycle/fault 与 package checks。
3. Keep macOS/Linux native rows pending until the Windows pre-candidate exits at HP-1.

## Blocking Status

- External validation blockers are recorded below: the Windows Codex sidecar
  artifact and four required native hosts are still unavailable in this
  workspace. They block full C8/native release evidence, but do not block the
  local implementation and targeted checks.
- Codex sibling checkout 与冻结对象当前不可本地复核；这不改变 release input
  的 pinned commit 要求，但会阻塞需要真实 sidecar/source 的 native/lifecycle
  证据。
- Repository-wide React/Arco typecheck baseline debt is recorded; focused UI tests and production build pass.

## 恢复更新（2026-08-31）

本轮从提交 `149fd8923a2feef8b2fbdd6ec59d04a8dbaccca1` 的未提交工作树继续，
没有回退或覆盖用户已有改动。当前工作树仍为 dirty，不能产生有效的 C8 native
candidate evidence。

已完成的实现闭合：

- Fresh-v4 provider/model/connection/capability、client preference、system settings
  使用同一个 canonical SQLite pool；installation token 直接读写
  `installation_auth`，不访问 legacy `instance_access_token`。
- Fresh-v4 canonical host 已挂载 auth、system/provider、AgentPreset/Binding、
  AgentSession、Remote REST 与 health 路由；Remote REST 提供
  `/api/remote/open`、`/api/remote/turn`、`/api/remote/observe`、
  `/api/remote/cancel`。
- Remote token rotate/revoke 与 REST/MCP Remote ingress 共用 D-026
  `RemoteAuthAdmissionFence`；旧 token 在 mutation commit 后的新 admission 返回
  `REMOTE_AUTH_REQUIRED`，不会先读取 Binding/Session。
- `AgentSessionStore::cancel_active_turn` 在同一 SQLite transaction 内选择当前
  `active_turn_id` 并提交 `turn/cancelled`，消除了 HTTP 层先读后取消的 TOCTOU；
  initial input 作为 `session/opening` 的持久 provenance，并在 Runtime ready 后
  由平台尝试首 Turn admission。
- Remote Runtime admission 现在有明确的生命周期边界：调度最多等待 5 秒，
  整个 post-commit admission 最多 35 秒，失败事实写入最多等待 10 秒；
  `runtime/bound + session/ready` 在同一 SessionStore transaction 内提交，并在
  transaction 内要求 Session 仍是 `opening`。取消、panic、Host shutdown 与
  迟到 ACK 都有单飞/RAII 清理路径，`observe/turn` 看到遗留 `opening` 时会重新触发
  同一 coordinator，不会把失败的 Session 当作 ready 执行。另修复了幂等
  `open` 在 Session 已处于 `running` 时被错误投影为 `opening` 的问题；现在
  `running` 明确投影为 `ready`，未知状态 fail-closed。
- RemoteBinding 删除不再通过外键静默改写既有 Session provenance；既有 Session
  保留冻结的 RemoteBinding ID/version，删除只阻止后续 open。
- OpenAI Chat 与 Gemini raw SSE/JSON decoder 已覆盖真实嵌套 text、usage、finish
  与 tool shape；Vertex 继续 typed unsupported，不伪装成可执行 route。
- C8 gate 已将 dirty worktree 设为 fatal preflight，native checks 会被跳过；
  residual scan 读取 triad 与 domain deletion manifests 并区分 allowlisted/
  blocking/unclassified residual，但不会用 allowlist 把非零 global residual 假装为
  pass。`c8-merge` 与 fail-closed `c9-hard-delete` 入口已加入。

本轮关键验证：

- `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`
  通过，generated contract artifacts 已按当前 schema digest 重生成。
- AgentSession、AgentPlatform、Auth、Control Plane、Public Remote、Chat broker、
  Model invoke 与 Fresh-v4 canonical host 定向测试通过。
- Remote REST 状态投影回归测试通过（2 项）：覆盖 `running -> ready`、
  `open_failed/failed` 终态和未知状态 fail-closed。
- Remote 定向 E2E 通过（1 项，约 0.4 秒）：无 sidecar 时在 2 秒窗口内进入
  `open_failed`，并拒绝后续 `turn`。
- `bun run gate:agent-v2 -- --self-test` 通过。
- `bun run gate:agent-v2 -- c8-win-pre` 正确阻断：dirty worktree 与 global legacy
  residual 非零；当前不能将旧 C8 evidence 视为候选证据。

当前真实 blocker：

1. C8-WIN-PRE 仍未闭合：必须先在 clean commit 上重跑；global legacy residual
   当前为 1046（714 blocking、26 contract-allowed、306 deferred-to-C9），
   不能进入 HP-1。主要 blocking consumer 集中在
   `nomifun-conversation/src/service_test.rs`、`nomifun-app/src/router/state.rs`、
   `nomifun-app/src/services.rs`、`nomifun-ai-agent/src/factory/nomi.rs`、
   Gateway capability files 和旧 API/Registry composition；不能全部标成 C9
   deferred。
2. C8-MERGE/C9 尚无五格 native evidence、D-027 final drain/zero proof 或 C9
   hard-delete manifest；`c9-hard-delete` 按设计 fail-closed。
3. Fresh-v4 production runtime 尚未获得可由发布 manifest 验证的实际 Codex
   sidecar artifact/path；Remote open 已接入单飞 Runtime coordinator，首次响应
   仍按两事务语义返回 `opening`，随后在 artifact 缺失、启动/握手/open ACK/
   binding 失败时追加 `session/open-failed`。在 SQLite 可写且 Host/Runtime 正常的
   条件下不会无限停留；但若 SQLite pool 在失败事实写入期间持续不可用，当前
   实现仍可能出现条件性永久 `opening`，因为任何实现都无法凭空提交 durable
   Event。该情况会被记录，并在后续 `observe/turn` 或进程重启 recovery 时再次
   收敛；这不是可用 mock 或无界重试消除的测试问题。Runtime hello/open 内层上限为
   10 秒，整个 admission 上限为 35 秒。
4. Fresh-v4 host 尚未迁移所有旧 UI/domain API；未挂载的旧入口不能视为 v4
    产品功能已完成。
5. Web host 的 robot advertise 在 Fresh-v4 明确 disabled，因为当前 Web host
    没有 robot routes；不能发布空 endpoint。
6. 本轮曾发现一个真实的共享编译阻塞：`ActionExecutor::new` 的调用方仍传入已
   删除的旧 `agent_type` 参数；已在 `router/state.rs` 修复，并清理对应测试导入，
   不再阻塞当前定向编译。
7. 之前 C8 预检日志中的 `chat_broker_host.rs: ChatTask` 编译错误属于旧源快照；
   当前源文件已刷新，`router::agent_platform_host` 定向测试 10/10 通过。C8
   预检仍不能据此视为通过，因为它还被 dirty worktree 和全局 residual 阻断。

## 继续更新（2026-08-31）

- Remote `open` 的结论是有条件的：在 SQLite 可写、Host/Runtime 调度正常时，
  调度上限为 5 秒，整个 Runtime admission 上限为 35 秒，失败事实写入上限为
  10 秒，Session 会从 `opening` 收敛到 `ready` 或 `open_failed`。因此普通运行
  路径不会永久停在 `opening`。
- 若 SQLite 在写入 `session/open-failed` 期间持续不可用，任何实现都无法提交
  durable 终态，Session 仍可能条件性停在 `opening`。这属于存储故障边界，不
  通过 mock、伪造 ready 或无界重试掩盖。现在 admission 无法 settle 时，Remote
  HTTP 错误会保留 `agent_session_id`、cursor 和 `host_restart_reconcile` 恢复
 线索，避免客户端丢失已提交 Session。
- `knowledge.autogen` 的 `overwrite_readme` 已改为严格布尔解析；错误类型不再
  静默退化为 `false`。Wave 1 focused tests 3/3 通过。
- 本轮 focused 验证通过：Remote REST 状态单测 2/2、Remote REST E2E 1/1、
  Wave 1 3/3、Fresh-v4 production host 10/10；相关 `cargo fmt --check` 及
  `git diff --check` 通过。
- 已停止的长时测试不再重复执行：`bootstrap::canonical_host` 两次超过一小时的
  harness/lifecycle 调查、历史 `STATUS_ACCESS_VIOLATION` 和 `spawnSync ENOBUFS`
  均保留原始证据，不作为当前产品行为结论。后续只有在代码或环境前置条件发生
  明确变化时才重新安排。

## 需要手动配合的验证项

以下项目不是普通 Rust 编译失败，继续重复运行不会产生新信息：

1. **Windows Codex sidecar runtime**
   - 需要提供与 `runtime_release_digest =
     c4075b2f7c118fa5eeeb6fc4a0b21cf940d5af6a8acc080e1c8721a8a738a380`
     对应的 `windows_desktop_x64` sidecar 可执行文件路径。
   - 该可执行文件的预期 SHA-256 是
     `36f175f56e065560749fcc16caffbe06639eece66e19b655ea9104052d85cab4`；
     需要确认它与 release input 中
     `runtime/windows/x64/nomifun-codex-runtime.exe` 的 manifest digest 一致。
   - 需要提供与 sidecar 配套的 hello metadata 文件，并确认它能响应
     fork-owned `runtime/hello`、八个 stable RPC 及 `runtime/session/dispose`。
   - 当前仓库只有 release/协议输入和 Rust supervisor，没有随仓库提供的实际
     sidecar binary；因此不能在本机安全地把 Remote Session 从 `opening` 推进
     到 `ready`，也不能验证 `initial_input → turn → provider → observe` 的真实链。
   - 建议手动提供：可执行文件绝对路径、`Get-FileHash -Algorithm SHA256`
     输出、hello metadata 校验结果、sidecar 构建/来源说明，以及一次
     `runtime/hello → create → start_turn → session_dispose` smoke 结果。
     Provider credential 只需在本机的 Fresh-v4 设置中配置并报告成功/失败代码，
     不要把 secret 粘贴到任务记录。路径不会写入 contract digest。

2. **其它 native cells**
    - `macos_desktop_arm64`、`macos_desktop_x64`、`linux_desktop_x64`、
      `linux_headless_x64` 必须在对应真实主机执行，Windows/cross-compile/VM
      不能代验。当前状态保持 `pending_native_verification`。
    - 当前 Gate CLI 已提供独立 dispatch：在对应真实主机分别执行
      `bun scripts/gate-agent-v2.mjs -- c8-ma --evidence <PlatformCellEvidence.json>`、
      `bun scripts/gate-agent-v2.mjs -- c8-mx --evidence <PlatformCellEvidence.json>`、
      `bun scripts/gate-agent-v2.mjs -- c8-ld --evidence <PlatformCellEvidence.json>`、
      `bun scripts/gate-agent-v2.mjs -- c8-lh --evidence <PlatformCellEvidence.json>`，
      或使用通用的
      `bun scripts/gate-agent-v2.mjs -- c8-native --cell <cell_id> --evidence <PlatformCellEvidence.json>`。
      证据文件必须携带同一 candidate tuple、native host fingerprint、artifact digest
      和真实命令结果；缺失、dirty checkout、跨编译/VM/模拟环境或 tuple 不一致都会
      fail-closed，不能把 cross-compile 当作 native PASS。

3. **已停止自动重试的测试**
    - `cargo test --locked -p nomifun-app --lib bootstrap::canonical_host
      -- --test-threads=1` 曾在 2026-08-31 两次运行超过 1 小时并留下测试进程；
      该进程已由本任务清理。它被记录为 test-harness/lifecycle cleanup
      investigation，未作为 pass 或 fail 证据，也不会再次盲目重跑。
    - 历史 Windows workspace run 在
      `d6391775b13f076cb10c7e03351692fa532888a8` 的
      `c8-win-pre/commands/workspace_cargo_test.stderr.log` 记录了
      `STATUS_ACCESS_VIOLATION (0xc0000005)`；另一次
      `f8e2de0a0bcc6c876b95cc5af41f824081da714b` 因 Gate 的
      `spawnSync ENOBUFS` 失败。二者都不是当前产品行为结论，不再重复长跑。
    - Remote/Session/Runtime 的短 focused checks 已通过：
      `cargo test --locked -p nomifun-app --lib router::remote_rest::tests
      -- --test-threads=1`（5 passed）以及
      `cargo test --locked -p nomifun-app --test remote_rest_e2e
      -- --test-threads=1`（1 passed，约 0.4 秒）；后者断言无 sidecar
      时在 2 秒窗口内进入 `open_failed`，并拒绝后续 `turn`。
    - Channel 旧 `agent.select` 测试已改为验证 retired action 不会修改 Session；
      `cargo test --locked -p nomifun-channel --test session_action_integration
      -- --test-threads=1`：13 passed。
   - 本轮新增的 Runtime admission atomic boundary test、AgentSession 14 项、
     AgentPlatform 11 项、Codex Runtime 31 项、domain support 7 项，以及
     `chat_minimal_runs_the_formal_final_stack`（单测约 6.3 秒）均通过。
   - 曾出现的 1 次 `chat_minimal` 失败已定位为测试暴露的真实 resume 回归：
     全局状态校验错误拒绝了 `ready` Session 的新 Runtime binding；已改为只在
     Remote opening admission 上要求 `opening`，修复后完整 `chat_minimal` 2 项通过。
     这不是外部测试障碍，也未通过 mock 或跳过逻辑掩盖。

下一阶段顺序固定为：取得 Windows sidecar artifact/hello metadata 后执行一次
真实 `open → ready → initial turn → observe → cancel → dispose`；同时完成剩余
Fresh-v4 direct consumers 与 runtime/package wiring；在 clean candidate 重跑
C8-WIN-PRE；取得真实 macOS/Linux native cells 后执行 C8-MERGE 与 D-027 final
drain；只有 C8-MERGE 和 exact-zero 通过后才进入 C9 physical deletion。C9 未开始。

## 网络中断恢复后的更新（2026-08-31）

- Fresh-v4 canonical host 现在同时挂载 Remote REST 与 canonical `/mcp`；
  MCP 只公开 `open/turn/observe/cancel` 四个操作，并直接使用
  `AgentPlatform/AgentSession` 与注入的 Runtime admission port。旧
  `/mcp-agent`、`/v1`、`profile/domains`、Remote Registry dispatch、browser
  scope 与对应 legacy smoke 已物理删除。
- Remote Runtime 普通路径有明确上限：调度 5 秒、完整 admission 35 秒、
  `session/open-failed` 写入 10 秒。SQLite 持续不可写时会记录 recovery
  blocker，后续请求保留 `agent_session_id + cursor + host_restart_reconcile`，
  不重复启动 sidecar。
- Wave 2/3/4 已从 synthetic deterministic receipt 切换到 typed host port；
  默认未接线时 fail-closed，定向测试分别为 5/5、7/7、10/10。
- Conversation relay 已删除 relay-owned completion authority；failover
  观察需匹配 durable accepted receipt，相关定向测试 40/40 通过。
- `NomiBuildExtra.backend` 已从运行配置投影中移除；其余字段仍被未迁移的
  Nomi factory/Conversation transitional path 使用，因此没有做不安全的类型删除或
  兼容 alias。API types 545/545、Nomi factory focused tests 37/37 通过。
- Remote E2E 2/2 通过：缺少 sidecar 时 REST 在约 0.4 秒进入 `open_failed`；
  MCP 完成 initialize、exact four-tool inventory、真实 `open -> observe`。
- 此前 bounded `c8-win-pre` 只执行 dirty preflight/static residual，没有运行
  workspace tests：`total=1046`、`blocking=714`、`contract-allowed=26`、
  `deferred-to-C9=306`。
- canonical MCP/Remote REST selector query 已补上 fail-closed 回归：
  MCP query（例如 `?profile=agent`）在 transport session 分配前返回
  `REMOTE_INVALID_REQUEST`；REST 四操作只允许 observe 的
  `agent_session_id/after_seq/limit`，`profile/domains` 及其他未声明 query
  在 handler 前返回同一 canonical code。Public transport 定向测试 14/14，
  Remote REST 单测 5/5。

### 已停止自动重试的测试障碍

- Windows 全 workspace `cargo fmt --all -- --check` 因路径长度错误 206 失败；
  受影响 package 的定向 rustfmt 已通过。
- Channel 全 feature lib tests 曾在 150 秒超时；只保留 parser/plugin focused tests。
- `nomifun-app` 的 `bootstrap::canonical_host` 单测曾两次超过 1 小时并遗留进程；
  不再盲目重跑。
- 历史 workspace `STATUS_ACCESS_VIOLATION (0xc0000005)` 与 Gate
  `spawnSync ENOBUFS` 只作为 harness evidence，不作为产品行为结论。

### 需要用户提供或手动执行

1. Windows `windows_desktop_x64` Codex sidecar 与 hello metadata。当前
   `runtime_release_digest` 为
   `c4075b2f7c118fa5eeeb6fc4a0b21cf940d5af6a8acc080e1c8721a8a738a380`；
   提供 sidecar 绝对路径、`Get-FileHash -Algorithm SHA256` 输出和 hello metadata
   后，执行一次真实 `open -> ready -> initial turn -> observe -> cancel -> dispose`。
2. 真实 macOS arm64/x64、Linux Desktop x64、Linux Headless x64 主机仍为
   `pending_native_verification`；Windows/cross-compile/VM 不能代验。

## 网络中断恢复后的继续更新（2026-08-31）

- canonical MCP 与 Remote REST 已在 handler 前拒绝旧 selector query：
  `/mcp?profile=agent`、`/mcp?domains=...` 以及 REST 非 observe 的任意 query
  都返回 `REMOTE_INVALID_REQUEST`；observe 只接受
  `agent_session_id/after_seq/limit`。
- REST/MCP 的 `binding_id`、`idempotency_key`、`initial_input`/`input` 与
  `observe.limit` 输入校验已统一：空白标识、非法/超限 JSON 和零 limit 均在
  平台调用前返回 `REMOTE_INVALID_REQUEST`。
- Fresh-v4 host 已恢复 `NOMIFUN_ACCESS_TOKEN` headless seed：启动时只将环境值
  的 SHA-256 verifier 写入 `installation_auth`，同一 token 不重复写；持久化失败
  会阻止继续发布一个未认证的 Host。新增 seed 单测通过。
- `nomicore tools`、`nomicore call` 及 `/v1/tools/*` generic Registry CLI 入口已
  删除，替换为显式 `nomicore remote open/turn/observe/cancel`；CLI 不再读取
  `Registry::global()` 或推断最近 Session。
- Gateway 不再发布 `Surface::Remote`、`CallerCtx.remote` 或 Remote generic
  conversation/execution 创建旁路；Remote 产品流只经 Fresh-v4
  `AgentPlatform/AgentSession`。Browser Gateway 中仅保留测试期的旧 attachment
  lifecycle helper，生产 API 不发布第二个 Remote 入口。
- 当前用户文档、架构说明、CLI 配置说明、Remote 对接示例和
  `drive-nomifun` skill 已统一为 `/mcp` + `/api/remote/*` 四操作；历史 deletion
  manifests 仍原样保留，作为删除证据，不作为当前生产入口。

本轮定向验证：

- `nomifun-public` library：14/14；
- `nomifun-app` Remote REST 单测：5/5；
- Fresh-v4 Remote E2E：2/2（包含 selector query fail-closed；本机无 sidecar 时
  `open` 在约 0.4 秒收敛到 `open_failed`）；
- Fresh-v4 installation token seed 单测：1/1；
- `nomifun-gateway` library：123/123；
- Gateway browser-use lifecycle focused tests：36/36；
- Gateway browser-use `parallel_registry` integration：3/3；
- Gateway computer-use focused tests：3/3；
- `cargo check --locked -p nomifun-app --bin nomicore`：通过；
- `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`：
  通过；`cargo fmt` 受影响 package 定向检查与 `git diff --check`：通过。

当前 Gate 证据：

- 上一轮 `bun run gate:agent-v2 -- c8-win-pre` 结果为 fail-closed：
  `1078` residual（`748` blocking、`26` contract-allowed、`304`
  deferred-to-C9），同时因 dirty worktree 跳过所有 native/workspace checks；
  本轮最新数字见下方恢复更新。
- 主要 blocking 仍是未迁移的 `ConversationService`、`AppServices`、
  `GatewayDeps`、Nomi factory/runtime 及其 direct consumers；没有把这些项目改成
  allowlist 或用 mock/bypass 掩盖。
- Windows Codex sidecar、macOS/Linux 四个真实 native cell 仍未提供，因此无法
  验证 `open → ready → turn → observe → cancel → dispose` 的真实 Runtime 路径。

## 网络中断恢复后的验证更新（2026-08-31）

- Conversation relay 的真实 recovery journal 测试已完成迁移，删除
  `with_test_legacy_unjournaled_artifacts` 测试旁路；StreamRelay 定向测试
  `111/111` 通过。
- AgentExecution 的 session adapter 与 IDMM 的 typed session port 定向验证通过：
  AgentExecution `86/86`、IDMM probe `46/46`；两者均只委托现有 canonical
  authority，不创建第二套 Session/Runtime 事实源。
- Gateway browser-use feature 的旧 Remote authority 测试调用已删除或改为
  signed-child/Hub 语义；browser-use 全库 `164/164`、browser registry
  `28/28` 通过。共享 Host cleanup 测试保留了真实 sibling lane，验证失败时
  pending owner 不会被错误报告为已清理。
- 本轮定向编译通过：Conversation、AgentExecution、IDMM、Gateway，以及
  `nomifun-app` 默认和 `browser-use,computer-use` binary check。未运行
  workspace 全量测试，也未重跑已确认会长时间挂起的 lifecycle harness。
- 最新 `bun run gate:agent-v2 -- c8-win-pre` 仍按设计 fail-closed：工作树
  未 clean，residual `1065`，其中 `735` blocking、`26` contract-allowed、
  `304` deferred-to-C9；相较上一轮 `1078/748` 有收口，但尚不能形成 C8
  native candidate。

### 本轮新增的跨 crate 阻塞

1. App composition 仍在 `router/routes.rs` 手工构造 `GatewayDeps`，在
   `router/state.rs` 构造多个 `ConversationService`，并在 `services.rs`
   构造 `AgentFactoryDeps`/旧 Runtime registry。当前
   `nomifun-agent-platform` 只公开 Session command/query/delete 与 Codex
   runtime port，尚无能覆盖这些旧业务服务的 canonical host port；直接删除
   会造成真实编译断裂，新增本地 adapter 则会制造第二 authority。
2. Gateway capability handler 仍依赖 `GatewayDeps` 的业务字段；Gateway
   Cargo 未接入 `nomifun-agent-platform`，且当前 manifest 将 Cargo/中央
   组合文件保留给主线 owner。不能靠重命名、allowlist 或 mock 关闭该残留。
3. Channel/IDMM/AgentExecution 已收窄为 typed delegator，但完整切换仍需
   `nomifun-agent-platform` 的统一 host port 和对应 Cargo DAG 调整。

这些项目已记录为需要下一批中央接口与组合迁移的代码阻塞，不再重复执行
同一失败测试。

## C8 typed consumer 与 Gateway facade 收口（2026-09-01）

本轮继续保留现有 dirty worktree，没有 reset、覆盖、提交或 push。完成的生产代码
收口如下：

- `nomifun-agent-execution` 已公开 `AgentExecutionSessionPort`；生产 engine config
  不再接收 `ConversationService`/`AgentRuntimeRegistry`，app 只挂载一个无状态纯委托
  transitional adapter，不增加第二套 Session authority。
- Cron、Requirement/AutoWork、Companion、IDMM 与 Channel 已分别改为窄 typed
  Session port。各 adapter 只委托当前 Conversation owner 和 Runtime registry，
  不缓存事实、不重试、不 fallback、不伪造成功。
- Gateway Computer、Browser、Channel、Companion、System 与 Agent capability handler
  已改为领域窄依赖 view；完整 `GatewayDeps` 只在 capability adapter 边界投影一次，
  handler 不再直接持有整包业务服务。
- Gateway `computer_registry` 改为共享 `Arc<ComputerRegistry>`，仍保持单桌面、
  串行执行和 unavailable fail-closed 语义。

本轮验证：

- `nomifun-agent-execution`：86/86。
- `nomifun-cron` library：191/191。
- `nomifun-idmm` probe focused：46/46。
- Channel `session_action_integration`：13/13；`stream_relay_test`：21/21；
  Channel 全测试目标 `--no-run` 编译通过。
- Gateway `browser-use + computer-use` library：167/167。
- `cargo check --locked -p nomifun-app --features computer-use` 通过；
  受影响 Rust 文件定向 rustfmt 与 `git diff --check` 通过。

最新 fail-fast：

```text
bun run gate:agent-v2 -- c8-win-pre
```

仍按设计 fail-closed：

- worktree：dirty
- residual：984
- blocking：662
- contract-allowed：26
- deferred-to-C9：296

相较本轮开始的 `1065/735/26/304`，减少 81 条 residual 和 73 条 blocking。
当前最大 blocking 仍集中在旧 Conversation 测试/实现、`router/state.rs`、
`services.rs`、Nomi factory/runtime、剩余 Gateway 领域 facade 与旧 API DTO。
Windows Codex sidecar及四个非 Windows native cell 仍是外部证据阻塞，不重复自动
测试；在 clean candidate、exact-zero 与真实 native evidence 之前仍不能宣称
C8/HP-1/发布完成。

## C8 residual 与生产 Host 审计（2026-09-01）

本轮在不放宽扫描规则、不扩大 D-004 allowlist 的前提下完成了生产组合图的静态
收口：

- 2026-09-01 dirty preflight 的 C8 source residual 为 `561`：
  `0 blocking`、`26 contract-allowed`、`535 deferred-to-C9`。
  状态文档本身也属于 source scan，因此总数高于代码收口前的临时统计；
  `contract-allowed` 仍是原有 D-004 exact allowlist；
  其余项目均有逐路径、逐符号的 C9 删除登记。
- `AppServices`、旧 router builder 与 legacy server start 已隔离到显式
  compatibility surface；Embedded、Desktop 与 Web 生产入口只组合
  `FreshV4Application`。
- `GatewayDeps` 已收窄为 compatibility capability host，业务 handler 只接收领域
  窄依赖 view；AgentExecution、Cron、Requirement/AutoWork、Companion、IDMM 与
  Channel 使用 typed Session port，不再把完整 Conversation/Runtime service bag
  作为生产构造参数。
- Agent Settings 的 Coding workspace 使用逻辑资源 ID `workspace.default`；
  Host 的真实 `work_dir` 只进入 `typed_parameters.workspace_root`。历史草稿在
  Preview/Save/Test 前补齐 Host-resolved 参数，不从 `resource_id` 猜本地路径。

当前 canonical tuple：

- confirmed decision contract：
  `b45efce157933d72671a9158ff87d4a84b5b288bc8ec6bf3688226497c6e0cf5`
- canonical schema：
  `e28723d7fc524cfdd351c6fc8cc17b8a48d8fd1f5be16a7aebd395ce669f98ff`
- platform validation：
  `fa3cd9c542bab988afc366d512c279e34f33bef07bf2546a78094845f81bb948`
- runtime release：
  `c4075b2f7c118fa5eeeb6fc4a0b21cf940d5af6a8acc080e1c8721a8a738a380`
- Cargo.lock：
  `26e121277eb2054fc43f80dbfc72b7a8ee4fc2cebcc8294752217944989dfb14`

已完成的定向验证：

- `cargo check --locked -p nomifun-app -p nomifun-web -p nomifun-desktop`
- `health`、`webui_lan_smoke`、`content_e2e_suite` representative targets
  `--no-run`
- Wave 4 typed unavailable：1/1
- Agent Settings focused tests：18/18
- `bun run check:i18n`：7078 keys / 33 modules
- `cargo fmt --package nomifun-app -- --check`
- 相关 `git diff --check`
- `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`
- C8 Gate self-test

全量 TypeScript typecheck 仍失败于既有 React/Arco 与 implicit-any 基线；focused
workspace helper 没有新增类型错误。本轮不重复执行同一已知失败。

### Fresh-v4 Domain Wave 的真实功能状态

静态 residual 为零不等于业务 action 已可执行。当前生产组合的真实状态如下：

- Wave 1：Fresh-v4 已挂载真实 `web.fetch` 与基于 Kernel PluginState 的有界
  project/companion memory mutation owner；Research search、Knowledge、
  memory read/citation/recall 与 Skill invoke 仍 typed fail-closed。
- Wave 2：`fs.read/fs.search/fs.write/fs.patch/fs.delete/fs.snapshot` 以及
  `vcs.status/vcs.diff/vcs.stage/vcs.commit` 已通过 owner-scoped workspace
  binding 调用真实 `FileService`、`SnapshotService` 与 Git owner；`process.exec`、
  `vcs.push`、SSH、MCP connector、Browser、Computer 及其余 workspace action
  尚未接入 canonical owner，调用时 typed unavailable。
- Wave 3：19 个 Creation/Workshop/Office/MiniApp action 仍使用 metadata-only
  unconfigured host，调用时 typed fail-closed。
- Wave 4：Fresh-v4 显式挂载应用 typed host，但当前没有 v4-native
  Channel/Companion/Customer/Robot owner，因此明确返回 typed unavailable，不重试、
  不 fallback、不制造成功。
- Wave 5：AgentExecution、schedule.store 与 Requirement action 仍使用
  unconfigured host；Remote REST/MCP 的 `open/turn/observe/cancel` 是独立挂载的
  canonical transport，不能反向证明其它 Wave 5 action 已完成。

D-028 只定义 Coding、Browser、Computer 的平台 availability，不能自动豁免上述
业务域。根据 C8 contract，七模板和 all-scene executable conformance 仍需真实
业务 owner 与原生 evidence；因此即使 clean Gate 的现有结构/测试检查通过，也不得
把该结果解释为完整功能发布或正式 HP-1。

### Clean checkpoint 与 macOS 工程验证边界

2026-09-01 已在 clean SHA
`4de100692983cb0e4e81091d60456dc26a9d8e69` 上执行一次完整
`bun run gate:agent-v2 -- c8-win-pre`。该轮约 25 分钟完成，没有 workspace
lifecycle 挂起、`STATUS_ACCESS_VIOLATION` 或 `ENOBUFS`：

- PASS：contract validation、Domain registration、Fresh-v4 root、production host、
  production broker、UI build、Windows startup smoke、Windows package contract。
- `baseline_fail`：repository-wide UI typecheck，仍是既有 React/Arco/implicit-any
  基线；生产 UI build 通过。
- FAIL：C7 manifest 的四个 confirmed input digest 尚停留在旧生成物。
- FAIL：workspace `cargo test` 在
  `nomi-tools::file_cache::normalize_above_root_is_clamped` 的 Windows 路径表示断言
  上停止；该 crate 在失败前为 `301 passed / 1 failed`。

随后已做两项定向修复，没有重复完整 C8 Gate：

- C7 manifest 同步当前 schema、contract ledger、deletion manifest set 与 runtime
  release payload digest；`bun run gate:agent-v2 -- c7-domain-waves` 通过。
- `nomi-tools` 测试改为验证 `/../b` 与 `/b` 的归一化等价性，不再硬编码 POSIX
  路径表示；`cargo test --locked -p nomi-tools --lib -- --test-threads=1`
  为 `302 passed / 0 failed`。
- `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`、
  `cargo fmt --package nomi-tools -- --check` 与 `git diff --check` 通过。

上述修复生成新的 source SHA，因此 `4de10069` 的完整 Gate evidence 不能提升为
最终 pass；本轮不重复运行同一全量测试风暴。后续如需正式 HP-1，必须在最终 clean
tuple 上由 validation coordinator 安排一次新的完整 Windows Gate。

Windows Codex sidecar 仍缺失，预期 SHA-256 为
`36f175f56e065560749fcc16caffbe06639eece66e19b655ea9104052d85cab4`。在提供
sidecar 与 hello metadata 前，真实
`open -> ready -> turn -> observe -> cancel -> dispose` 只能列为手动阻塞项。

修复后的 clean checkpoint 可以作为 macOS arm64 的工程逻辑验证候选，用于编译、启动、
路径/权限、bundle、进程树与 target-specific adapter 补齐；它不是正式 HP-1。
正式 HP-1 仍要求 Windows 全场景功能、sidecar/native Gate 和 clean remote SHA
全部闭合。

## 本轮继续实施记录（2026-09-01，macOS arm64 Host）

本轮从远端候选 `0f9dfd63d9c6a1630620096e088d3ffcde77fc81` 继续，先核对了
branch、HEAD、origin、worktree、Git identity 与远端 SHA。当前已形成并普通推送的
实现提交为：

- branch：`rf/agent-capability-platform-v2`
- current/last verified remote SHA：
  `3b7236f13c3120b3fabfdab2f43d56cf1795b28b`
- origin：`git@github.com:nomifun/nomifun-desktop.git`
- native host：`Darwin` / `arm64`，`sysctl.proc_translated=0`，
  `rustc host=aarch64-apple-darwin`
- `../codex`、`../deepseek-harness` 与要求的 MACOS arm64 handoff 文件在本机均
  不存在；没有偷偷改用其他 checkout。

本提交 `feat(agent): harden domain host boundaries and macOS preflight` 已完成：

1. 五个 Domain Wave 增加逐 action typed operation、capability/action identity
   校验、资源 owner/operation/cardinality 检查、canonical fail-closed error
   传播和可组合 host-port seam；Wave 5 保持 Remote 四操作 transport 与 action
   host 严格分离，Wave 1/5 只透传 Kernel 授权的 namespace state handle。
2. Fresh-v4 中央组合增加显式 `AgentDomainHostPorts` 五波次注入 seam；未有真实
   v4 owner 的波次仍使用 fail-closed port，未以 composed metadata 或测试返回
   冒充业务 owner。
3. Conversation relay 删除 duplicate runtime completion projection 和
   failover teardown waiting tip；保留 service-owned durable completion/receipt。
4. Codex Runtime process pin 拒绝 symlink、非 regular file、不可执行或
   group/world-writable sidecar；release input、runtime digest 与当前冻结 cohort
   对齐。
5. macOS 构建脚本要求真实 arm64/x64 sidecar、固定 SHA、Mach-O 架构、权限、
   大小写路径、hello/profile/RPC metadata，并为 Universal app 预留两个 Darwin
   sidecar；缺制品时直接失败，不复制伪制品。新增独立 arm64 native helper，
   不写入或提升 PlatformCellEvidence。

已通过的本轮验证：

- `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`
- `cargo fmt --all -- --check`、`git diff --check`
- 五个 Wave 定向 tests：`6 + 8 + 9 + 16 + 11 = 50 passed`
- `cargo test --locked -p nomifun-conversation --lib -- --test-threads=1`：
  `323 passed`
- `cargo test --locked -p nomifun-codex-runtime --lib -- --test-threads=1`：
  `31 passed`
- `cargo test --locked -p nomi-tools --lib -- --test-threads=1`：
  `311 passed`
- `cargo test --locked -p nomifun-app --lib router::agent_platform_host
  -- --test-threads=1`：`9 passed`
- `cargo check --locked -p nomifun-app -p nomifun-web -p nomifun-desktop`
- `bun run check:i18n`：`7078 keys / 33 modules`
- `bun run build:ui`
- `bun run gate:agent-v2 -- --self-test`
- `bun run gate:agent-v2 -- c7-domain-waves`
- `bun test scripts/validation/check-macos-arm64-native.test.mjs`：`2 passed`
- `bash -n scripts/desktop-build-mac.sh scripts/release-mac.sh`

macOS arm64 native helper 实际结果（当前 source `3b7236f13...`）：

- 当前编译的 `target/debug/nomicore` 可在 absent root 与 pre-created empty root
  启动；`/health` 与 137 条 canonical capability inventory 检查通过，进程树
  清理检查通过。
- `universal-app` 当前仅观察到 `arm64`，尚未生成要求的 `arm64+x86_64`
  Universal app/DMG。
- 缺少真实 macOS arm64 Codex sidecar，固定期望 SHA-256：
  `7863db3a77545eec8966483f26fb5b493aea6e285ac35b5c29d0920342438060`。
- 因 sidecar、hello metadata、真实 binding/token 不存在，尚未执行真实
  `open → ready → initial turn → observe → cancel → dispose`；helper 将该项
  保持 `blocked`，没有生成 PASS evidence。

当前状态仍为 `C8-WIN-PRE/C8-MA pending`，不是 HP-1、C8-MERGE 或 Stable PASS。
继续实施前的真实阻塞为：

1. Windows x64 sidecar 与 hello metadata 缺失（期望 SHA-256：
   `36f175f56e065560749fcc16caffbe06639eece66e19b655ea9104052d85cab4`），因此
   Windows 最终 C8-WIN-PRE 和 HP-1 不能由本机代验；
2. macOS arm64/x64 sidecar、hello metadata、Universal signed package 与真实
   provider/binding 资源缺失；
3. Wave 1、Wave 3、Wave 5 仍没有可由当前 Fresh-v4 schema 验证的完整真实业务
   owner；Wave 4 仍明确没有 v4-native Channel/Companion/Customer/Robot owner。
   这些 action 继续 typed fail-closed，不能改成 mock/synthetic success；
4. repository-wide UI typecheck 仍保留既有 React/Arco/implicit-any baseline，
   但本轮 i18n、生产 UI build 与受影响 focused checks 通过。

下一可执行批次：在取得真实 sidecar/hello 与业务 owner 后，先执行对应
`runtime/hello`、`open → ready → initial turn → observe → cancel → dispose`、
Universal package/path/permission/process checks，再由对应原生 Host 生成
PlatformCellEvidence；在此之前不运行 `c8-ma` 或 `c8-ma` 的 PASS 结论，也不把
本提交的静态/target-specific checks提升为 C8 正式通过。

## 增量实现记录（2026-09-01，Wave 2 workspace/VCS）

在上述提交之后又完成并普通推送：

- `5aa3d9930c8e27dd1af9b50e0229c47aa08c1c97`
  `feat(agent): confine workspace vcs actions to bound roots`
- 当前 local/remote SHA 均为该值，worktree 在本记录开始时 clean。

`Wave2ApplicationHost` 现在除真实 `fs.read/fs.write/fs.delete` 外，增加了真实
owner-backed `fs.search`、`vcs.status`、`vcs.diff` 与 `vcs.stage`：

- 所有操作仍先验证同一 immutable workspace binding、owner 和 operation grant；
- Git 仓库可位于绑定 workspace 的祖先目录，但 status/diff/stage 均按绑定
  workspace 前缀投影和限制，不泄漏或修改 workspace 外文件；
- 搜索行、diff 输出和结果集合有明确大小/数量上限，路径输出保持逻辑相对路径，
  不把本机绝对 repository path 写入结果；
- 非 Git workspace、越界路径、非法输入和 worker 失败均返回 typed error，
  不 fallback 到其他根或成功回显。

新增的 targeted regression：

- `cargo test --locked -p nomifun-app --lib router::agent_wave2_host
  -- --test-threads=1`：`5 passed`
- `cargo check --locked -p nomifun-app`、app rustfmt/check 与 `git diff --check`：
  通过。

该增量（截至 `043c9d5ba`）只关闭 Wave 2 的一部分 workspace/VCS action gap；
`fs.patch`、`fs.snapshot`、`process.exec`、SSH/MCP/Browser/Computer 仍需各自真实 owner
与 lifecycle seam，不能由本实现推断为 Wave 2 或 C8 全部通过。当前 SHA 变化也
使此前以 `3b7236f13...` 为 source 的任何 native/tuple evidence 失效，待最终
候选冻结后重新生成。

## 增量实现记录（2026-09-01，Wave 1 Web Fetch 与 Host 组合）

随后又完成并普通推送（以下为该轮历史记录）：

- `e12f0ede4a7894e23f4b3441a4a60918fe3cbd00`
  `feat(agent): mount partial Fresh-v4 capability owners`
- 当前 local/remote SHA 均为该值。

截至该轮，Fresh-v4 中央 `AgentDomainHostPorts` 为 Wave 1 挂载一个真实的
`HttpFetcher` owner：`web.fetch` 通过既有 SSRF/DNS 校验、重定向/大小限制和
HTML→Markdown 归一化返回真实页面结果；Research search、Knowledge、Memory
和 Skill actions 在没有对应 v4 owner 时仍显式返回 `CAPABILITY_UNAVAILABLE`，
没有把未实现操作伪装成成功。Wave 2 的 workspace/VCS owner 同时继续由同一
显式组合 seam 注入。

本轮中央 Host 回归验证：

- `cargo test --locked -p nomifun-app --lib router::agent_platform_host
  -- --test-threads=1`：`10 passed`
- app rustfmt、`git diff --check` 与 `cargo check --locked -p nomifun-app`：
  通过。

该提交仍是**部分 owner closure**，不是 Wave 1/3/4/5 全量业务 owner，也不改变
以下待办：Fresh-v4 Knowledge/Memory/Skill/Creative/Automation/Channel/
Companion/Customer/Robot 的真实持久化与 effect owner；真实 Codex sidecar、
Universal package、Windows pre candidate 和五格 native evidence。当前
`PlatformValidationManifest` 仍需在最终候选冻结时按新 source SHA 生成/对账；
旧 tuple evidence 不可沿用。

## 增量实现记录（2026-09-01，Wave 1 PluginState memory mutation）

在上述部分 owner closure 之后，已提交并普通推送：

- `3c095d98c` — `feat(agent): persist bounded memory mutations via plugin state`
- 远端分支 `rf/agent-capability-platform-v2` 已核对指向该提交；
  代码提交前的远端 SHA 为 `3303f93ab53de78e85edac6381e0b25c00107179`。

本提交把 Wave 1 的五个**写入/变换 action**接到一个真实的
Kernel `PluginState` CAS owner：

- `memory.project.write`、`memory.project.distill`；
- `memory.companion.write`、`memory.companion.merge`、
  `memory.companion.evolve`。

实现边界保持明确：

1. 状态使用完整四元 identity
   `(package_id,mount_id,scope_key,state_key)`；`scope_key` 由唯一、已通过
   owner/operation 校验的 typed resource 派生为 `resource:<resource_id>`，不会
   回退到 Session scope，也不会跨 project/companion package 或资源串写。
2. 每条 mutation 先 canonicalize request 并保存 request digest；同一资源、
   同一 idempotency key 且同一请求会返回原结果，不同请求返回
   `IDEMPOTENCY_CONFLICT`，不会追加第二条记录。
3. Read/modify/write 只使用 Host `get` + bounded CAS；CAS conflict 只做有限
   次重试。单条 entry、总 PluginState bytes、entry count、revision、state
   format 和 stored JSON shape 都有上限/校验，容量、损坏或未知 format 均
   fail-closed，不隐式迁移或覆盖原状态。
4. 这只是**bounded mutation owner**，不是完整 Memory domain closure：
   `memory.project.read`、`memory.project.citation`、
   `memory.companion.recall`、Knowledge、Research search 和 Skill invoke
   仍按未接入真实 owner 的既有语义返回 typed unavailable；当前不能据此宣称
   Wave 1 全部可执行、七模板/all-scene、C8/HP-1、native evidence 或发布完成。

新增定向回归（均在 macOS arm64 host 执行）：

- `cargo test --locked -p nomifun-app --lib router::agent_platform_host
  -- --test-threads=1`：18 passed；
- 覆盖 Kernel→host→PluginState 的 project/companion variant dispatch、
  resource/package/mount isolation、same-key replay/conflict、并发 CAS、
  bounded state/no-partial-append、损坏 state、unsupported format；
- Fresh-v4 SQLite-backed platform restart/reopen 后的 state restore/replay：
  通过；
- `cargo check --locked -p nomifun-app -p nomifun-web -p nomifun-desktop`、
  `cargo fmt --package nomifun-app -- --check`、`git diff --check`、
  `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`：
  通过。

本次代码变化使此前以 `3303f93ab...` 为 source 的任何 candidate tuple/evidence
失效；最终代码稳定后仍需由中央 owner 更新/重新生成
`candidate_source_sha`、platform validation manifest digest 和 cohort tuple。

## 增量实现记录（2026-09-01，Wave 2 workspace patch/VCS completion）

在上述记录之后，已提交并普通推送：

- `043c9d5ba` — `feat(agent): close bounded workspace patch and vcs owners`
- 当前远端 `rf/agent-capability-platform-v2` 与本地提交一致。

本提交继续只关闭已有真实 Workspace owner 的 bounded action，不扩展到
SSH/MCP/Browser/Computer/process 或 `vcs.push`：

1. `fs.patch` 现在通过 `FileService` 的 typed
   `AgentSessionWorkspaceBinding` 执行多文件、逐 hunk 的内存预验证，再进行
   authority-confined publication；支持严格 JSON patch line union、路径/符号链接/
   文件数/行数/字节数上限、外部变更 precondition、失败回滚和无 clobber 的
   新文件发布。
2. `vcs.commit` 使用真实 Git index/tree/commit API，仅允许提交绑定 workspace
   内的 staged changes；nested workspace 会投影逻辑相对路径，rename 同时校验
   old/new path，损坏或非空仓库的异常 HEAD fail-closed，literal-backslash
   Unix 路径不会被错误解释为目录分隔符。
3. Wave 2 HostContext 透传 Kernel 授权的 namespace-scoped `PluginStateHandle`。
   `fs.write`、`fs.patch`、`fs.delete`、`vcs.stage`、`vcs.commit` 使用按
   capability/resource scope 的 bounded effect journal：同一请求 replay 返回原
   结果、不同 payload 返回 `IDEMPOTENCY_CONFLICT`，started/uncertain 记录禁止
   自动重试；不会使用第二个数据库或 Session authority。
4. `FileService` 的 patch API 保持 `nomifun-file` 既有 public service 边界；
   Windows 原子替换使用 target-specific `windows-sys` API，lockfile 仅增加该
   已存在依赖的 package edge。

定向验证（macOS arm64）：

- `cargo test --locked -p nomifun-agent-domain-wave2 --lib -- --test-threads=1`：
  9 passed；
- `cargo test --locked -p nomifun-file --lib -- --test-threads=1`：200 passed；
- `cargo test --locked -p nomifun-app --lib router::agent_wave2_host
  -- --test-threads=1`：18 passed；
- `cargo check --locked -p nomifun-agent-domain-wave2 -p nomifun-file -p nomifun-app`
  及 `cargo check --locked -p nomifun-app -p nomifun-web -p nomifun-desktop`：
  通过；
- 受影响 package `cargo fmt --check`、`git diff --check`：通过。

该提交仍不代表 Wave 2 全量完成：`fs.snapshot`、`process.exec`、SSH/MCP/
Browser/Computer、`vcs.push` 尚无对应真实 v4 owner。缺少真实 Codex sidecar、
hello metadata、Universal package 和 provider/binding lifecycle 资源的外部阻塞
也仍未改变；不得将本地 target checks 写成 C8-MA/HP-1/native release PASS。

## 增量实现记录（2026-09-01，Wave 2 snapshot owner 与 macOS 工程复验）

随后已提交并普通推送：

- `9493428fb` — `feat(agent): add scoped workspace snapshot owner`
- 远端分支与本地提交已核对一致（状态文档更新本身另形成后续 status-only
  commit）。

本提交把已有 `nomifun-file::SnapshotService` 作为 `fs.snapshot` 的真实
Wave 2 owner，通过 typed workspace binding 提供以下 bounded operation：

- `init`：为当前 AgentSession 初始化或复用同一 workspace snapshot；
- `compare`：返回有上限的 staged/unstaged 相对路径变化；
- `baseline`：读取绑定 workspace 的 baseline 文本，并限制返回大小；
- `dispose`：按 AgentSession 引用释放 snapshot。

实现不创建第二套快照存储：Git workspace 复用真实 repository，非 Git workspace
复用既有临时 snapshot repository；同一 Session 的重复 `init` 不增加引用，
不同 Session 的 snapshot 使用独立 session ownership，未初始化或非 owner
Session 的 `compare/baseline` fail-closed。`SnapshotService::info` 是只读状态
查询，不改变 refcount。

macOS arm64 本轮定向复验：

- 原生环境：`Darwin` / `arm64` / `sysctl.proc_translated=0`；
  `rustc host=aarch64-apple-darwin`；
- `cargo check --locked -p nomifun-app -p nomifun-web -p nomifun-desktop`：
  通过；
- Wave 2 domain：9 passed；
- `nomifun-file` library：202 passed；
- `router::agent_wave2_host`：16 passed；
- `cargo fmt --check`、`git diff --check`：通过；
- i18n：7078 keys / 33 modules，生成物已是最新；
- `bun run build:ui`：通过（Vite 7720 modules；仅有既有 chunk size warning）；
- macOS helper self-test：通过；helper test：2 passed；
- `bun scripts/validation/check-macos-arm64-native.mjs`：按设计 fail-closed，
  不是 C8-MA PASS。启动空 root/预创建空 root、health、137 capability inventory
  和进程清理通过；Universal app 当前仅 `arm64`，缺少 `x86_64`；真实 arm64
  sidecar 缺失，期望 SHA-256 为
  `7863db3a77545eec8966483f26fb5b493aea6e285ac35b5c29d0920342438060`；没有
  endpoint/binding/token/provider/credential，因此
  `open → ready → initial turn → observe → cancel → dispose` 未执行；
- `bash scripts/desktop-build-mac.sh --check-only`：按设计拒绝缺失真实
  arm64 sidecar，没有复制或制造替代制品；
- repository-wide `bun run typecheck` 仍保留既有 React/Arco/implicit-any
  baseline，本轮未将其误报为生产 UI build 失败。

当前仍不能宣称 C8-WIN-PRE、HP-1、C8-MA、Universal release 或 native
PlatformCellEvidence PASS。由于 `9493428fb` 改变了 source，且
`598a63203` 刷新了 release tuple，旧 `candidate_source_sha`/platform
tuple/evidence 均不能沿用；待代码稳定后需由中央 validation owner 重新生成并
对账。

## Canonical release tuple 刷新（2026-09-01）

`043c9d5ba` 为 `nomifun-file` 增加 Windows target-specific
`windows-sys 0.61.2` 依赖后，仓库 `Cargo.lock` 的实际 SHA-256 已从旧的
`b69f75e6...` 变为：

```text
26e121277eb2054fc43f80dbfc72b7a8ee4fc2cebcc8294752217944989dfb14
```

因此先前以 `b69f75e6...` 为输入的 runtime/platform digest tuple 已失效。
本轮没有把旧 tuple 或旧 evidence 改名继续使用，而是运行 canonical
`agent-v2-contract write`，并将以下真实产物同步到
`598a63203`；随后在 `0bacc9ab` 修正了 C8 manifest 对 schema digest 的
显式 field reference：

- runtime release：
  `c4075b2f7c118fa5eeeb6fc4a0b21cf940d5af6a8acc080e1c8721a8a738a380`
- platform validation fixture：
  `fa3cd9c542bab988afc366d512c279e34f33bef07bf2546a78094845f81bb948`
- contract digest ledger：
  `de0e564866d0d0ffc896eeaabc0d9ec629f25884ef5055cf35354f4fd653e8a2`

已同步的机器输入包括 `C7-WRITE-MANIFESTS.json`、
`C8-WIN-PRE-MANIFEST.json`、Gate 常量、Codex runtime vendor input、
runtime frozen digest 与 macOS helper 的 expected release digest。随后
`agent-v2-contract check`、Codex runtime 定向测试（31/31）和 Gate self-test
通过；这些结果仍只是 contract/targeted checks，不是 C8-WIN-PRE、C8-MA 或
HP-1 通过。

保留的两个审计边界：

1. `C7-CLOSURE.json` 引用的 `c5e1015f...` evidence 不在当前 checkout；
   没有创建副本、改名或以其他 SHA 的 evidence 冒充它。
2. `C8-WIN-PRE-MANIFEST.json` 的
   `immutable_inputs.platform_validation_contract` 已改为显式 field
   reference：canonical schema envelope 的
   `payload.platform_validation_contract_digest` 字段必须等于
   `78f264...`。Platform validation payload/fixture 仍独立使用
   `fa3cd9...`；三种 identity 不再混用。Gate 现在同时校验 entry 的 exact
   field set、引用字段和 digest 值。

当前 platform payload 的 `candidate_source_sha` 仍是生成器使用的历史
`7a2ade3c...` 基线；它不能被文档更新或手工替换成当前 HEAD 来伪造自引用闭合。
最终 candidate 冻结时必须由 validation owner 在 clean source checkpoint 上重新
生成 payload/fixture 并同步四字段 tuple；在此之前 native C8 gate 对该 mismatch
保持 fail-closed，旧 native evidence 不可沿用。

## 本轮最终定向复验（2026-09-01）

- 当前 branch 与 origin 均为 `rf/agent-capability-platform-v2`；
  本轮定向 Rust/UI 验证时的 remote SHA：
  `9e2098298d079545f1de0a5cee76212f6acccb9c`；随后只提交了 handoff
  文档，当前 branch/remote HEAD 为 `e43afe44c26db7a68d489ae6724c1852cfd86022`，
  worktree clean。
- 原生环境：`Darwin` / `arm64` / `sysctl.proc_translated=0`，
  `rustc host=aarch64-apple-darwin`。
- contract generator `check`、C7 domain-wave gate、C8 Gate self-test、
  Gate/arm64 helper syntax checks、`cargo fmt --all -- --check` 与
  `git diff --check` 通过。
- Rust 定向复验通过：六个 Domain crate `58` tests、`router::agent_platform_host`
  `18`、`router::agent_wave2_host` `16`、`nomifun-file` `202`、
  `nomifun-codex-runtime` `31`、`nomi-tools` `311`；app/web/desktop
  `cargo check --locked` 通过。上述 Rust 结果之后仅发生文档/Gate reference
  修正，没有改变这些 Rust owner 实现。
- `bun run check:i18n`：`7078` keys / `33` modules；`bun run build:ui`：
  `7720` modules transformed，构建成功。全仓 UI typecheck 的既有
  React/Arco/implicit-any baseline 仍未被误报为通过。
- `check-macos-arm64-native.mjs` 在真实 arm64 Host 上仍按设计
  fail-closed：native host、空 root/预创建空 root、health、`137` capability
  inventory 和进程清理通过；当前 app 仅 `arm64`（缺 `x86_64`），真实 arm64
  sidecar 缺失（期望 SHA-256：
  `7863db3a77545eec8966483f26fb5b493aea6e285ac35b5c29d0920342438060`），
  live lifecycle 未执行。`desktop-build-mac.sh --check-only` 同样因缺真实
  sidecar 拒绝，没有复制或制造替代制品。
- 本轮没有运行 macOS `c8-ma` full Gate、没有生成
  `PlatformCellEvidence` PASS，也没有运行 workspace-wide `cargo test`；
  缺少 sidecar/Universal 双架构包/真实 binding、token、provider 资源及
  其它 native Host，仍是外部阻塞。
- 已生成 Windows 连续验收交接包：
  `MACOS-ARM64-ENGINEERING-HANDOFF.zh.md` /
  `MACOS-ARM64-ENGINEERING-HANDOFF.json`。其状态为
  `ready_for_windows_continuation`，明确标记 C8-MA/HP-2 未通过，不是 native
  PASS evidence。

## macOS 回传后的 Windows 合流（2026-09-02）

远端 `45655e80...` 的 16 个提交已作为合流基线，包含 PluginState memory
mutation owner、workspace patch/VCS owner、snapshot owner、契约 tuple 刷新和
macOS 到 Windows 的验证交接。Windows 侧两个尚未发布的提交在该基线上重放；
冲突按 owner 语义合并，不使用 force-push、mock success 或放宽 Gate。

合流后的 Wave 2 主线同时保留：

- 远端的持久化 effect journal、完整 `fs.snapshot`
  `init/compare/baseline/dispose` 生命周期和 `vcs.commit`；
- Windows 的受监管 `process.exec`、timeout/cancel 后进程树回收、
  `process_session.workspace_root` 强制绑定和 junction escape 拒绝；
- staged/unstaged `vcs.diff`、目录/新文件/删除 `vcs.stage`、Windows Git
  路径逐组件大小写无关投影和 reparse entry 拒绝；
- capability 未声明额外 resource binding 的 fail-closed 校验；
- 非权威 `turn_metrics` token telemetry、唯一 `turn.completed` lifecycle
  authority，以及无调用方 Conversation test-only legacy wrapper 删除；
- C8 production owner coverage：Wave 1～5 owner 未物理闭合时保持失败。

合并后的 canonical 产物由 `agent-v2-contract write` 生成：

- confirmed decision contract：
  `b45efce157933d72671a9158ff87d4a84b5b288bc8ec6bf3688226497c6e0cf5`
- canonical schema：
  `e28723d7fc524cfdd351c6fc8cc17b8a48d8fd1f5be16a7aebd395ce669f98ff`
- contract digest ledger：
  `cf11cc4feb9b0d64d85759f90bd9fed36d1feeea6b3e48bc92bb641f5bfee54b`
- runtime release：
  `7c0c297dd0dd7c11c71cd589965e930ddec0008bebaaf510eabcd0c597358838`
- platform validation：
  `885ad04ecbd798ae5285d956fe25cb6d3426b1f66244a33ac42db87c732687eb`
- `Cargo.lock`：
  `8542e44b505368b7ae19e9ce064c2b9726bba6db5d597495e9899e868637a52c`

历史 C8-WIN-PRE 在 clean `b849e2ac...` 上运行一次：owner coverage 按设计
列出 Wave 1～5 未闭合项；workspace test 停在
`canonical_remote_rest_freezes_binding_and_auth_fence`，单独子进程在无进展后
终止，未盲目重试。该 evidence 仅用于障碍记录，不属于当前合流 SHA，也不构成
C8/HP-1 PASS。

当前真实剩余项仍包括 Wave 1 Knowledge/Memory/Skill 完整 v4 resource owner，
Wave 2 `vcs.push`、SSH、MCP/Connector、Browser、Computer owner，Wave 3/4 的
typed effect/resource/outbox，以及 Wave 5 canonical AgentSession command/query
ServiceKey。Windows sidecar 和其余 native cell 证据仍是外部阻塞；在这些项完成前
不能宣称 C8-WIN-PRE、C8-MA、C8-MERGE 或发布完成。
### Windows targeted validation before rebase（2026-09-02）
- Windows `cargo check --locked -p nomifun-app -p nomifun-web
  -p nomifun-desktop`
- 五个 Wave boundary：`6 + 8 + 9 + 16 + 11 = 50 passed`
- Wave 2 application host：`17 passed`
- Agent Kernel：`12 passed`
- Codex Runtime：`32 passed`
- Conversation：`323 passed`
- token telemetry/read-only UI focused tests：`10 passed`
- macOS helper tests：`2 passed`，Windows `--self-test` PASS
- Agent v2 contract check、C7 Gate、C8 Gate self-test
- `bun run check:i18n`：`7078 keys / 33 modules`
- `bun run build:ui`
- 受影响 package rustfmt 与 `git diff --check`

### 当前真实 owner 阻塞

- Wave 1：已有真实 `web.fetch`、五个 PluginState memory mutation owner，以及
  binding-backed `knowledge.search/read`。仍缺 `web.search`、
  `knowledge.write/autogen/embedding/rerank` 与 `skill.invoke` 的完整 owner；
  `web.fetch` 也尚未闭合 ExternalTransmit idempotency/receipt。
- Wave 3：19 个 action 尚无 action-specific DTO/outcome，Fresh-v4 也没有
  Canvas/Asset/CreationTask/TemplateRun/MiniApp schema，因此不能直接挂旧 service。
- Wave 4：缺 typed `succeeded | failed | uncertain` effect outcome 和四域 v4
  resource/outbox；旧 Channel/Companion/Customer/Robot graph 不能直接注入。
- Wave 5：AgentExecution/Requirement/Schedule/IDMM 仍依赖旧 Conversation facts；
  必须先导出同一 AgentPlatform 实例的 canonical Session command/query ServiceKey，
  再实现各域 v4 repository/facade。
- Wave 2 尚缺 `vcs.push`、SSH、MCP/Connector、Browser 和 Computer action
  owners。`process.exec` 当前只对显式、host-resolved process binding 可用。

### Windows sidecar 源码阻塞

Windows `../codex` checkout 存在，HEAD 为 `4ee04c0...`，冻结 SHA
`dc2ccc6843abb09c9d297862dc10b6bd12a3935d` 是其祖先。但冻结 upstream 中不存在：

- `runtime/hello`
- `runtime/session/dispose`
- `native_action/start`

`vendor/codex-runtime/patches/series.json` 只记录 6 个 patch ID/ownership，没有实际
patch 源码，`upstream_source_files_in_vendor=[]`。因此普通构建或重命名
`codex.exe` 不能成为符合 NomiFun wire/credential/dispose 合同的 sidecar。
在取得 patch 源或实现等价可审计 sidecar 前，真实
`open -> ready -> turn -> observe -> cancel -> dispose` 仍是明确外部/源码阻塞，
不得用普通 upstream app-server 冒充。

当前 Windows 属于 affected cell；最终 clean candidate 必须运行完整
`C8-WIN-PRE`，不能使用 scoped attestation。但 C8 owner coverage 已按设计在上述
Wave owner 未闭合时 fail-closed，因此当前不能宣称 C8/HP-1/C8-MA/C8-MERGE。

### Windows C8 Gate 结果（2026-09-02）

在 clean SHA `b849e2ac7c3356468f86064a47180f5442a8e0a6` 上执行了一次完整
`bun run gate:agent-v2 -- c8-win-pre`。本轮未发生 ENOBUFS 或 access violation，
最终按设计 FAIL：

- PASS：toolchain、C7、contract、Domain registration、Fresh-v4 root、
  production host、production broker、UI build、Windows startup smoke、
  Windows installer contract；
- `baseline_fail`：repository-wide React/Arco/implicit-any UI typecheck；
- FAIL：production owner coverage 精确列出
  `wave1,wave2,wave3,wave4,wave5`；
- FAIL：workspace `cargo test` 在 app lib 的
  `bootstrap::canonical_host::tests::canonical_remote_rest_freezes_binding_and_auth_fence`
  开始后不再推进。该子进程运行 7.4 分钟、无子进程且 CPU 基本不增长后被本任务
  单独终止；Gate/cargo 随后记录 exit `0xffffffff` 并继续完成后续检查。没有重跑。

本轮 Gate residual：

- source total：`556`
- blocking：`0`
- contract-allowed：`26`
- deferred-to-C9：`530`
- unclassified：`0`
- canonical owner residual：`0`

Evidence：

- path：
  `build.noindex/agent-capability-v2/b849e2ac7c3356468f86064a47180f5442a8e0a6/c8-win-pre/summary.json`
- SHA-256：
  `b3b6ac5a52642f3edcef1b459b450d84958b8ce6be7100297fec5000a1df3e77`

生产 Remote Runtime 已有 35 秒 admission 与 45 秒 shutdown deadline；本轮日志
无法确认测试卡在 compose/open/revoke/close 的哪一步。为防止后续 workspace Gate
再次等待数分钟，该单一集成测试增加 60 秒总 test deadline，并把原 body 独立出来；
这只使 harness fail-fast，不改变生产逻辑，也不把 timeout 视为 PASS。按用户要求，
本轮只做后续 `--no-run` 编译，不再次执行该已知挂起测试。

新增 test deadline 与本状态更新产生后续 source SHA，因此 `b849e2ac` 的 Gate
证据只能作为诊断 evidence，不能提升为最终 C8 pass。真实 owner 与 sidecar
阻塞未改变。

## 增量实现记录（2026-09-02，Wave 1 Knowledge search/read owner）

Fresh-v4 `Wave1ApplicationHost` 已接入数据库无关、binding-backed 的只读
Knowledge owner：

- typed binding 必须恰好包含一个 `knowledge_base`，owner 与 principal 相同，
  并分别授予 `search` 或 `read`；
- `resource_id` 必须是 canonical lowercase UUIDv7；
- `typed_parameters.knowledge_root` 必须是绝对 host path，可选
  `knowledge_name` 不得为空；
- 每次调用都会重新验证物理 root，拒绝相对路径、`..`、非 Markdown、
  symlink、junction 与 name-surrogate reparse point；
- search 返回 opaque handle、resource ID、相对路径、heading、snippet 和 score，
  不暴露绝对路径，并限制 4096 个文档、单文件 8 MiB、总内容 64 MiB；
- read 复核 handle 中的 KB ID 与 binding resource 一致，返回相对路径、正文、
  byte size 与 SHA-256，并设置 8 MiB 读取上限；底层 root/path 错误会脱敏，
  不把绝对 host path 返回给 action caller。

该 owner 复用 `nomifun-knowledge` 已有的 root/path 安全策略、Markdown walker、
keyword scorer 与 document handle codec，不访问旧 Conversation/Nomi runtime，
也不修改 Fresh-v4 baseline/schema。

Windows 定向验证：

- `cargo test -p nomifun-knowledge bound_knowledge --lib`：`3 passed`；
- `cargo test -p nomifun-app --lib router::agent_platform_host
  -- --test-threads=1`：`22 passed`；
- `cargo check -p nomifun-knowledge`、`cargo check -p nomifun-app`：通过；
- 受影响 package `cargo fmt --check` 与 `git diff --check`：通过。

C8 production owner coverage 仍按设计 FAIL。`Wave1ApplicationHost` 的统一
fail-closed fallback 仍服务于 `web.search`、
`knowledge.write/autogen/embedding/rerank` 和 `skill.invoke`；在这些 action
获得真实 owner 前不得删除 fallback、放宽 Gate，或宣称 Wave 1/C8/HP-1 完成。

### Knowledge binding UI 与 write 安全审计（2026-09-02）

Agent Preset 编辑器已直接接入现有 KnowledgeBase catalog。用户选择真实知识库后，
Draft 自动写入：

- `resource_id = knowledge_base_id`；
- `typed_parameters.knowledge_root = root_path`；
- `typed_parameters.knowledge_name = name`。

根目录不存在的知识库不会作为可选项启用，用户无需手填 UUIDv7 或绝对路径。定向
验证通过：

- Agent Settings tests：`9 passed`；
- `bun run check:i18n`：`7082 keys / 33 modules`；
- `bun run build:ui`：`7720 modules transformed`，仅保留既有 chunk warning；
- `bun run dev`：2026-09-02 15:02 本地启动完成，`NomiFun` desktop 窗口响应正常，
  无启动 panic 或渲染崩溃。

Playwright harness 未完成页面截图：当前 Node REPL 的全局 `playwright` 包发生
ESM export 装载错误，仓库未安装本地 Playwright。已停止重试；Agent Preset 子页的
Knowledge 下拉选择保留为人工交互验收项。

本轮曾实现一个未提交的 `knowledge.write` 原型，但只读安全审计发现：

- `safe_md_path` 校验与最终 open/rename 分离，无法阻止外部进程在窗口内替换
  symlink/junction/reparse parent；
- 现有“读取比较后 rename”不是面对外部编辑器的原子 CAS，可能覆盖竞争修改；
- 仅观察最终 content hash 不能证明历史 Effect 归因，也不能替代
  `uncertain -> reconciled` durable fact；
- timeout-and-drop 的 blocking 目录创建不适用于 mutation。

因此该原型未进入分支，生产 `knowledge.write` 继续 fail-closed。要恢复实现，
必须先交付跨
Windows/macOS/Linux 的 root-anchored no-follow filesystem primitive、明确
create-vs-append DTO，以及 typed publication outcome 和 durable
`started -> succeeded|failed|uncertain -> reconciled` 记录。

### Knowledge search/read anchored filesystem 加固（2026-09-02，Windows 基线记录）

后续只读安全审计确认，原先 `safe_md_path` 检查后再按绝对路径执行
`WalkDir` / `File::open` 仍存在本地外部进程替换 parent 为
symlink/junction 的 TOCTOU 窗口。因此 search/read 已改为
`cap-std 4.0.3` + `cap-fs-ext 4.0.3` 的 root-anchored capability：

- 从 `/`、Windows drive root 或 UNC share root 建立初始 handle；
- 每个 Knowledge root component 都单独 `open_dir_nofollow`；
- search 通过已打开的 `Dir` handle 流式枚举，每个 child directory 和最终
  Markdown file 都再次 no-follow open，不再使用路径式 `WalkDir`；
- read 只通过 parent `Dir` 的 relative no-follow open 获取最终 file handle，
  检查、大小限制和读取都在同一已打开对象上完成；
- binding search 不再跨调用缓存路径对应的正文，避免 root 被替换后因相同
  mtime/size 复用旧对象内容；
- 新增 32768 filesystem entries、4096 documents、128 directory depth、
  单文件 8 MiB 和总正文 64 MiB 上限。

Windows 基线 fault tests 证明：

- root component 为 junction 时 capability 建立失败；
- root 内 linked directory 的 Markdown 不会被 search/read 打开；
- child directory handle 存活时，Windows 拒绝 rename/path replacement。

同一 fault test 包含 Unix 分支：如果 pathname replacement 成功，后续读取必须
仍绑定原 directory handle；在该 Windows 基线记录形成时，Unix 分支尚需原生
执行，不能由 Windows 结果代替。当前 macOS 结果见下方续记。

Windows 基线定向验证：

- anchored filesystem tests：`3 passed`；
- bound Knowledge tests：`3 passed`；
- Kernel→Host Knowledge tests：`4 passed`；
- Codex runtime release tests：`2 passed`；
- macOS helper tests：`2 passed`；
- `cargo check --locked -p nomifun-knowledge -p nomifun-app
  -p nomifun-codex-runtime`：通过；
- Agent v2 contract check、C8 Gate self-test、C7 domain-wave Gate：通过。
- `bun run dev`：2026-09-02 16:00 本地完成编译并启动，desktop 进程响应正常，
  canonical dev root 无 startup panic；烟测后进程树已清理。

`cargo clippy -p nomifun-knowledge --lib --tests -D warnings` 仍被该 crate 及
`nomifun-common` 的既有 broker/routes/service warning 基线阻断。本批新增的
collector 参数过多、single-match 和 unit-struct Default warning 已修复；未扩大
到无关 clippy 清理。

新增依赖改变 `Cargo.lock`，canonical generator 和机器镜像已同步：

- `Cargo.lock`：
  `792362a8edf3e7994a59c89c182698fe1d4661fec5d04409e960a4185663acc9`
- runtime release：
  `7d86f492219867e52b35db103a8df8282ba0fd1acc0079b8c5d01b3236a7e17f`
- platform validation：
  `fe631261c82226d5d824bd2ee35e1195c9eefb8afdc1522790b825ca9258305c`
- contract digest ledger：
  `79aaa49f672c844bb65cf50564dc9964bf11fc46a1b064a7761b75554e5e049f`

这只关闭 search/read 的 path-replacement 风险，不解决跨外部编辑器的原子
compare-and-replace；`knowledge.write` 仍保持 fail-closed。该段 Windows 基线
结果不能写成 C8-MA/C8-MX/C8-LD/C8-LH PASS；当前 macOS arm64 定向复验结果见
下方续记。

原生主机复验命令（本轮已执行，供 Windows 复现）：

```bash
cargo test --locked -p nomifun-knowledge service::anchored_fs::tests --lib -- --test-threads=1
cargo test --locked -p nomifun-knowledge bound_knowledge --lib -- --test-threads=1
cargo test --locked -p nomifun-app wave1_knowledge --lib -- --test-threads=1
cargo check --locked -p nomifun-knowledge -p nomifun-app -p nomifun-codex-runtime
cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check
bun run gate:agent-v2 -- c7-domain-waves
```

## macOS arm64 Knowledge anchored filesystem 原生复验（2026-09-02）

本轮从 Windows 已推送的
`5d6918241115daa6fb875bb463847ee934f6c482` 开始；local/remote exact-equal，
`git merge-base --is-ancestor 5d691824... HEAD` 退出 `0`，启动工作树 clean。
原生环境为 Darwin 25.5.0 / arm64，`sysctl.proc_translated=0`，
`rustc host=aarch64-apple-darwin`。

首次执行 anchored filesystem tests 时出现 macOS 特有失败：

- `tempfile` 暴露 `/var/folders/...` 路径；
- macOS 的 `/var` 是固定系统别名，目标为 `/private/var`；
- 原实现从 `/` 对每个词法 component 执行 `open_dir_nofollow`，因而在真正的
  Knowledge root 之前错误拒绝了系统 `/var` 别名；
- 结果为 `1 passed / 2 failed`，不是测试环境豁免。

修复与回归提交：

- `efbcb598191e66f953b84b8f0dfeb128010d9695`
  `fix(knowledge): accept macos system alias roots safely`；
- `1a547f3a1ef25d56b799cca1cd429ba22ce764ff`
  `fix(knowledge): verify macos system root aliases`；
- `23e039ff0262d11f6085dea27d4019da15ff4412`
  `test(knowledge): cover replaced bound root reads`。

修复范围只包含
`crates/backend/nomifun-knowledge/src/service/anchored_fs.rs`：

- macOS 仅对 `/var`、`/tmp`、`/etc`、`/home` 的固定系统别名做词法映射；
- 映射后的每个 component 仍通过 `open_dir_nofollow` 打开；
- 用户控制的中间 symlink 和最终 Knowledge root symlink 仍 fail-closed；
- search/read 的 child directory 与最终 Markdown file 仍只相对于已打开
  `Dir` handle 执行 no-follow open，没有恢复绝对路径 `File::open`；
- `BoundKnowledgeReadService` 仍不复用旧 `safe_md_path`、`WalkDir` 或
  pathname+mtime 正文缓存；每次调用建立新的 root capability。

新增/原生闭合的 fault evidence：

1. macOS `/var` 系统别名下的真实 root 可以建立 capability；
2. linked Knowledge root、intermediate linked component、root 内 linked
   directory 和 linked Markdown file 均被拒绝，search 不读取外部正文；
3. Unix child handle 与 root handle 在 pathname rename/replacement 后仍读取原
   directory object；
4. entry/document/depth/per-file/total-byte limits 均返回 fail-closed error。

最终定向验证：

- anchored filesystem tests：`9 passed`；
- bound Knowledge tests：`4 passed`；
- Kernel→Host `wave1_knowledge` tests：`4 passed`；
- `cargo check --locked -p nomifun-knowledge -p nomifun-app
  -p nomifun-codex-runtime`：通过；
- Agent v2 contract check、C7 domain-wave Gate：通过；
- `bun run check:i18n`：`7082 keys / 33 modules`；
- `bun run build:ui`：`7720 modules transformed`；
- Agent Settings binding/navigation focused tests：`6 passed`；
- bound root pathname replacement regression：替换 root 后重新 search/read
  得到新目录对象内容，未复用旧正文；
- `cargo fmt -p nomifun-knowledge -p nomifun-codex-runtime -- --check` 与
  `git diff --check`：通过；
- `Cargo.lock` 未变化，因此未运行 canonical contract `write`。
- 在 `1a547f3a...` 的 alias-target guard 之后，anchored/bound/app 三组
  Knowledge 测试再次通过（`9 + 4 + 4`）；该 guard 只拒绝不符合固定系统别名
  target 的重写，不改变用户路径的 no-follow 行为。

桌面烟测执行了一次 `bun run dev`：完成 debug 编译并启动
`target/debug/nomifun-desktop`，WindowServer 观察到一个 on-screen
`nomifun-desktop` 窗口（1280x832），Vite dev surface 返回 HTTP 200，启动日志无
panic；随后使用 Ctrl-C 正常结束，desktop/Vite/Tauri 进程和 5173 listener 均已
清理。当前执行环境没有 macOS Accessibility/Screen Recording TCC 权限：
`osascript` 返回辅助访问拒绝，ScreenCaptureKit 返回 `-3801`。因此不能把
“窗口非空白”以及 Agent Presets 中的实际点击、KnowledgeBase 下拉选择和 Preview
视觉结果记为人工 PASS。对应数据映射由 focused UI test 证明：

- `resource_id = knowledge_base_id`；
- `typed_parameters.knowledge_root = root_path`；
- `typed_parameters.knowledge_name = name`；
- canonical Agent Settings route 存在。

macOS arm64 helper 只运行一次（在 clean `efbcb598...` code checkpoint），
最终状态为 FAIL（不是 C8-MA evidence）；后续 `1a547f3a...` 只增加 alias
target guard，未重新运行 helper：

- native Darwin arm64 / 非 Rosetta：pass；
- app 仅包含 `arm64`，缺少 `x86_64` Universal slice；
- 真实 arm64 sidecar 缺失，期望 SHA-256：
  `7863db3a77545eec8966483f26fb5b493aea6e285ac35b5c29d0920342438060`；
- 缺 endpoint/binding/token/provider/credential，未执行
  `open -> ready -> initial turn -> observe -> cancel -> dispose`。

本轮没有运行完整 `c8-ma` Gate，没有生成 synthetic
`PlatformCellEvidence`，没有恢复 `knowledge.write` 原型，也没有放宽 Gate
allowlist。当前只能声明 Knowledge search/read 的 macOS anchored filesystem
定向行为通过；不能声明 C8-MA、HP-1 或发布完成。
