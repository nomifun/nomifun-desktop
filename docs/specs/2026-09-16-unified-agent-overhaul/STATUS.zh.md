# UARC 状态台账

> 唯一状态 owner：Integration
> 更新时间：2026-09-17
> 当前阶段：Wave 3 gate complete / Wave 4 ready
> 当前 source HEAD：`877b1a751536e40a3185c31790e6e63e86c6fa32`
> UARC-000 冻结提交：`2147863da396835240296ec0a9b865200050b438`
> Wave 0 inventory 提交：`440626d91dc800af0c5b2c81cf13f63eac9abfaf`
> UARC-010 实现提交：`c059728ae4395fcdf72df59a54b4e53e8b7562a1`
> UARC-011 实现提交：`fa164520f72a053e8e244721cb9682bc58b1269b`
> UARC-012 实现提交：`82954016810ed5fabe48248adc4952d2bd5f199e`
> UARC-013 实现提交：`5f317024d63c6845d896d379c201254101410d1b`
> UARC-014 实现提交：`3983deff110f7e22b4eb85b6cc8ce00e9e8c4009`
> Wave 1 gate 修复提交：`8afc40c7a`
> Wave 2 实现提交：`efe80298f`
> Wave 3 实现提交：`8454133b124a3d38636f882b098de80ce17028c0`
> 当前主机：Windows
> Initiative 状态：`active / Wave 4 ready`

## 1. 当前事实

- source HEAD 上继承的 21 个 Git 状态条目（展开为 26 个实际文件）已逐文件归属并冻结到
  `2147863da396835240296ec0a9b865200050b438`；详见
  [UARC-000 工作树冻结清单](UARC-000-WORKTREE-INVENTORY.zh.md)。
- 已完成三份目标设计和 Browser 历史文档纠正；这些是实施输入，不是生产完成证据。
- 先前实现的“执行引擎设置页”仍按 Nomi/Coding 双 Runtime 展示，与最新单 Runtime 决定不完全一致；
  归属 `UARC-050`，不能作为最终 UI 直接交付，也不能未经检查删除。
- 当前 Browser 仍是 Conversation-scoped BrowserWorkspace，Guid 仍会创建浏览器专属空 Session。
- 当前源码已经包含 macOS 独立 CEF host、原生 fixture 与部分底层验证；生产注入、产品会话闭环和
  UARC 新 Browser Resource 模型适配仍未完成。UARC-061 必须复用这套基线，不得回退到 WKWebView。
- 产品组合只安装一个 `nomifun.nomi` provider/factory，旧 Nomi factory 与 `nomifun.coding` family
  已不可达；旧实现源码等待 `UARC-020/052` 提取与物理删除。
- AgentPreset/API 已删除 Runtime selector；Kernel/Control Plane/App projection 按 contribution 编译，
  不再用 Runtime family 或 Capability ID 映射决定支持。136/136 旧 ID 已有机器可验退役路线，现存
  Domain/UI migration input 分别归 `UARC-020..053`，没有兼容翻译器。
- canonical `/api/agent-sessions` 已切换 generation 5 Store；旧领域入口等待后续 wave/cutover 删除。
- Wave 1 的 UARC-010/011/012/013/014 已集成；Windows 静态、UI、Desktop 与 debug native build
  milestone 已通过；当时的 `skill.hooks` Context factory transition 已由 `UARC-022` 闭合。
- Wave 2 的 UARC-020/021/022 已统一集成：一个自适应 Runtime loop、四个 Workspace Module、exact
  per-tool MCP/Plugin/Skill contribution、Store effect causation 与 Windows owner/path/process 证据均已闭合。
- Wave 3 的 UARC-030/031/032 已统一集成：Web/Knowledge/Memory、Channel/Companion/Customer Service、
  Creation/Workshop/Office/Plugin Development 均只暴露产品 Module 与 exact Actions；scene Context 从 binding
  派生，provider/model transport 细节不再成为 Agent grant。
- App 的 Wave 2 gate 为 499/499；过滤的 10 个 Robot unified fixture 明确归 `UARC-042`，另 1 个
  Bootstrap SQLite WAL 字节比较不稳定项保留到 Windows 回归闭合，不构成 UARC authority fallback。
- 当前没有本轮源码重构的 macOS 编译、原生或打包证据。

## 2. 已确认产品决定

- 单一官方 Nomi Runtime；
- 不支持第三方/多 Runtime；
- Runtime 直接替换，不建设双 Runtime 灰度；
- cancel/recovery/compaction 为 Runtime invariants；
- Runtime 按回合自适应，无静态执行档位；
- Module + Action grants 替代 136 个碎片化 authoring IDs；
- 最简 Agent 无外部 Tool，可处理用户当前输入附件；
- 通用 Agent 默认启用 Skill/MCP/Schedule/Browser/Computer；
- IDMM 从 Agent 路径删除，Terminal 可保留专用 Supervisor；
- AutoWork 复用 AgentExecution；
- Browser 删除专属 Session 入口，成为任意 Agent 可授权能力；
- Attached Chrome 使用简单安装级连接，不做每 Session/Tab 二次授权；
- Agent 数据 clean cut 采用方案 A：清空 Agent 数据，保留非 Agent 配置；
- 大胆删除无用代码，不保留永久兼容；
- UI 任务必须交付完整、美观、可用的产品交互。

