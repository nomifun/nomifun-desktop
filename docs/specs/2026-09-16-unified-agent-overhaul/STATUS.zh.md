# UARC 状态台账

> 唯一状态 owner：Integration
> 更新时间：2026-09-18
> 当前阶段：Wave 6 / UARC-053 active
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
> Wave 4 barrier：`c5d64b943a7862bb9837686214e413899a47efc2`
> Wave 5 实现提交：`a72503995`
> UARC-051 实现提交：`ab94f33f2b6201c2850803fefbfe72430ef89234`
> UARC-052 实现提交：`48fbfb09c`
> UARC-052 barrier：`e6aca70e4`
> 当前主机：Windows
> Initiative 状态：`active / UARC-053 physical legacy removal`

## 1. 当前事实

- source HEAD 上继承的 21 个 Git 状态条目（展开为 26 个实际文件）已逐文件归属并冻结到
  `2147863da396835240296ec0a9b865200050b438`；详见
  [UARC-000 工作树冻结清单](UARC-000-WORKTREE-INVENTORY.zh.md)。
- 已完成三份目标设计和 Browser 历史文档纠正；这些是实施输入，不是生产完成证据。
- Agent Workbench 已只展示一个 Nomi Runtime 诊断面；JavaScript Runtime 保持独立设置目的地，旧 Runtime
  selector 与兼容路由已物理删除。
- Browser 已是任意 AgentSession 可授权的 `browser` Module；Guid 不再创建 Browser 专属空 Session，
  Windows managed provider 使用 WebView2，attached provider 使用安装级 Chrome 连接。
- 当前源码已经包含 macOS 独立 CEF host、原生 fixture 与部分底层验证；生产注入、产品会话闭环和
  UARC 新 Browser Resource 模型适配仍未完成。UARC-061 必须复用这套基线，不得回退到 WKWebView。
- 产品组合只安装一个 `nomifun.nomi` provider/factory；旧 Nomi loop/factory/manager、
  `nomifun.coding` family、multi-Runtime catalog/selector 与 private transcript 已物理删除。统一实现位于
  `nomifun-agent-runtime`，诊断 API 为 singular `/api/agent-runtime`。
- AgentPreset/API 已删除 Runtime selector；Kernel/Control Plane/App projection 按 contribution 编译，
  不再用 Runtime family 或 Capability ID 映射决定支持。136/136 旧 ID 已有机器可验退役路线，现存
  Domain/UI migration input 分别归 `UARC-020..053`，没有兼容翻译器。
- canonical `/api/agent-sessions`、领域 Session ports、Gateway、Robot、Remote、Creative Studio、Creation、
  Cron 与 renderer 已统一切到 generation 5 Store；产品组合不再挂载 `/api/conversations/*`。
- Wave 1 的 UARC-010/011/012/013/014 已集成；Windows 静态、UI、Desktop 与 debug native build
  milestone 已通过；当时的 `skill.hooks` Context factory transition 已由 `UARC-022` 闭合。
- Wave 2 的 UARC-020/021/022 已统一集成：一个自适应 Runtime loop、四个 Workspace Module、exact
  per-tool MCP/Plugin/Skill contribution、Store effect causation 与 Windows owner/path/process 证据均已闭合。
- Wave 3 的 UARC-030/031/032 已统一集成：Web/Knowledge/Memory、Channel/Companion/Customer Service、
  Creation/Workshop/Office/Plugin Development 均只暴露产品 Module 与 exact Actions；scene Context 从 binding
  派生，provider/model transport 细节不再成为 Agent grant。
