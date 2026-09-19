# UARC macOS 实施与验证移交

## 0. 冻结坐标

```text
source branch:                 rf/agent-capability-platform-v2
UARC-054 input barrier:        d14af94e611de1cef986287f72cb027cf770e4ef
Windows implementation source: b09c680bbedd939a3dd9c5a05d9432a6ae79bdee
UARC-060 acceptance barrier:   71fd47fbb0a49073b8e736ee6d279f331c34f9e3
Windows candidate SHA-256:     ab572e619fb85b511488e25a94c76cf62f4ba7eace4c13751c43b1a6b6bb6dda
```

Mac implementation branch/worktree must be created from the exact UARC-060 acceptance barrier above. This handoff file
is delivered by its immediate documentation-only child; it does not change the tested production source. Before work:

```bash
git rev-parse HEAD
git merge-base --is-ancestor b09c680bbedd939a3dd9c5a05d9432a6ae79bdee \
  71fd47fbb0a49073b8e736ee6d279f331c34f9e3
git status --short
```

The ancestor check must exit 0 and the new Mac worktree must be clean. Do not rebuild the plan from a moving branch tip.

## 1. 状态声明

UARC-060 已在 Windows 完成并形成上述 barrier。任何 Windows 编译、WebView2、Win32、NSIS、真实模型或
视觉证据都不能证明 macOS 完成。

现有 CEF Browser 的 macOS 证据对应旧 Runtime、旧 Capability、旧 Store 和旧 Browser 产品模型，
不能直接复用为 UARC 完成证据。独立 CEF host、共享语义实现和原生 fixture 应作为基线复用，
但必须在 UARC shared barrier 上按新 Browser Resource/Action 合同重新验证。

## 2. 移交前置

macOS Worker 只在以下条件成立后开始：

- `TASK-MANIFEST.json` 中目标任务已 ready；
- shared contract barrier commit 已记录在 `STATUS.zh.md`；
- Windows Integration worktree 干净或已有明确 dirty ownership；
- Mac 分支从该 barrier 创建；
- write set 与活动 Windows tasks 不重叠；
- 交付目标、测试和 delete set 明确。

Mac Worker 占一个 Feature 席位。保留一个 Integration owner；UARC-061 与 UARC-062 只有在 Mac host、依赖
和最终解析后的 write set 均不重叠时才可并行，Feature worker 总数不得超过三个。UARC-063、合并、状态台账、
根配置和最终生成物只由 Integration 修改。

## 3. macOS 专属任务

### UARC-061：Browser

当前源码已有独立 macOS CEF host 和原生 fixture，但生产 `main.rs` 注入、完整产品生命周期、打包与
UARC Browser Resource/Action 合同尚未闭合，不能声明产品完成。固定架构为 Windows WebView2、
macOS 独立 CEF child NSView；不得恢复 WKWebView 方案。

需要实现/验证：

- 复用并收口 CEF child NSView 的创建、位置、显示、隐藏、缩放和关闭；
- 生产 host 注入、CEF helper 生命周期、bundle、签名和退出顺序；
- Retina logical/physical bounds；
- Tab/Popup/Dialog/Permission；
- 用户与 Agent 输入锁；
- 键盘、焦点、中文/日文 IME；
- 文件选择、上传、下载和取消；
- 页面 crash、Runtime teardown 和 App 退出；
- Browser Panel 与主窗口/侧栏 resize；
- managed browser Provider；
- attached Chrome 的简单安装级连接（若 Chrome 平台接口可用）；
- 无 Browser grant 时 fail closed。

Shared source 已提供 official Wave 2 Browser module、generic compiler、exact action schemas、managed/attached
provider adapter、effect settlement 与 canonical cancellation。Mac 工作不是重新引入旧 Browser Runtime；必须把
现有独立 CEF child NSView 接到 `engine_browser_tools` 所代表的 Browser owner/Resource/Action 生命周期，并证明
同一 authority/receipt 约束在 CEF transport 上成立。

不得复制 Windows WebView2 COM 层。Shared Browser Resource/Action contract 复用，macOS CEF transport
保持独立；已有 native fixture 只作为底层证据，不能替代正式产品会话与 UARC 组合验证。

### UARC-062：Process、PTY、Computer

需要实现/验证：

- process group/generation 身份；
- timeout/cancel/descendant cleanup；
- PTY stdin、resize、UTF-8 locale 和快速 TUI 输出；
- Command-Q/异常退出后的进程清理；
- Seatbelt/TCC 真实可用性与错误投影；
- Screen Recording 和 Accessibility denied/granted；
- Retina screenshot 坐标；
- Command/Option/Control 修饰键；
- Computer observe/input/launch；
- Terminal UI 焦点和 IME。