## 3. 当前任务

| Task | 状态 | Owner | Windows | macOS | 说明 |
| --- | --- | --- | --- | --- | --- |
| `UARC-000` | windows_verified | Integration | verified | n/a | barrier `2147863da`；26/26 文件归属，基线 gate 通过 |
| `UARC-001` | integrated | Integration | verified | pending | 机器 inventory/self-test/timing/platform gap 已完成；Mac gaps 保持 pending |
| `UARC-010` | integrated | Integration | verified | n/a | Module 多 contribution、authoring policy、exact Action grant 已闭合 |
| `UARC-011` | integrated | Integration | verified | n/a | generation 5 Store、main migration、effect ledger、reset gate 已闭合 |
| `UARC-012` | integrated | Integration | verified | n/a | 单一 AgentSession owner、完整生命周期 API 与 exact receipt 已闭合 |
| `UARC-013` | integrated | Integration | verified | n/a | 单一官方 provider/factory、Driver lifecycle 与 typed host ports 已闭合 |
| `UARC-014` | integrated | Integration | verified | n/a | Runtime selector 删除、通用 Compiler/projection、136-ID retirement 已闭合 |
| `UARC-020` | integrated | Integration | verified | pending | 自适应单 Runtime、长程 Coding、exact restart proof |
| `UARC-021` | integrated | Integration | verified | pending | 四个 Workspace Module、owner effects、Artifact/VCS hardening |
| `UARC-022` | integrated | Integration | verified | pending | Skill locks、per-tool MCP、Plugin contributions；无 broad Runtime/proxy |
| `UARC-030` | integrated | Integration | verified | pending | Web、Knowledge、Memory Module 与 sensitive Action authority |
| `UARC-031` | integrated | Integration | verified | pending | scene-derived Context；Channel/Companion/Customer Actions |
| `UARC-032` | integrated | Integration | verified | pending | Creation/Workshop/Office/Plugin Modules 与 Creative UI |
| 其余任务 | planned | unassigned | pending | pending/not applicable | 按 manifest 依赖释放 |

## 4. 当前 dirty worktree 归属

| 文件组 | 已确认归属 | 处理原则 |
| --- | --- | --- |
| `docs/reviews/2026-09-16-*`、`docs/specs/2026-09-16-*`、Browser superseded notices | `UARC-000` | 保留并作为设计/审计基线 |
| `docs/specs/2026-09-16-unified-agent-overhaul/**` | `UARC-000/001` | 保留，Integration 独占 |
| Settings Router/Sider/ExecutionEngines/JavaScriptRuntime 页面 | `UARC-050` | 保存现状证据；按单 Runtime 目标重做，不把双 Runtime UI 当最终设计 |
| Settings/AgentSession tests、i18n 生成物 | `UARC-050` + Integration | 源文案归 UI task；生成物由 Integration 更新 |

逐文件列表、删除文件和 Integration-only 生成物的 owner 已记录在冻结清单；本表只保留汇总。

## 5. 验证状态