- Wave 5 已把 Computer/Robot 收敛为单一 Module + slash Action，旧 generic Robot proxy/fixture 已删除；
  Bootstrap WAL 测试也已改为稳定的逻辑持久状态证据。App 全量 gate 为 522/522。
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
| `UARC-033` | integrated | Integration | verified | pending | AutoWork 复用 AgentExecution；Agent-path IDMM 已删除 |
| `UARC-034` | integrated | Integration | verified | pending | Schedule/Notification/Remote/SSH exact Actions 与 typed ingress |
| `UARC-040` | integrated | Integration | verified | pending | Browser Module、Resource 与 provider-neutral contract |
| `UARC-041` | integrated | Integration | verified | n/a | Windows WebView2 Browser capability UI；无 Browser-only Session |
| `UARC-042` | integrated | Integration | verified | pending | Computer/Robot 单 Module、slash Actions 与真实 availability |
| `UARC-050` | integrated | Integration | verified | pending | Official Agents、Workbench 与单 Nomi Runtime 诊断 UI |
| `UARC-051` | integrated | Integration | verified | pending | generation 5 Store/API/projection 与领域入口完成切换 |
| `UARC-052` | integrated | Integration | verified | pending | 唯一官方 Runtime；旧 Runtime/selector/compatibility 已物理删除 |
| `UARC-053` | active | Integration | pending | pending | 删除旧 capability/store/IDMM/Browser entry 与 obsolete assets |
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
| Wave 4 barrier | `c5d64b943a7862bb9837686214e413899a47efc2` | UARC-033/034/040 统一 gate 后的干净施工源 |
| UARC-041 Browser | Platform 55 + App 15 + UI 79 passed；native smoke passed | WebView2、exact Action controls、managed/attached 状态与无 Browser-only Session |
| UARC-042 Computer/Robot | Computer 103 passed / 7 explicit ignores；Robot 150 + fake-device 4；Domain 17 + 6 | 单 Module/slash Actions、typed observations、effect receipt、真实设备状态 |
| UARC-050 Workbench | focused UI 280 passed；最终 affected 30 passed；880×600 visual passed | official exact defaults、Server catalog、single Runtime、键盘/焦点/完整状态 |
| Wave 5 Contracts/Control Plane/Session | 108 + 48 + 35 passed | exact catalog/defaults、session-scoped resource binding 与 frozen selections |
| Wave 5 App | lib 522 + route-gap 28 + official preset 3 passed | App composition、canonical admission 与完整 transition regression |
| Wave 5 DB | Agent Store reset/schema 3 passed | 迁移后 schema 与 clean baseline 相等；跨 Session binding 可复用 |
| Wave 5 UI/build/contract boundary | `bun run check` + `build:ui` + generator write/check + UARC boundary passed | 880×600 boundary、i18n、typecheck、生成物与删除边界一致 |
| UARC-051 App/route | App lib 512 + route-gap 29 passed | canonical create/list/update/turn/rebuild/delete；retired routes 404 |
| UARC-051 Store/domain/DB | AgentSession 35 + Cron 187 + reset 3 + ID schema 20 + Remote repo 1 passed | Session ports、projection rebuild、Agent-only reset 与 Remote owner |
| UARC-051 UI | focused 112 passed；typecheck、production build、880×600 boundary passed | immutable edit retry、canonical history/search/creation、无 legacy artifact/writeback surface |
| Commercial selected-model integration | StepFun Coding Plan `step-3.7-flash` canonical smoke passed | credential-isolated Session → Runtime → projection；未持久化、打印或提交密钥 |
| UARC-052 Runtime/AI/Conversation | Runtime 40 + AI 452 + Plugin consumer 23 + Conversation 335 passed | 单一 Runtime loop、固定 factory、提取 adapters 与无 private transcript |
| UARC-052 App/process | App lib 511 + route-gap 29 + Process architecture 16 passed | single-factory boot、canonical Session dispatch 与单 process owner |
| UARC-052 UI/build/boundary | focused UI 7；typecheck、production build、880×600、Browser/Process/UARC scanners passed | Runtime selector/compatibility reachability为 0；migration 099 的 2 个 immutable 文本归 UARC-054 |
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
- `UARC-051/052` Windows/shared cutover 已收口；产品入口均使用 generation 5 Store 与唯一官方 Runtime，
  旧 Conversation projection、旧 Runtime 实现、multi-Runtime selector 与 private transcript 不再可达。
- 历史 Agent schema/migrations、旧 capability projection、obsolete product smoke/IDMM/Browser-entry leftovers 与
  Fresh-v4 root aliases 仅为接下来的 `UARC-053/054` 物理删除输入。

## 8. Next ready tasks

1. `UARC-053`：active；串行删除旧 capability/store/IDMM/Browser entry 及无 owner 的测试、脚本、文案与样式。
2. `UARC-061/062`：依赖外部 Mac 真机，在 Windows 串行主线完成且 handoff 就绪后执行。

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

### 2026-09-17 Wave 4 feature lanes started

