# UARC-001 Reachability、存储、耗时与平台基线

> 日期：2026-09-17
> UARC-000 closeout：`6b09b4094e36c722bb14544e9671e340baafd48c`
> 机器可读真相：[`UARC-001-INVENTORY.json`](UARC-001-INVENTORY.json)
> 扫描器：`scripts/check-uarc-boundary.mjs`

## 1. 扫描合同

扫描器从 Git tracked/untracked 文件集合读取 `apps`、`crates`、`ui/src/common` 和
`ui/src/renderer`，排除 tests、examples、fixtures、snapshots、历史合同和生成合同。当前扫描
3,029 个生产文件。13 组旧架构入口均绑定 manifest 中的 owner task；普通 gate 允许引用数量收缩，
禁止数量增长或在数量不变时把旧引用横移到另一组文件。最终 `--completion` 要求所有组归零。

扫描结果是 reachability inventory，不是“这些字符串全部等价”的断言。每组 matcher 都刻意围绕
明确的旧入口、旧表、旧文件或旧 ID；后续 owner 修改 matcher 或 baseline 时必须由 Integration
审查，不能通过扩大排除项让旧路径消失。

## 2. 当前生产旧路径

| Group | Matches | Files | Owner tasks |
| --- | ---: | ---: | --- |
| `runtime.multi_family_literals` | 27 | 10 | `UARC-013/020/052` |
| `runtime.preset_selector` | 45 | 24 | `UARC-014/050/052` |
| `runtime.old_nomi_loop_paths` | 62 | 62 | `UARC-013/052` |
| `runtime.multi_runtime_infrastructure_paths` | 9 | 9 | `UARC-013/020/052` |
| `runtime.compatibility_branches` | 28 | 9 | `UARC-013/052` |
| `capability.legacy_projection_paths` | 4 | 4 | `UARC-010/014/053` |
| `capability.legacy_authoring_ids` | 2,595 | 166 | capability/domain tasks + `UARC-053` |
| `store.legacy_agent_tables` | 548 | 51 | `UARC-011/012/051/054` |
| `store.private_nomi_transcript` | 44 | 8 | `UARC-011/052/054` |
| `store.fresh_v4_parallel_root_and_schema` | 470 | 9 | `UARC-011/012/054` |
| `automation.agent_idmm_surface` | 20 | 20 | `UARC-033/053` |
| `automation.autowork_parallel_receipts` | 202 | 24 | `UARC-033/051` |
| `browser.dedicated_session_entry` | 40 | 13 | `UARC-040/041/053` |

合计 4,094 个文本或路径命中。`capability.legacy_authoring_ids` 的源 catalog 精确包含 136 个旧 ID；
扫描器同时冻结 catalog item count、生产命中数量和文件集合摘要。

## 3. 存储 owner 收敛图

| 当前事实链 | 当前事实 | 目标 owner / facts | Tasks |
| --- | --- | --- | --- |
| Agent Session | `conversations`、`messages`、delivery receipt、runtime events | canonical Agent Store：`agent_sessions/turns/events/messages/payloads` | `UARC-011/012/051` |
| Agent Effect | MCP/hosted/Git/domain-specific receipts | `agent_effects` + Domain owner 结果语义 | `UARC-011/051/054` |
| Runtime checkpoint | `nomi-sessions` transcript/index + Fresh-v4 root | Unified Runtime derived checkpoints + canonical DB cursor | `UARC-011/020/052/054` |
| Execution automation | AgentExecution attempts + AutoWork attempts/receipt polling + IDMM | AgentExecution attempt + canonical Agent Turn receipt | `UARC-012/033/051` |

新 `nomifun-agent-session` crate 本身不是删除目标；目标是删除它当前的 Fresh-v4 命名、
`session_events/session_payloads` schema 和并行 root 假设，并让它成为唯一 canonical Agent Store。

## 4. Windows 命令耗时样本

这些是同一 Windows checkout 上的单次工程预算样本，不是性能承诺。Cargo 样本按 test lease 串行运行。

| Command | Wall time | Result |
| --- | ---: | --- |
| `bun run typecheck` | 17.842 s | passed |
| UARC-000 三个设置/导航测试 | 2.379 s | 9 passed |
| `bun run check:i18n` | 0.275 s | passed |
| `bun run check:desktop-ui-boundary` | 0.333 s | passed |
| `bun run check` | 45.319 s | passed |
| UARC boundary self-test | 2.591 s warm sample | passed |
| `cargo test -p nomifun-agent-contracts --lib` | 32.550 s | 101 passed |
| `cargo test -p nomifun-agent-kernel --lib` | 26.235 s | 58 passed |
| `cargo test -p nomifun-agent-session --lib` | 21.488 s | 25 passed |
| `cargo test -p nomifun-engine-core --lib` | 19.303 s | 14 passed |
| `cargo test -p nomifun-browser-platform --lib` | 8.724 s | 50 passed |
| `cargo test -p nomi-process-runtime --lib -- --test-threads=1` | 3.748 s warm | 119 passed |
| `cargo test -p nomifun-terminal --lib -- --test-threads=1` | 93.162 s | 134 passed |

### Process Runtime 并行基线异常

默认 `cargo test -p nomi-process-runtime --lib` 完成编译和大部分测试后，5 个共享
`serial(conpty_close_executor)` 用例均被报告运行超过 60 秒，随后数分钟无进展；约 210 秒后由
Integration 中止。单独 panic/quarantine 用例通过；同一 119 项库测试在 `--test-threads=1` 下
3.30 秒全部通过。

该异常归 `UARC-021`。后续必须让默认并行 crate gate 稳定，或明确固化有工程理由的串行 gate；不能
通过连续重跑掩盖。它不阻止 UARC-001 inventory 完成，但会让最终 completion gate 保持失败，直到
inventory 中该 anomaly 标为 `closed` 并附修复证据。

## 5. macOS 缺口

当前没有活跃 Mac 主机，以下状态全部为 `pending`：

- `UARC-061`：生产 CEF Browser Resource 注入、Retina/focus/物理 IME、popup/dialog/permission、
  用户 picker、上传下载、crash/teardown/helper/Command-Q，以及 framework/helper 签名结构。
- `UARC-062`：process group/generation、PTY/UTF-8/TUI/IME、TCC denied/granted、Retina 输入坐标和
  Command-Q 无孤儿进程。
- `UARC-063`：最终 source 上的 shared gates、完整 880×600/宽窗口产品矩阵、arm64 `.app`/DMG、
  原生 Browser/Computer/Terminal 生命周期；有凭据时才验证 Developer ID/notarization。

现有 macOS CEF native fixture 仍是有价值的底层证据，但不改变上述 pending 状态。

## 6. Gate 用法

```text
bun scripts/check-uarc-boundary.mjs --self-test
  校验 scanner、manifest task ownership、inventory schema 和当前不增长基线

bun scripts/check-uarc-boundary.mjs --json
  输出含逐文件逐行命中的机器可读当前报告

bun scripts/check-uarc-boundary.mjs --completion
  最终门禁；当前应因 13 组旧引用、1 个基线异常和 3 个 macOS 缺口而失败
```

后续 Wave 只能通过真实删除/重构降低命中，并由 Integration 更新 baseline；不得把生产路径塞进
排除项、把旧 ID 改成 alias，或保留无 owner 的 compatibility wrapper。