| Gate | 当前结果 | 是否可用于 UARC 完成 |
| --- | --- | --- |
| 设计文档 `git diff --check` | passed | 仅证明文档格式 |
| 136 Capability inventory coverage | 136/136 | 仅证明设计映射完整 |
| 先前 Settings UI typecheck/定向测试 | 历史通过 | 最新目标已变化，不作为最终验收 |
| UARC-000 `git diff --check` | passed | 可用于 barrier 格式基线 |
| UARC-000 UI typecheck | passed | 可用于当前设置页检查点基线 |
| UARC-000 设置页/导航定向测试 | 9 passed / 0 failed | 可用于当前设置页检查点基线 |
| UARC-000 i18n parity/types | passed，7,787 keys / 35 modules | 可用于当前生成物基线 |
| UARC-000 desktop UI boundary | passed，1,931 renderer sources | 可用于 880×600 边界基线 |
| UARC-000 `bun run check` | passed | barrier 候选完整静态 gate |
| UARC boundary scanner/self-test | passed，3,029 files / 13 groups / 4,094 matches | UARC-001 machine-readable baseline |
| Wave 1 targeted Rust baseline | 198 passed（contracts/kernel/session/engine-core） | Windows shared foundation baseline |
| Browser/Process/Terminal targeted baseline | 303 passed with Process serial control | Windows platform timing baseline |
| Process Runtime default parallel sample | interrupted after >210 s | open anomaly owned by `UARC-021`; serial 119/119 passed |
| UARC-010 Contracts/API/Kernel | 103 + 533 + 60 passed | Module/Action/authoring/Snapshot authority gate |
| UARC-010 Session/Engine/Plugin consumers | 25 + 14 + 40 passed | shared consumer regression |
| UARC-010 contract generator/rustfmt/boundary | passed | generated schema and reachability consistent |
| UARC-014 Contracts/API/Kernel/Control Plane | 107 + 534 + 60 + 48 passed | generic contribution Compiler 与 136/136 retirement gate |
| UARC-014 AI Agent/Engine Core/App focused | 545 + 14 + 19 passed | single provider、projection、middleware、state regression |
| UARC-014 App compile/generator/boundary | lib/bin/tests + generator passed | selector 45→21；legacy matches 4,094→3,947 |
| Wave 1 full UI | 3,575 passed / 0 failed | source HEAD test drift repaired；renderer behavior unchanged |
| Wave 1 Desktop/native debug | 144 passed、3 environment ignored；`build:fast` passed | Windows desktop binary produced |
| Wave 1 Core workspace transition | stopped at domain-wave1 9/11 | 2 `skill.hooks` Context factory failures owned by `UARC-022` |
| App full transition | 444/497 passed | 27 UARC-021 + 26 UARC-022 failures；无 UARC-014 Runtime/Compiler failure |
| UARC-011 Contracts/Store | 105 + 27 passed | empty baseline, Turn/Event/Effect/Resource invariants |
| UARC-011 DB reset/schema/migration | 3 + 20 + 5 passed | non-Agent preservation and main SQLite parity |
| UARC-011 retained root consumer | 14 passed | temporary alias remains owned by `UARC-054` |
| UARC-012 Store/Conversation | 27 + 335 passed | single owner、one-active-turn、exact replay、Fork ready 与 projection rebuild |
| UARC-012 API/App boundary | 534 + 6 passed | lifecycle DTO、canonical route reachability 与 legacy-authority non-reentry |
| UARC-012 App check/rustfmt/boundary | passed，3,032 files | main-pool composition and no UARC legacy growth |
| UARC-013 AI Agent/Engine Core | 552 + 14 passed | one-active-turn、cancel/cleanup、quarantine 与 typed ports |
| UARC-013 App composition | 2 + 6 passed | one provider/factory、foreign-family rejection 与 state wiring |
| UARC-013 App compile/boundary | lib/bin/tests passed，3,033 files | no registration API; compatibility 28→26 |
| UARC-020 Coding/Engine/AI Runtime | 40 + 15 + 548 passed；新增 focused 1 | simple-turn、multi-compaction、restart、cancel/steer |
| UARC-021 Domain/Session/File/App host | 17 + 28 + 378 + 29 passed | exact Action resources、effect causation、path/VCS/process owners |
| UARC-022 Nomi/Plugin consumers/Kernel/Control Plane/Wave4 | 638 + 41 + 61 + 49 + 20 passed | frozen Skill、per-tool MCP、Plugin hooks、no authority expansion |
| Wave 2 App gate | 499 passed / 0 failed / 11 filtered | 10 UARC-042 Robot fixtures + 1 Bootstrap WAL byte-test anomaly |
| Wave 2 App feature compile | default + `browser-use,computer-use` passed | Windows desktop feature composition compiles |
| Wave 2 contract/boundary | generator check + 108 contracts + scanner self-test passed | 3,035 files；baseline anomaly 1；Mac gaps 3 |
| UARC-030 Domain/AI/App | Wave1 4 + Knowledge 329 + AI 542 + App 73 passed | Product modules、sensitive read/write、citation provenance |
| UARC-031 Domain/App | Channel 353 + Companion 275 + Customer 31 + Wave4 target 6 + App 10 passed | scene binding、receipts、persona、notes/handoff |
| UARC-032 Domain/App/UI | Wave3 13 + Office 89 + App slash 2 + UI 23 passed | product actions、Office owner、two-authority slash discovery |
| Wave 3 compile/contract/UI boundary | Browser feature + live smoke no-run + generator/inventory/typecheck passed | 1,931 renderer sources；880×600 boundary |
| Wave 3 full App transition probe | 491 passed / 11 known future-owned failures | 10 UARC-042 Robot + 1 final Windows WAL anomaly |
| Commercial selected-model probe | StepFun Coding Plan `step-3.7-flash` direct HTTP 200 | canonical turn dispatch pending UARC-051/052；未记作 integrated smoke pass |
| Windows UARC full gate | not run | 否 |
| macOS UARC shared compile | not run | 否 |
| macOS native Browser/Computer/Process | not run | 否 |
| Windows/macOS packages | not run | 否 |

## 6. Test Lease

| Lease | 当前 owner | 状态 |
| --- | --- | --- |
| Windows Cargo | none | free |
| Windows Full UI | none | free |
| Windows Native Desktop | none | free |
| Windows Packaging | none | free |
| macOS Cargo/UI/Native/Packaging | no active Mac host | unavailable |

## 7. Blockers

- UARC 实施无产品决定 blocker。
- macOS 任务需要可用 Mac 主机；在主机可用前状态保持 `pending`，不能标完成。
- Wave 3 barrier 已收口；`UARC-033/034/040` 可从同一 barrier 启动，Integration 继续独占共享合同、
  根配置、生成物、状态台账和最终合并。
- App 当前只保留 10 个 `UARC-042` Robot device-MCP transition fixture；旧 proxy 已物理删除，后续必须
  通过 materialized Robot Actions 修复。
- Bootstrap 的 `v3_validation_failures_preserve_data_with_or_without_prior_retirement` 在 Windows 对 SQLite
  WAL checkpoint 后的主文件做字节级比较，单测可通过也可复现失败；不涉及 UARC 数据丢失，须在
  `UARC-060/064` Windows gate 前改为稳定的持久状态证据并全量复跑。