- Barrier/source: Wave 3 closeout `5bec65d182d5798d5fa99398a39ae59a218bbba1` for all three Feature lanes.
- Owner/write set: Feature Automation Core → `UARC-033`; Feature Platform Services → `UARC-034`; Feature Browser
  Shared → `UARC-040`. Manifest write sets are pairwise disjoint; Integration retains shared contracts, App composition,
  UI paths outside declared sets, generated artifacts, root config, status and final merge.
- Changed: three tasks claimed from one clean barrier; no implementation change in this status commit.
- Deleted: pending each task's declared delete set.
- Retained + reason: business queues/effects, transport owners and Browser engine/providers remain target implementation
  owners while Agent-facing fragments and Conversation-only identity are replaced.
- Tests: workers start without a Cargo or full-UI lease; Integration will serialize all focused and wave gates.
- Windows: implementation active.
- macOS: shared source implementation active where applicable; no Mac host verification claimed.
- Not run: Wave 4 gates wait for all three bounded deliveries and Integration merge.
- Remaining/blocker: none. External Mac host remains a later platform prerequisite.
- Next ready tasks: none until Wave 4 is integrated.

### 2026-09-18 UARC-033/034/040 integrated and Wave 4 gate complete

- Barrier/source: Wave 3 closeout `5bec65d182d5798d5fa99398a39ae59a218bbba1`; unified implementation
  `26e50f4f1f962c3cf9996fb87f51aa9955a920f9`.
- Owner/write set: Integration merged the three bounded Feature lanes, alone reconciled App composition, shared
  contracts/generated artifacts, Agent Store schema/migration, root Cargo files, status and the final gate.
- Changed: AutoWork now delegates every execution to AgentExecution and persists one canonical owner-scoped config;
  deletion is a fenced process-owned saga with durable non-private manual-override audit; Schedule/SSH expose exact
  Actions with retained teardown evidence; Browser uses one provider-neutral Module/Resource model for managed and
  attached providers; Windows Desktop injects the canonical persistent profile owner.
- Deleted: Agent-path IDMM crate/routes/UI/i18n, AutoWork Conversation/Terminal dispatch and receipt state machine,
  fragmented Schedule/Requirement/SSH Gateway capabilities, old SystemBrowser/BrowserWorkspace implementation and
  controls, and Conversation supervision/turn-scope/failover paths with no remaining owner.
- Retained + reason: Requirements business facts and queue policy, AgentExecution DAG/Attempt, typed remote ingress,
  native transport/resource owners, Browser engine/WebView2 and independent macOS CEF child NSView. The latter is a
  platform implementation input for `UARC-061`, not a WKWebView or compatibility path.
- Tests: Agent Session 34; Requirement 60; AgentExecution 103; Cron 197 + integration 64; DB Cron 43, Agent reset 3,
  ID schema 20 and published migration 5; Public 27; SSH 40; Browser Platform 55; Browser Engine 293 with 9 explicit
  real-Chrome ignores; App Browser 15, Wave2 host 30, Session boundary 11 and Requirements e2e 10; Gateway 107;
  Contracts 108. Contract generator write/check, target inventory, UARC scanner, `bun run check`, Desktop all-targets
  check and debug build passed. Full App probe is 520/530: the exact 10 failures are Robot transition inputs owned by
  `UARC-042`, with no Wave 4 regression.
- Windows: verified for shared contracts, Store/migrations, service composition, Browser WebView2 resource/profile
  lifecycle, 880×600 UI boundary and debug Desktop build.
- macOS: pending; shared source and the existing independent CEF child NSView are not Mac compile/native/package
  evidence. CEF integration, TCC, lifecycle, arm64 app/DMG and signing structure remain mandatory on a Mac host.
- Not run: 18 real-sshd `pool_lifecycle` cases self-skipped because this Windows host has no sshd/ssh-keygen; only the
  non-sshd harness case executed. No native macOS, DMG or signing claim was made.
- Commercial-model evidence: only StepFun Coding Plan `step-3.7-flash` was selected; the direct provider probe returned
  HTTP 200. Canonical integrated dispatch remains pending `UARC-051/052`, so its timeout is not reported as success.
  No API credential was persisted, logged into repository artifacts or committed.
- Remaining/blocker: none for Wave 4. Cron's bounded 4096 in-process receipt ledger remains fail-closed at capacity;
  external Mac hardware is a later platform prerequisite, not a Windows Wave 4 blocker.