不能把权限拒绝当成工具成功，也不能因 TCC 复杂而自动给 Agent 缩减/扩大 Snapshot 权限。

### UARC-063：完整产品与制品

- 所有官方 Agent 和自定义最简 Agent；
- 通用 Agent 默认 Skill/MCP/Schedule/Browser/Computer；
- Coding 长程任务、compaction、cancel、restart；
- Requirements/AutoWork/AgentExecution；
- Companion/Robot 可用项；
- Creation/Office/Workshop；
- Agent 数据 clean cut；
- Browser/Computer/Terminal native UI；
- unsigned arm64 `.app`/DMG 工程检查；
- bundle resources、架构、启动、Command-Q 和清理；
- 有凭据时再做 Developer ID/notarization；无凭据必须标 `not_run`。

所有真实模型步骤固定使用商业 StepFun Coding Plan `step-3.7-flash`，不得用免费模型代替。凭据必须由 macOS
Keychain 或同等级本机 secret store 提供，经 credential-isolating runner 只向测试进程的 stdin 注入；不得写入
argv、环境继承链、仓库、fixture、报告或日志。若 Mac 尚无该凭据，模型步骤标为 blocked，不能用 mock/免费
模型改写为 pass。

## 4. shared 代码的 macOS 检查

即使任务没有 macOS 专属文件，只要修改以下内容，也必须在 Mac 重跑对应 gate：

| 领域 | macOS 检查 |
| --- | --- |
| Contracts/Kernel/Store | Rust tests、fresh DB、Agent-only reset、文件权限/路径 |
| Runtime | simple/Tool/long-horizon/cancel/restart tests |
| Workspace | case sensitivity、symlink、atomic replace、Git、temp path |
| Skill/MCP/Plugin | stdio process、resource path、shutdown |
| UI | typecheck/tests、Safari/WebKit layout、Retina、focus/IME |
| Browser | CEF native smoke、生产 child NSView、helper lifecycle 与 UARC Browser Resource 闭环 |
| Computer | TCC + physical input |
| Packaging | app/DMG/resources/architecture/signature structure |

## 5. 环境记录

Mac evidence 必须先记录：

```text
source commit
git status
macOS version/build
hardware model
architecture
Xcode/Command Line Tools version
rustc/cargo/bun/node versions
Tauri/CEF runtime facts（含 CEF/Chromium 固定版本）
TCC state used by the test
signing/notarization credential availability（只记 available/absent，不记 secret）
StepFun Coding Plan credential availability（只记 available/absent，不记 secret）
```

## 6. 工程命令

按任务选择最小集合；最终 UARC-063 执行完整集合。

```bash
git rev-parse HEAD
git status --short
bun run typecheck
bun run check

cargo test -p nomifun-agent-contracts
cargo test -p nomifun-agent-kernel
cargo test -p nomifun-agent-session
cargo test -p nomifun-coding-engine
cargo test -p nomifun-browser-platform
cargo test -p nomi-process-runtime
cargo test -p nomifun-terminal

bun test --cwd ui
bun run build:mac --check arm
bun run build:mac arm
```

UARC-061 还必须在 Mac 运行 CEF/native Browser 定向套件和统一 Agent Browser smoke；UARC-062 必须运行真实
Process/PTY cleanup 与 TCC denied/granted Computer 矩阵。UARC-063 在合并后的 clean barrier 上统一运行完整
workspace crates/doctests、完整 UI、production renderer build、`bun run check`、arm64 app/DMG 与人工矩阵。
不要同时运行多个 Cargo、全量 UI、native 或 packaging gate。

生成 release lock 后运行：

```bash
bun scripts/validation/check-macos-arm64-native.mjs \
  --release-lock /absolute/path/release-lock.json \
  --app /absolute/path/NomiFun.app \
  --dmg /absolute/path/NomiFun.dmg \
  --report /absolute/path/uarc-macos-report.json
```

命令名在实际 crate rename 后由 Integration 更新；Mac Worker 不为让文档命令通过而保留旧 crate。

## 7. 原生人工矩阵

### Agent Workbench

- 880×600 和宽窗口；
- 官方 Agent 卡片、模块分类、搜索、勾选、Action 子项；
- 默认通用 Agent 模块；
- loading/empty/error/unavailable/dirty/save；
- 键盘导航、VoiceOver 基本语义、Retina 清晰度。

### Browser

- 普通 Agent 和 delegated Agent 授权/未授权；
- 打开 Panel 不创建新 Session；
- managed provider 创建与复用；
- attached Chrome 一次连接；
- navigate/observe/act/render/download/upload/evaluate；
- popup/dialog/permission/file；
- Agent 运行时用户输入锁；
- resize/hide/show/close/crash/Command-Q。

