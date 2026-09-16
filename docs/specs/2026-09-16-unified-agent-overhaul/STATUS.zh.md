# UARC 状态台账

> 唯一状态 owner：Integration
> 更新时间：2026-09-17
> 当前阶段：Wave 0 / UARC-000 active
> 当前 source HEAD：`877b1a751536e40a3185c31790e6e63e86c6fa32`
> 当前主机：Windows
> Initiative 状态：`active / baseline freeze`

## 1. 当前事实

- source HEAD 上继承的 dirty worktree 有 21 个 Git 状态条目，展开目录后为 26 个实际文件；已逐文件归属，详见
  [UARC-000 工作树冻结清单](UARC-000-WORKTREE-INVENTORY.zh.md)。尚未形成 UARC barrier commit。
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
| `UARC-000` | active | Integration | pending | n/a | 26 个继承文件已逐项归属；基线检查进行中 |
| `UARC-001` | planned | Integration | pending | pending | 等待 UARC-000 |
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
- 当前 dirty worktree 尚未收口，禁止启动 Wave 1 或并行 Feature tasks。

## 8. Next ready tasks

1. `UARC-000`：完成最终基线检查、审查 staged diff 并生成 barrier。
2. `UARC-001`：在 barrier 上生成 reachability/test/platform inventory。
3. 完成 Wave 0 后串行执行 `UARC-010`。

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