- Next ready tasks: `UARC-041`, `UARC-042`, `UARC-050` as pairwise-disjoint Windows/shared Feature lanes; `UARC-061`
  remains pending a Mac host and must not be claimed from Windows.

### 2026-09-18 Wave 5 Windows/shared feature lanes started

- Barrier/source: Wave 4 closeout `c5d64b943a7862bb9837686214e413899a47efc2` for all three Feature lanes.
- Owner/write set: Feature Browser Windows UI → `UARC-041`; Feature Devices → `UARC-042`; Feature Agent UI →
  `UARC-050`. Manifest write sets are pairwise disjoint; Integration alone owns shared contracts, generated artifacts,
  root configuration, status, final merge and all broad/native/UI gates.
- Changed: three bounded tasks claimed from the clean Wave 4 barrier; no feature implementation is recorded in this
  status commit.
- Deleted: pending each task's declared delete set; no macOS CEF source or WKWebView substitute is authorized here.
- Retained + reason: Windows WebView2, Computer/Robot physical owners and exact Agent compile preview remain the target
  production foundations while their fragmented entry points and implementation vocabulary are replaced.
- Tests: workers begin without a Cargo/full-UI/native-build lease. Integration will serialize focused checks, 880×600
  visual acceptance, full typecheck/UI boundary and the Wave gate after all deliveries.
- Windows: implementation active.
- macOS: shared UI/source changes may be prepared, but no Mac verification is claimed; `UARC-061/062` remain unclaimed.
- Not run: Wave 5 gates wait for the three bounded deliveries and Integration merge.
- Remaining/blocker: none for the Windows/shared lanes. External Mac hardware remains a later platform prerequisite.
- Next ready tasks: none until `UARC-041/042/050` integrate; then `UARC-051` is the next serial integration task.

### 2026-09-18 UARC-050 exact Module/Action catalog transport granted

- Barrier/source: Wave 5 start `90ba8f95f`; task dependencies and lane topology are unchanged.
- Owner/write set: Integration adds only `nomifun-api-types/src/agent_platform.rs` and the matching renderer contract
  type to `UARC-050`; the Feature worker still owns its original UI/catalog files and may not edit other shared inputs.
- Changed: the already-defined strict `CapabilityModuleCatalogItemDto` will be carried as `AgentCatalogResponse.modules`
  from canonical manifests instead of duplicating first-party Action IDs in the renderer.
- Deleted: the proposed renderer-local Module/Action registry is rejected before implementation.
- Retained + reason: server preview/save remains the sole compile authority; plugin/unknown exact grants remain opaque
  and cannot be widened by the UI.
- Tests: pending API serialization, Control Plane catalog and focused Workbench fixtures in the Wave 5 gate.
- Windows/macOS: shared transport only; no native verification claim changes.
- Remaining/blocker: transport gap resolved by this bounded grant; no product decision or external input is required.

### 2026-09-18 UARC-042 Robot composition/test cutover granted

- Barrier/source: Wave 5 start `90ba8f95f`; task dependencies and lane topology are unchanged.
- Owner/write set: Integration adds only `nomifun-app/src/robot_wiring.rs` and its test module to `UARC-042`; final
  App composition cleanup remains Integration-owned after the Feature worker delivers the Robot/Computer owners.
- Changed: the 10 preassigned App failures are now explicitly writable so their legacy Conversation + generic
  device-MCP harness can be physically replaced by canonical Session + exact Robot Action acceptance.
- Deleted: no `mcp_connect`/`mcp_tool_proxy` or capability-string compatibility path may be retained to satisfy tests.
- Retained + reason: real Robot registry/link/physical protocol doubles remain valid evidence behind exact Actions.
- Tests: pending focused Robot owner tests and the 10-case App transition suite in the unified Wave gate.
- Windows: implementation scope expanded only to the already-owned App Robot composition seam.
- macOS: no native claim; `UARC-062` remains the Mac TCC/lifecycle owner.
- Remaining/blocker: write-set gap resolved; no user decision is required.

### 2026-09-18 UARC-041 exact Browser UI authority seams granted

- Barrier/source: Wave 5 start `90ba8f95f`; Integration review found three pre-gate correctness gaps in the completed
  renderer lane.