- canonical `/api/agent-sessions` 已只写 generation 5；Cron/Channel/AutoWork/Companion/IDMM/
  AgentExecution/Remote 的旧 Conversation writers/readers 仍由 `UARC-033/034/051/054` 迁移删除。
  Fresh-v4 root coordinator aliases 仅为 `UARC-054` 保留。

## 8. Next ready tasks

1. `UARC-033`：Requirements、AutoWork、AgentExecution 与 IDMM 收敛。
2. `UARC-034`：Schedule、Notification、Remote 与 SSH Module。
3. `UARC-040`：Browser 产品模型与共享 Resource/Provider contract。

## 9. 状态更新模板

```text
### <timestamp> <task-id>

- Barrier/source:
- Owner/write set:
- Changed:
- Deleted:
- Retained + reason:
- Tests:
- Windows:
- macOS:
- Not run:
- Remaining/blocker:
- Next ready tasks:
```

## 10. 事件记录

### 2026-09-17 UARC-000 started

- Barrier/source: `877b1a751536e40a3185c31790e6e63e86c6fa32`; upstream divergence `0/0` at start.
- Owner/write set: Integration; inherited 26-file dirty checkpoint enumerated in `UARC-000-WORKTREE-INVENTORY.zh.md`.
- Changed: no inherited file discarded; manifest now assigns the settings surfaces consumed by `UARC-050`.
- Deleted: none by UARC-000; the two already-deleted legacy `AgentSettings` files remain part of the authorized checkpoint.
- Retained + reason: confirmed design/Browser corrections and settings-page checkpoint, because both are explicit implementation inputs.
- Tests: `git diff --check`; UI typecheck; 9 focused UI tests; i18n check; desktop UI boundary; `bun run check` — all passed. Dirty inventory comparison covers 26/26 inherited files.
- Windows: baseline checks passed; barrier pending.
- macOS: not applicable to UARC-000; no Mac host evidence claimed.
- Not run: Rust/native/package gates are outside UARC-000's documentation/settings checkpoint scope.
- Remaining/blocker: stage and inspect the complete diff, rerun final whitespace validation, commit the barrier. No product blocker.
- Next ready tasks: none until UARC-000 is integrated; then `UARC-001`.

### 2026-09-17 UARC-000 integrated and gate complete

- Barrier/source: source `877b1a751536e40a3185c31790e6e63e86c6fa32`; frozen barrier `2147863da396835240296ec0a9b865200050b438`.
- Owner/write set: Integration; the inherited settings and design checkpoint is now immutable in Git.
- Changed: preserved 14 design/review/Browser documents, 12 settings/UI files, and added exact ownership/gate records.
- Deleted: only the two already-authorized legacy `AgentSettings` files; no UARC-000 cleanup deleted user work.
- Retained + reason: Browser engine/CEF evidence and settings-page checkpoint remain inputs for their manifest owners.
- Tests: `git diff --cached --check`; `bun run check`; 9 focused UI tests — all passed. Inventory coverage `26/26`.
- Windows: verified for the UARC-000 documentation/settings baseline.
- macOS: not applicable; existing CEF evidence is retained but not promoted to UARC product verification.
- Not run: Rust/native/package gates, because UARC-000 changed no Rust/native/package implementation.
- Remaining/blocker: none for UARC-000. External Mac host remains a later platform prerequisite, not a Wave 0 blocker.
- Next ready tasks: `UARC-001` only; no Feature task is released.

### 2026-09-17 UARC-001 started

- Barrier/source: UARC-000 closeout `6b09b4094`; frozen content barrier `2147863da`.
- Owner/write set: Integration; `docs/specs/2026-09-16-unified-agent-overhaul/**` and `scripts/check-uarc-boundary.mjs` only.
- Changed: task claimed; inventory schema and existing boundary-script conventions are being inspected.
- Deleted: none planned.
- Retained + reason: existing focused boundary scripts remain separate domain gates; UARC boundary inventory will aggregate facts without replacing them.
- Tests: pending inventory self-test and timed baseline commands.
- Windows: active.
- macOS: pending inventory only; no Mac host evidence claimed.
- Not run: implementation gates pending inventory construction.
- Remaining/blocker: enumerate production legacy reachability, storage ownership, test costs and macOS-only work. No blocker.
- Next ready tasks: none until UARC-001 gate; then `UARC-010`.

### 2026-09-17 UARC-001 integrated and Wave 0 gate complete

- Barrier/source: UARC-000 closeout `6b09b4094`; inventory commit `440626d91dc800af0c5b2c81cf13f63eac9abfaf`.
- Owner/write set: Integration; only UARC docs and `scripts/check-uarc-boundary.mjs` changed.
- Changed: machine-readable 13-group reachability inventory, four storage ownership chains, timing samples, macOS evidence gaps and a self-testing scanner.
- Deleted: none; existing focused boundary scripts remain intact.
- Retained + reason: current old paths remain only as explicitly counted debt with manifest owners and zero-reference completion requirements.
- Tests: scanner self-test and JSON parse passed; contracts 101, kernel 58, agent-session 25, engine-core 14, Browser Platform 50, Process Runtime serial 119 and Terminal 134 tests passed; `git diff --cached --check` passed.
- Windows: verified for Wave 0 inventory and targeted baseline.
- macOS: pending by design; three exact evidence groups are machine-readable and owned by `UARC-061/062/063`.
- Not run: no macOS host, native CEF product, DMG/signing or notarization evidence.
- Remaining/blocker: Process Runtime default parallel test anomaly is open under `UARC-021`; no blocker for starting serial Wave 1.
- Next ready tasks: `UARC-010` only.