### Coding

- Workspace read/search/write/patch；
- Process start/poll/input/cancel；
- VCS status/diff/stage/commit；
- 多轮长任务与 compaction；
- steering、取消、重启、不重放未知效果；
- 完成证据和未验证项。

### Computer/Terminal

- 权限未授予时清晰引导；
- 权限授予后截图、输入、快捷键；
- PTY/IME/resize/退出；
- App 退出后无残留子进程。

## 8. 证据格式

每项原生证据记录：

```text
task_id
source_commit
platform/os/arch
scenario
exact steps
expected
observed
result: pass | fail | blocked | not_run
logs/artifact paths
known limitations
```

截图/视频可辅助视觉问题，但不能替代日志、进程退出和实际状态检查。不得记录用户凭据、Cookie、
页面私密正文或签名 secret。

## 9. 回合并

1. 从 `71fd47fbb0a49073b8e736ee6d279f331c34f9e3` 创建 clean Mac branch/worktree；
2. UARC-061 与 UARC-062 各提交一个主实现 commit 和最多一个修复 commit；
3. 各自提交 delivery summary 和逐项 native evidence；
4. Integration 检查依赖、write/delete/retained set、物理删除范围和无 WKWebView/旧 Runtime 回流；
5. 在 Mac integration barrier 串行运行定向 gate，再执行 UARC-063 完整产品与 arm64 package gate；
6. Integration 合入 Mac commits 与证据，只在 gate/集成/blocker 变化时更新状态台账；
7. 回到 Windows 执行 UARC-064：受影响 crates/UI、WebView2 Browser、Process/Computer、`bun run check`；
8. 两平台均 verified 后执行 UARC-070 requirements-to-evidence、production reachability 与临时代码清理审计；
9. 任一项不通过则保持 task active/blocked，不创建平台特例、免费模型替代或合同绕过。

## 10. 当前 macOS 状态

| 项目 | 状态 |
| --- | --- |
| UARC-060 shared-source barrier | ready: `71fd47fbb0a49073b8e736ee6d279f331c34f9e3` |
| 当前 UARC source 在 Mac 编译 | pending real-Mac evidence |
| Browser managed Provider | shared owner implemented; CEF production binding pending |
| Attached Chrome Provider | Windows verified; macOS pending investigation/implementation |
| Process/PTY | existing code, UARC revalidation pending |
| Computer/TCC | existing code, UARC revalidation pending |
| Agent Workbench visual | pending |
| New Agent Store clean cut | pending |
| `.app`/DMG | pending |
| Developer ID/notarization | credential-dependent, not run |

## 11. Windows 移交证据

Windows acceptance 的完整记录见 `UARC-060-IMPLEMENTATION.zh.md`。冻结候选来自 clean implementation source
`b09c680bbedd939a3dd9c5a05d9432a6ae79bdee`：UI 3538/3538、workspace crates/doctests、UARC boundary、
WebView2 deterministic/live Browser、read-only Computer、Process/native stability、真实 `step-3.7-flash` 与
NSIS 14-check install smoke 均通过。候选安装、启动、`/health` 200、WebView2 CDP、完整进程树清理与卸载残留
检查均为 pass。

这些结果只冻结 shared source 和 Windows 行为。Mac 必须独立产生以下不可替代证据：

- CEF helper/child NSView 生产注入、Retina bounds、focus/IME、popup/dialog/permission/file/download；
- Browser Resource/Action exact authority、effect receipt、cancel/terminal-before-unlock 与 app teardown；
- Process group、PTY、Command-Q cleanup；
- TCC Screen Recording/Accessibility denied 与 granted，以及物理输入/Retina 坐标；
- 880×600 和宽窗口 UI/VoiceOver 检查；
- arm64 `.app`/DMG 的资源、架构、启动、退出和签名结构；Developer ID/notarization 仅在凭据 available 时运行。

## 12. 当前 blocker 与恢复点

UARC-061、UARC-062 已 ready，但当前 Windows 主机无法生成上述 Mac 真机证据。唯一当前外部 blocker 是可用的
Apple Silicon Mac host（以及真实输入步骤所需 TCC 用户授权；签名/notarization credential 可合法记为 absent）。
拿到 Mac host 后从第 0 节 barrier 恢复，不重跑规划、不跳过 UARC-061/062，也不提前执行 UARC-064/070。

在这些项闭合前，任何 UARC 总结必须明确写“Windows implemented; macOS pending”，不能写“跨平台完成”。