- Owner/write set: Integration adds the exact Browser snapshot builders and focused Rust/App tests to `UARC-041`;
  renderer/ChatLayout files were already in the task write set.
- Changed: Browser snapshots must expose the Session's exact Action allowlist so human controls can be disabled before
  dispatch; attached Chrome is informational until an existing target is explicitly projected; entering the 880×600
  focus layout must move focus out of the hidden chat surface.
- Deleted: optimistic all-controls authority, attached-provider `userReady` claims and hidden-element focus retention.
- Retained + reason: backend Action admission remains authoritative and still rejects forged/stale commands; the UI
  projection only prevents guaranteed failures and explains unavailable controls.
- Tests: pending subset-grant control matrix, attached zero-command case and ChatLayout open/resize focus handoff.
- Windows: fixes are required before WebView2 native/visual acceptance.
- macOS: snapshot contract is shared; CEF native behavior remains `UARC-061`.
- Remaining/blocker: review findings are fully specified and require no product decision.

### 2026-09-18 Wave 5 exact defaults and resource-status seams granted

- Barrier/source: Wave 5 start `90ba8f95f`; completed UI/device lanes exposed two final cross-surface contracts.
- Owner/write set: Integration adds official-template draft materialization to `UARC-050`, and Computer automatic
  resource plus Robot resource-status picker files to `UARC-042`. Agent contracts/generated outputs remain globally
  integration-only.
- Changed: official seeds can carry exact Action allowlists without client-side risk inference; `computer` resolves only
  to server-owned `local-desktop`; Robot choices can present live connection/permission/missing-hardware guidance.
- Deleted: expand-all-template behavior and an unresolvable Computer resource requirement.
- Retained + reason: server compile remains authoritative, while UI resource choices carry identities only.
- Tests: pending official seed contract/compiler/UI tests and Computer/Robot resource selection interactions.
- Windows: required for Wave 5 acceptance; macOS native availability remains `UARC-062`.
- Remaining/blocker: write sets are complete; implementation remains with Integration.

### 2026-09-18 UARC-042 hosted-effect rejection seam granted

- Barrier/source: Wave 5 start `90ba8f95f`; the completed owner exposed one final receipt-classification call site.
- Owner/write set: Integration adds only `router/hosted_effect_receipts.rs` to `UARC-042`.
- Changed: pre-dispatch permission, missing Action and revoked-Session rejections can settle as rejected facts rather
  than being quarantined as an unknown physical outcome.
- Deleted: no compatibility or retry inference; only the exact canonical rejection codes are admitted.
- Retained + reason: dispatch-started physical effects still require returned/failed/unknown terminal proof.
- Tests: pending hosted-effect receipt and Robot canonical Session gates.
- Windows/macOS: shared receipt semantics only; native verification status is unchanged.
- Remaining/blocker: final call-site grant resolved.

### 2026-09-18 UARC-042 canonical owner integration seams granted

- Barrier/source: Wave 5 start `90ba8f95f`; the bounded Robot/Computer owner implementation exposed the exact
  remaining production call graph before Integration began edits.
- Owner/write set: Integration adds Domain Wave2/Wave4 manifests and seven exact App Kernel/resource/composition files
  to `UARC-042`; Feature ownership remains confined to Computer, Robot and `*robot*` route sources.
- Changed: the unified Runtime can now be wired to one `computer` Module and one `robot` Module with slash Action
  policies instead of compiling old dotted Capability fragments around the new owners.
- Deleted: old Robot lifecycle grants (`robot.link`, `robot.audio`) and Computer/Robot fragment dispatch must be removed
  at these call sites, not hidden behind aliases.
- Retained + reason: source-integrated Engine plans, hosted-effect receipts, typed resource bindings and the physical
  Robot/Computer owners remain the single runtime path.
- Tests: pending Domain Wave2/Wave4 contracts, App Kernel/session boundary and Robot transition gates.
- Windows: integration implementation authorized; native input/permission evidence remains part of the unified gate.
- macOS: no native claim; shared manifest changes become `UARC-062` inputs.
- Remaining/blocker: call-graph write-set gap resolved; `coding_runtime_host.rs` stays untouched because its old
  Runtime path is physically owned by `UARC-052`, not a compatibility target for this task.

### 2026-09-18 UARC-041 Browser copy/navigation assertions granted