### 2026-09-17 UARC-010 started

- Barrier/source: Wave 0 closeout `89a4f820d06aa75c4a248932b5c8f12d79988785`.
- Owner/write set: Integration; `nomifun-agent-contracts`, `nomifun-api-types`, `nomifun-agent-kernel` only.
- Changed: task claimed; current manifest/selection/snapshot/compiler contracts are being audited before edits.
- Deleted: pending removal of single-action authoring assumptions and authorable transport/resource/runtime-internal forms.
- Retained + reason: exact action schema, effect class, resource binding and Snapshot authority remain security boundaries.
- Tests: pending contract, schema and Kernel authority gates.
- Windows: active.
- macOS: not applicable to this shared contract task; later Mac consumers still require platform verification.
- Not run: no task implementation yet.
- Remaining/blocker: implement and verify multi-contribution Module plus exact Action grants. No blocker.
- Next ready tasks: none until UARC-010 barrier; then `UARC-011`.

### 2026-09-17 UARC-010 integrated and gate complete

- Barrier/source: Wave 0 closeout `89a4f820d`; implementation `c059728ae4395fcdf72df59a54b4e53e8b7562a1`.
- Owner/write set: Integration; Contracts, API Types, Kernel, generated contracts and UARC evidence only.
- Changed: Module multi-contribution validation, authoring policy, exact Action grant DTO/compiler/Snapshot/authority and contribution-driven dispatch.
- Deleted: implicit empty-allowlist-to-all expansion; Tool/Context kind-based generic dispatch; direct authoring of platform/dependency/internal forms.
- Retained + reason: `CapabilityKind` is presentation summary/resource-provider dispatch; v1 outer selection names remain until the serial `UARC-014` document switch.
- Tests: Contracts 103, API Types 533, Kernel 60, Agent Session 25, Engine Core 14 and Plugin Platform 40 passed; contract generator, targeted rustfmt, UARC boundary and staged whitespace checks passed.
- Windows: shared contract implementation verified.
- macOS: not applicable to this contract task; Mac platform consumers remain pending in their tasks.
- Not run: full Wave 1 milestone gate waits for `UARC-014`. Control Plane probe is 48/50 with two intentional old-contract failures assigned to `UARC-014`.
- Remaining/blocker: no UARC-010 blocker; do not restore implicit grants or kind authority to satisfy transitional tests.
- Next ready tasks: `UARC-011` only.

### 2026-09-17 UARC-011 started

- Barrier/source: UARC-010 closeout `dd2c098dab8bc13bc8693719953f9864498360d4`.
- Owner/write set: Integration; `nomifun-db`, `nomifun-agent-session`, Agent contract schema/generated artifacts and UARC evidence.
- Changed: task claimed; current v3 migrations, Fresh-v4 schema, Session Store and reset paths are under ownership audit.
- Deleted: pending legacy Conversation/Message/receipt/runtime-event/effect schema and duplicate Fresh-v4 root assumptions.
- Retained + reason: one SQLite transaction boundary plus content-addressed payload/checkpoint storage.
- Tests: pending empty DB, schema owner, effect ledger and Agent-only reset preservation gates.
- Windows: active.
- macOS: not applicable to this schema task; filesystem/path behavior remains a later Mac revalidation item.
- Not run: no implementation gate yet.
- Remaining/blocker: freeze exact non-Agent preserved tables and replace Agent facts without legacy readers. No blocker.
- Next ready tasks: none until UARC-011 barrier; then `UARC-012`.

### 2026-09-17 UARC-011 integrated and gate complete

- Barrier/source: UARC-010 closeout `dd2c098da`; implementation `fa164520f72a053e8e244721cb9682bc58b1269b`.
- Owner/write set: Integration; DB, Agent Session Store, Agent schema/contracts/generated artifacts, Cargo lock and UARC evidence.
- Changed: generation 5 schema, main migration 109, canonical Turn/Event/Effect/Resource facts, exact main-pool Store validation and Agent-only reset.
- Deleted: `0001_fresh_v4.sql`, old Session table identities and projection embedded-event compatibility reader.
- Retained + reason: content-addressed object/checkpoint contract; temporary Fresh-v4 root aliases have the explicit `UARC-054` deletion owner.
- Tests: Contracts 105; Agent Session 27; reset/schema parity 3; ID schema 20; published migration upgrade 5; v4-root consumer 14; generator/rustfmt/boundary/whitespace checks passed.
- Windows: verified for schema, SQLite transaction behavior and reset preservation.
- macOS: not applicable to this shared schema task; path/permission behavior is revalidated later on Mac.
- Not run: full post-migration `nomifun-db` 411-test suite; the same task's pre-migration serial baseline was 411/411 and all migration-sensitive post-change suites passed.
- Remaining/blocker: none for UARC-011. Old production Session writers remain intentionally owned by cutover tasks, not by a compatibility reader in the new Store.
- Next ready tasks: `UARC-012` only.

