# UARC 状态台账

> 唯一状态 owner：Integration
> 更新时间：2026-09-17
> 当前阶段：Wave 1 / UARC-011 active
> 当前 source HEAD：`877b1a751536e40a3185c31790e6e63e86c6fa32`
> UARC-000 冻结提交：`2147863da396835240296ec0a9b865200050b438`
> Wave 0 inventory 提交：`440626d91dc800af0c5b2c81cf13f63eac9abfaf`
> UARC-010 实现提交：`c059728ae4395fcdf72df59a54b4e53e8b7562a1`
> 当前主机：Windows
> Initiative 状态：`active / baseline freeze`

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
- 当前产品仍运行旧 Nomi/Coding 双 Runtime、旧 Agent Store 和旧 Capability IDs。
- 当前没有本轮源码重构的 Windows 集成证据。
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
| `UARC-011` | active | Integration | pending | n/a | canonical Agent Store baseline 与 Agent-only reset 实施中 |
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
| Control Plane transition probe | 48/50 passed | 2 old kind/direct-middleware assumptions owned by `UARC-014` |
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
- Wave 0 已收口并释放串行 `UARC-010`；并行 Feature tasks 仍须等待全部 Wave 1 barrier。
- `nomi-process-runtime --lib` 默认并行样本存在 ConPTY serial-group 挂起；已归 `UARC-021`，不阻塞
  Wave 1，但最终 gate 前必须闭合。
- UARC-010 后 Control Plane 有两项旧语义测试等待 `UARC-014`：kind-only Context 和 direct
  TurnMiddleware；这是已登记的串行迁移，不是恢复兼容的理由。

## 8. Next ready tasks

1. `UARC-011`：建立 canonical Agent Store baseline 和 Agent-only reset 合同。
2. UARC-011 barrier 后执行 `UARC-012`；不得提前启动 Feature tasks。

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