- Barrier/source: Wave 5 start `90ba8f95f`; Browser implementation completed its in-scope focused suite before the
  final integration-owned copy/navigation seams were identified.
- Owner/write set: Integration adds the exact Sider structure test and `browserWorkspace.json` locale pair to
  `UARC-041`; generated i18n keys remain integration-only as already declared globally.
- Changed: product copy and structural assertions can now name the generic current-Session Browser capability and its
  managed/attached providers instead of the deleted Conversation Browser entry.
- Deleted: stale `chat-browser-toggle`/`BrowserWorkspacePanel` assertions and “Conversation browser” copy.
- Retained + reason: the tool-rail Browser button, accessible in-panel menu and native WebView2 surface are the new
  interaction contract.
- Tests: Feature-focused 114/114 passed; Integration will rerun i18n generation, Sider structure and full UI boundary.
- Windows: renderer implementation complete pending integrated native/visual gate.
- macOS: shared copy only; CEF native evidence remains `UARC-061`.
- Remaining/blocker: write-set gap resolved; no user decision is required.

### 2026-09-18 UARC-041/042/050 integrated and Wave 5 gate complete

- Barrier/source: Wave 4 closeout `c5d64b943a7862bb9837686214e413899a47efc2`; Wave 5 start
  `90ba8f95f`; unified implementation `a72503995`.
- Owner/write set: Integration merged the three bounded Feature lanes and alone reconciled shared contracts,
  generated artifacts, App composition, Store schema/migration, root Cargo files, status and all unified gates.
- Changed: Browser is a generic current-Session capability with exact controls and WebView2/attached-Chrome status;
  Computer and Robot each expose one Module with slash Actions, typed observations/resources and effect receipts;
  Workbench consumes the server Module catalog and exact official defaults, shows complete resource/permission states,
  and exposes one Nomi Runtime diagnostics surface. Product Agent binding now freezes required typed resources, while
  Agent resource binding identity is scoped by Session so two Sessions can safely bind the same product resource.
- Deleted: Guid Browser-only launch/session paths, `BrowserWorkspacePanel`, the Runtime selector and compatibility
  route, generic Robot proxy/legacy transition fixture, fragmented Robot/Computer grants and client-inferred template
  expansion. No WKWebView implementation was added or restored.
- Retained + reason: Windows WebView2 and attached Chrome owners, existing independent macOS CEF child NSView,
  native Computer/Robot physical owners, canonical Store/Session/compiler and JavaScript Runtime's separate settings
  destination remain production inputs with explicit later platform owners.
- Tests: Browser Platform 55, App Browser 15 and Browser UI 79 passed; Windows native Browser smoke emitted
  `BROWSER_WORKSPACE_SMOKE_PASS`. Computer 103 passed with 7 explicit real-environment ignores whose cursor/screenshot
  cases were run separately; Robot 150 plus fake-device 4 and Domain Wave2/Wave4 17 + 6 passed. Focused Workbench/UI
  was 280/280, then 30/30 after final visual fixes. Contracts 108, Control Plane 48, Session 35, DB reset/schema 3,
  App lib 522, route-gap 28 and official preset 3 passed. Contract generator write/check, UARC boundary,
  `bun run check`, `bun run build:ui` and `git diff --check` passed.
- Windows: verified for Browser native/resource lifecycle, Computer/Robot shared/native contracts, Workbench behavior,
  keyboard/focus states and a real 880×600 Desktop-class viewport. Normal-width visual inspection also passed.
- macOS: `UARC-041` is not applicable; shared sources for `UARC-042/050` remain pending. `UARC-061/062` still require
  a Mac for CEF child-NSView production injection, native interaction/TCC/lifecycle, visual QA, arm64 app/DMG and
  signing-structure evidence.
- Commercial-model evidence at Wave 5 close: provider reachability only; the later canonical integration result is
  recorded in the `UARC-051 integrated` entry below.
- Remaining/blocker at Wave 5 close: Remote open/delete Store split assigned to `UARC-051`; it is closed below.
- Next task at that barrier: `UARC-051`, serial Integration only.

### 2026-09-18 UARC-051 started

- Barrier/source: Wave 5 closeout `d45c6f54d5e4da5ef39ca2d37a052b23469bc15e`.
- Owner/write set: Integration only; App sources, DB, Conversation store boundary, renderer adapters and Conversation
  surfaces exactly as declared by the manifest. No Feature worker or overlapping Cargo/full-UI lease is active.