### 2026-09-17 UARC-012 started

- Barrier/source: UARC-011 closeout `9b62887aee46130a144bca7369c01823718e79c8`.
- Owner/write set: Integration; Conversation, Agent Session Store and App router/composition paths.
- Changed: task claimed; current Conversation creation, AgentSession routes, receipts and `extra` authority are under call-graph audit.
- Deleted: pending double-create/dual-ID path, compatibility projections and JSON authority.
- Retained + reason: user-facing “对话/conversation” terminology remains a projection of one AgentSession.
- Tests: pending open/turn/steer/cancel/fork/delete, idempotency and projection rebuild gates.
- Windows: active.
- macOS: not applicable to the shared API task.
- Not run: no implementation gate yet.
- Remaining/blocker: wire generation 5 Store into composition and route every Agent entrypoint through one receipt. No blocker.
- Next ready tasks: none until UARC-012 barrier; then `UARC-013`.

### 2026-09-17 UARC-012 integrated and gate complete

- Barrier/source: UARC-011 closeout `9b62887a`; implementation `82954016810ed5fabe48248adc4952d2bd5f199e`.
- Owner/write set: Integration; Agent Session Store, Conversation owner, API DTO, App router/composition, Cargo lock and UARC evidence.
- Changed: generation 5 canonical owner; direct open/turn/steer/cancel/fork/delete; atomic accepted message plus Turn; exact replay; child-ready Fork; Store-backed events/messages/active capabilities.
- Deleted: canonical AgentSession Conversation double-create/send/fork bridge, compatibility observation/message/event projections and `extra`-based Session authority.
- Retained + reason: Conversation terminology and old domain ingress remain explicit projections/migration inputs for `UARC-033/034/051`; no new AgentSession route reads them.
- Tests: Agent Session 27, Conversation 335, API Types 534 and App boundary 6 passed; App lib check, targeted rustfmt, UARC boundary scanner and whitespace checks passed.
- Windows: verified for shared Store/API/composition behavior.
- macOS: not applicable to this shared API task; later Mac native/runtime consumers remain pending.
- Not run: full Wave 1 milestone gate waits for UARC-013/014; no UI/native/package gate was required by this non-UI task.
- Remaining/blocker: no UARC-012 blocker. Durable accepted Turns deliberately have no legacy Runtime fallback; UARC-013 is the only next task that may consume them.
- Next ready tasks: `UARC-013` only.

### 2026-09-17 UARC-013 started

- Barrier/source: UARC-012 closeout `fe44927d5bb86c94f5b55f1e8736f817d2c1f4c7`.
- Owner/write set: Integration; `nomifun-engine-core`, AI Agent `runtime_*` modules and App `engine_*` host-port adapters only.
- Changed: task claimed; Runtime family registration, selector/downcast, active-turn ownership and EngineSessionHost ports are under call-graph audit.
- Deleted: pending arbitrary family registration, Agent Runtime selector and Nomi-specific runtime handle downcast.
- Retained + reason: an internal fake/replacement Driver seam plus exact build/checkpoint identity remain required for tests and future whole-driver replacement.
- Tests: pending Driver contract, one-active-turn, cancel/cleanup and non-official-family rejection gates.
- Windows: active.
- macOS: not applicable to this shared Driver contract task; native consumers remain later platform gates.
- Not run: no UARC-013 implementation gate yet.
- Remaining/blocker: fit one official Driver to the canonical accepted Turn without reopening a legacy Conversation path. No blocker.
- Next ready tasks: none until UARC-013 barrier; then `UARC-014`.

### 2026-09-17 UARC-013 integrated and gate complete

- Barrier/source: UARC-012 closeout `fe44927d5`; implementation `5f317024d63c6845d896d379c201254101410d1b`.
- Owner/write set: Integration; AI Runtime contract/registry, App provider/typed host composition, exact startup surfaces, retired examples/tests and UARC evidence.
- Changed: one immutable `NomiRuntimeProvider`, exact build binding, internal Driver seam, typed EngineSessionHost installation, one-active-turn/cancel/cleanup/quarantine contract and pre-factory foreign-family rejection.
- Deleted: source-extension startup callbacks, public Runtime host/factory export, register/channel APIs, `nomifun.coding` second-factory install, handle downcast branch, community Engine example and multi-Runtime production acceptance target.
- Retained + reason: source-integrated implementation becomes the sole `nomifun.nomi` Driver and is enhanced by `UARC-020`; old Nomi/Coding sources are unreachable migration inputs with physical deletion owner `UARC-052`.
- Tests: AI Agent 552, Engine Core 14, App provider 2 and state 6 passed; desktop/bootstrap filters, App lib/bin/test-target checks, rustfmt, UARC boundary and whitespace checks passed.
- Windows: verified for shared provider, Driver lifecycle and composition behavior.
- macOS: not applicable to this shared Driver contract task; later Mac consumers remain pending.
- Not run: full App transition is 455/515 because 60 old Capability fixtures await UARC-014/021/022; no Runtime test failed. Full Wave 1 gate waits for UARC-014.
- Remaining/blocker: no UARC-013 blocker. Do not restore the deleted registration API or second family while migrating Compiler fixtures.
- Next ready tasks: `UARC-014` only.

### 2026-09-17 UARC-014 started

- Barrier/source: UARC-013 closeout `e7babfd127c5e29d05a18e323243ae5b9b7765eb`.
- Owner/write set: Integration; Control Plane, Kernel Compiler, Agent projection and exact contract/generated inputs identified by the call-graph audit.
- Changed: task claimed; runtime selector fields, 136-ID seeds, handwritten Nomi/Coding mappings and stale empty-grant fixtures are under reachability audit.
- Deleted: pending Runtime selector, old first-party authoring IDs and Runtime-family capability branches.
- Retained + reason: preview compile, exact Snapshot locks, action/resource policies and typed diagnostics remain security/product contracts.
- Tests: pending determinism, Snapshot-outside rejection, 136-ID retirement, preview diagnostics and Wave 1 transition gates.
- Windows: active.
- macOS: not applicable to this shared Compiler task; later platform consumers remain pending.
- Not run: no UARC-014 implementation gate yet.
- Remaining/blocker: replace stale fixtures and mappings without weakening exact Action grants. No blocker.
- Next ready tasks: none until UARC-014 and the Wave 1 barrier complete.

### 2026-09-17 UARC-014 integrated and Wave 1 gate complete

- Barrier/source: UARC-013 closeout `e7babfd127`; implementation
  `3983deff110f7e22b4eb85b6cc8ce00e9e8c4009`; milestone gate repair `8afc40c7a`.
- Owner/write set: Integration; exact Contracts/API/Compiler/Kernel/App projection inputs, generated artifacts,
  and the minimal source-HEAD gate repairs now enumerated in the manifest.
- Changed: contribution-driven Snapshot compile and middleware order, source-neutral binding projection, one fixed
  Runtime provider, exact 136-ID retirement manifest, and stable Windows milestone tests.
- Deleted: Preset/API Runtime selector and create-session binding response, open Runtime selector/catalog types,
  Coding profile authority inflation, handwritten Nomi Capability mapping, kind-driven middleware authority and
  unused Compiler compatibility parameters.
- Retained + reason: exact Snapshot/runtime build binding, preview diagnostics and existing Domain migration inputs;
  every remaining old ID/runtime/store path has a later manifest owner and no compatibility translator was added.
- Tests: Contracts 107, API 534, Kernel 60, Control Plane 48, AI Agent 545, Engine Core 14, App focused 19,
  Browser Engine 296, Computer 98, UI 3,575 and Desktop 144 passed; generator, App lib/bin/tests compile,
  `bun run check`, UARC boundary and `build:fast` passed.
- Windows: verified for Wave 1 shared contracts, UI/static milestone, Desktop test target and debug native binary.
- macOS: not applicable to UARC-014; no macOS compile/native/package evidence claimed.
- Not run: release installer signing/package and real-window visual interaction, because Wave 1 changed no product
  layout or packaging contract; those remain required at `UARC-050/060/063`.
- Remaining/blocker: no UARC-014 blocker. Full App remains 444/497 and Core workspace first stops at
  domain-wave1 9/11; all 53 App failures plus both Core failures are exact UARC-021/022 migration inputs.
- Next ready tasks: `UARC-020`, `UARC-021`, `UARC-022`; at most three Feature workers, all from the next barrier.

### 2026-09-17 Wave 2 feature lanes started

- Barrier/source: `4b65cf019e9e6942c5c3458d6db64d868c241bd2` for all three worktrees.
- Owner/write set: Feature Runtime → `UARC-020`; Feature Workspace → `UARC-021`; Feature Extensions →
  `UARC-022`. Manifest write sets are pairwise disjoint; `agent_wave2_host.rs` belongs only to UARC-021.
- Changed: three isolated branches/worktrees created; no Feature worker may edit shared contracts, generated
  artifacts, root config, status ledger or another lane's files.
- Deleted: pending each task's declared delete set; no deletion performed by task start.
- Retained + reason: Integration main checkout remains the sole merge/status/gate owner.
- Tests: workers begin without a Cargo lease; Integration will grant and serialize focused Cargo runs.
- Windows: implementation active.
- macOS: source implementation active where applicable; no Mac host verification claimed.
- Not run: Wave 2 gates wait for each bounded delivery and Integration merge.
- Remaining/blocker: no product blocker. External Mac host remains a later platform prerequisite.
- Next ready tasks: none until the three active lanes are integrated and the Wave 2 gate completes.

### 2026-09-17 UARC-020/021/022 integrated and Wave 2 gate complete

- Barrier/source: Wave 1 closeout `4b65cf019`; unified implementation `efe80298f`.
- Owner/write set: Integration merged the three bounded Feature lanes, then alone updated shared contracts,
  generated artifacts, Cargo lock and this ledger. Final dirty-path audit covered all 86 implementation paths.
- Changed: one adaptive Runtime loop and exact restart proof; four Workspace Modules with 19 Actions; frozen
  Action-derived resources; exact Store effect causation; hardened Artifact/Git owners; frozen Skill/Plugin consumers;
  per-tool MCP materialization; contribution-driven Context/Event/middleware; host-owned Turn propagation.