- Changed: task state claimed from the clean Wave 5 barrier; implementation begins with a writer/reader/projection
  call-graph audit and the known Remote open/delete split.
- Deleted: pending legacy Agent Store writers/readers, old API bridges and old Session projections; no compatibility
  layer or accepted canonical DELETE 404 is authorized.
- Retained + reason: non-Agent configuration tables and the generation 5 canonical Session API are the only retained
  persistence/API foundations.
- Tests: targeted Store/API/projection tests will precede one serialized Wave 6 gate. Commercial provider dispatch
  will use the requested `step-3.7-flash` selection only after the canonical path is connected.
- Windows: implementation active.
- macOS: shared cutover source is active; no Mac-native evidence is claimed from this host.
- Remaining/blocker: none at start. External Mac hardware remains a later platform prerequisite.
- Next ready tasks: none until `UARC-051` completion; `UARC-052` depends on this cutover.

### 2026-09-18 UARC-051 integrated

- Barrier/source: Wave 5 closeout `d45c6f54d5e4da5ef39ca2d37a052b23469bc15e`; implementation
  `ab94f33f2b6201c2850803fefbfe72430ef89234`.
- Owner/write set: Integration only; manifest write set was expanded to the exact domain adapters, generated
  contracts, UI history surface and credential-isolating runner required by the cutover. No Feature worker ran.
- Changed: generation 5 Store now owns create/list/metadata/Turn/steer/cancel/history/search/rebuild/delete;
  Remote, Cron, Channel, Companion, AgentExecution, Gateway, Robot, Creative Studio, Creation and Terminal bindings
  all resolve the same canonical Session. Runtime progress, tool results and assistant text project from Session events.
- Deleted: product mounting of `/api/conversations/*`; legacy Gateway port, boot Conversation reconciliation,
  delivery-notify observer, Cron artifact/skill-suggest projections, renderer Conversation artifact cards and
  knowledge-writeback retry/stream bridges. Edit/retry now creates a new immutable Turn.
- Retained + reason: non-Agent Provider/model, Cron definition, product configuration and Creation task stores remain
  their domain authorities. The old Agent schema/migrations and old Runtime source remain only for `UARC-052..054`
  physical deletion and are not reachable through the product Session/API composition.
- Tests: App lib 512/512; route-gap 29/29 including canonical create/list/update/Turn/projection rebuild/delete and
  retired-route 404; AgentSession 35/35; Cron 187/187; DB Agent reset 3/3, ID schema 20/20, Remote repo 1/1;
  focused UI 112/112; typecheck, production UI build, 880×600 boundary, rustfmt/diff check and contract generator
  check passed. UARC boundary scanner passed with the remaining Runtime/schema matches assigned to `UARC-052..054`.
- Commercial-model evidence: credential-isolated StepFun Coding Plan `step-3.7-flash` selected-model integration
  passed through canonical Session → unified Runtime → durable assistant projection. The credential did not enter
  argv, Cargo/build-script environments, repository files or logs, and the runner's plaintext audit passed.
- Windows: verified. macOS: shared source pending; no Mac-native, CEF, TCC, DMG or signing evidence claimed here.
- Remaining/blocker: none for `UARC-051`. External Mac hardware remains a later Wave 8 prerequisite.
- Next ready task: `UARC-052`, serial Integration only.

### 2026-09-18 UARC-052 started

- Barrier/source: UARC-051 closeout `9c338f5f67de37d3fffaff6f6397d6078e2667f9`.
- Owner/write set: Integration only; `nomifun-ai-agent`, old `nomi-agent`, `nomifun-coding-engine` and
  `runtime_engines.rs` exactly as declared by the manifest. No Feature worker or overlapping gate is active.
- Changed: task claimed after canonical Store/API cutover; implementation begins from production reachability and
  Cargo-graph evidence, extracting only reusable domain adapters required by the unified driver.
- Deleted: pending old Nomi loop/factory/manager, `nomifun.coding` family, multi-Runtime catalog/selector,
  `uses_nomi_session`/restart compatibility and duplicate Runtime tests.