- Deleted: Coding process wrapper, `CapabilitiesActivated` restore path, generic MCP proxy/connect/resource tools,
  `mcp_capability_tools.rs`, unactivated `lazy_mcp.rs`, AI-factory MCP repository/OAuth injection and legacy device
  MCP Runtime entry. Retired Workspace IDs no longer derive canonical resource authority.
- Retained + reason: compaction/requirements/completion/history/continuation; physical Domain owners; frozen Skill
  locks; exact MCP resources/per-tool locks; fixed process-owned Gateway `nomi_delegate`; these are active target owners,
  not compatibility translators.
- Tests: Coding 40; Engine Core 15; Domain Wave2 17; Session 28; File all targets 378; JS Adapter 25;
  Nomi Agent all targets (lib 638); AI Agent lib 548 plus new focused 1; Plugin consumers 41; Factory integration 5;
  Kernel 61; Control Plane 49; Domain Wave4 20; App Wave2 host 29; App resource bindings 10; App unified gate
  499/499. Default and Browser/Computer feature checks, contract generator, Contracts 108, UARC boundary self-test
  and staged whitespace validation passed.
- Windows: verified for Wave 2 implementation and scoped gate.
- macOS: pending; no Mac compile/native/CEF/TCC/package evidence is claimed.
- Not run: Windows installer/native release package and UI visual gate because Wave 2 changed no renderer or packaging
  surface. Ten Robot transition tests remain assigned to `UARC-042`; the unrelated Bootstrap WAL byte-test anomaly is
  assigned to final Windows regression.
- Remaining/blocker: none for Wave 2. External Mac host remains required later.
- Next ready tasks: `UARC-030`, `UARC-031`, `UARC-032`, at most three Feature workers from the Wave 2 barrier.

### 2026-09-17 Wave 3 feature lanes started

- Barrier/source: Wave 2 closeout `67417aa08` for all three Feature lanes.
- Owner/write set: Feature Context/Data → `UARC-030`; Feature Conversation Domains → `UARC-031`; Feature
  Creation → `UARC-032`. Their manifest write sets are pairwise disjoint; Integration retains every shared contract,
  root config, generated artifact, status and merge path.
- Changed: three tasks claimed from one clean barrier; no implementation change in this status commit.
- Deleted: pending each task's delete set.
- Retained + reason: existing Domain owners and product services remain execution owners while their authoring IDs are
  replaced by product Modules/Actions.
- Tests: workers start without Cargo; Integration will serialize focused validation after delivery.
- Windows: implementation active.
- macOS: source implementation active where shared; no Mac evidence claimed.
- Not run: Wave 3 gates wait for all three bounded deliveries and Integration merge.
- Remaining/blocker: none. External Mac host remains a later platform prerequisite.
- Next ready tasks: none until Wave 3 is integrated.

### 2026-09-17 UARC-030/031/032 integrated and Wave 3 gate complete

- Barrier/source: Wave 2 closeout `67417aa08`; unified implementation
  `8454133b124a3d38636f882b098de80ce17028c0`.
- Owner/write set: Integration merged the three bounded Feature lanes, alone updated contracts/generated artifacts,
  resolved App composition seams, ran the single Cargo/UI gates and performed final review.
- Changed: four Context/Data Modules with 10 exact Actions; three conversation-domain Modules with seven Actions and
  binding-derived `BeforeTurn` scenes; four Creation/Office/Plugin Modules with 19 Actions; exact resource operations,
  real Domain owners, immutable slash discovery and finished Creative Studio states.
- Deleted: direct Web fetch Tool, local-Web binding projection test, old Wave4 App wrapper, provider/media and persistent
  attachment grants, 19 Creation fragments, and channel/persona/dialogue authoring Capability paths.
- Retained + reason: provider implementations, sensitive Knowledge/Memory operations, Channel/Companion/Customer
  business owners, Creation task routes, Canvas/asset owners and Session-scoped current-turn attachments remain target
  implementation facts, not compatibility translators.
- Tests: Wave1 4; Wave3 13; Wave4 target 6; Knowledge 329; Channel 353; Companion 275; Customer 31; Office 89;
  AI Agent 542; App focused 73 + Wave4 10 + slash HTTP 2; final UI 23. Browser feature App check, live-provider
  no-run compile, typecheck, 1,931-source desktop boundary, target inventory, contract generator, runner self-test,
  UARC scanner and whitespace checks passed. Full App probe remains 491/502 only on the 11 already assigned cases.
- Windows: verified for Wave 3 implementation, scoped runtime/domain/UI gates and 880×600 product boundary.
- macOS: pending; no Mac compile/native/CEF/TCC/package evidence is claimed.
- Not run: Windows installer/signing and macOS native/package gates, which are outside these three task surfaces.
  Commercial selected-model evidence used only StepFun Coding Plan `step-3.7-flash`: direct provider probe passed;
  canonical integrated reply remains pending the UARC-051/052 runtime-dispatch cutover and was not reported as pass.
- Remaining/blocker: none for Wave 3. External Mac host remains required later; no credential was persisted.
- Next ready tasks: `UARC-033`, `UARC-034`, `UARC-040`, at most three Feature workers from the Wave 3 barrier.