- Retained + reason: the unified Runtime crate, one internal official driver and reusable domain adapters only.
- Tests: focused Runtime/Engine/AI checks precede a single serialized App/Cargo/reachability gate.
- Windows: implementation active. macOS: shared source active; no Mac-native evidence claimed on this host.
- Remaining/blocker: none. External Mac hardware remains a later Wave 8 prerequisite.
- Next ready tasks: none until `UARC-052` completes; `UARC-053` depends on it.

### 2026-09-18 UARC-052 integrated

- Barrier/source: UARC-051 closeout `9c338f5f67de37d3fffaff6f6397d6078e2667f9`; implementation
  `48fbfb09c`.
- Owner/write set: Integration only. The manifest write set was expanded to the exact public lifecycle consumers,
  migration fence, diagnostics API/UI, boundary scripts and tests required by physical deletion; no Feature worker ran.
- Changed: `nomifun-coding-engine` became `nomifun-agent-runtime`; one `OfficialRuntimeProvider`, fixed factory,
  `OfficialRuntimeHost` and singular `/api/agent-runtime` now form the only production execution composition. Public
  types use Agent Runtime/build terminology, and Session lifecycle opens the official runtime directly.
- Deleted: 63,887 lines including `nomi-agent`, `nomi-cli`, old Nomi factory/manager/private transcript,
  `nomifun.coding`, multi-Runtime catalog/selector/registration, `uses_nomi_session` recovery branches, task-local
  plugin runtime wrappers and duplicate legacy tests.
- Retained + reason: reusable domain adapters, middleware/output contracts, process registry and the unified adaptive
  Driver remain under owned crates. Published migration 099 is checksum-stable; migration 112 transitions installed
  databases to `runtime_build_binding`, while UARC-054 owns baseline squash of its two historical selector literals.
- Tests: Runtime 40/40; AI lib 452/452 with one explicit real-Chrome/network ignore; Plugin consumer 23/23;
  Conversation 335/335; DB ID schema 20/20; App lib 511/511; route-gap 29/29; Process architecture 16/16; focused UI
  7/7. Typecheck, production UI build, 880×600 boundary, UARC/Browser/Process scanners and self-tests, rustfmt and
  diff check passed.
- Commercial-model evidence: Windows Credential Manager-isolated StepFun Coding Plan `step-3.7-flash`
  selected-model smoke passed through canonical Agent Session → unified Runtime → durable projection. The credential
  did not enter argv, Cargo/build-script environments, repository files or logs.
- Windows: verified. macOS: shared source pending; no CEF, TCC, native interaction, arm64 app, DMG or signing claim.
- Not run/deferred: the ignored legacy product-chain live fixture still targets retired `/api/conversations/*` and was
  confirmed stale at `guid.warmup`; its physical removal/replacement is explicitly owned by UARC-053's obsolete
  tests/scripts delete set and is not a second Runtime path.
- Remaining/blocker: none for UARC-052. External Mac hardware remains a later Wave 8 prerequisite.
- Next ready task: `UARC-053`, serial Integration only.

### 2026-09-18 UARC-053 started

- Barrier/source: UARC-052 closeout `e6aca70e4`.
- Owner/write set: Integration only; the manifest grants the cross-repository delete pass over `crates/**`,
  `ui/src/**`, `scripts/**` and `docs/**`. Shared contracts, root configuration, generated outputs and final gate remain
  Integration-owned; no Feature worker or overlapping gate is active.
- Changed: task state claimed from a clean barrier. Work begins from the machine inventory's remaining capability,
  Store, automation/IDMM and Browser-entry groups, plus the known obsolete product-chain live fixture.
- Deleted: pending old capability IDs/projections, legacy Agent tables/files, Agent IDMM, session-browser entry and
  obsolete tests/scripts/docs/i18n/CSS; no compatibility alias or unreachable wrapper is authorized.
- Retained + reason: only canonical Module/Action contracts, generation 5 Store, AgentExecution, generic Browser
  Resource/Provider model and product-owned non-Agent stores may remain.
- Tests: focused reachability/dead-asset checks will precede one serialized App/UI/UARC gate. Commercial provider
  validation continues to use only the requested StepFun Coding Plan `step-3.7-flash` selected-model path.
- Windows: implementation active. macOS: shared deletion source active; no Mac-native evidence claimed on this host.
- Remaining/blocker: none. External Mac hardware remains a later Wave 8 prerequisite.
- Next ready tasks: none until UARC-053 completes; UARC-054 depends on it.
