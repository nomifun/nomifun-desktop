# UARC macOS 实施与验证移交

## 1. 状态声明

当前 UARC 规划和后续首轮实现发生在 Windows。任何 Windows 编译、WebView2、Win32、NSIS 或视觉
证据都不能证明 macOS 完成。

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

Mac Worker 占一个 Feature 席位。存在 Mac Worker 时，Windows Feature tasks 最多同时两个。

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
```

## 6. 工程命令

按任务选择最小集合；最终 UARC-063 执行完整集合。

```bash
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

bun run build:mac --check arm
bun run build:mac arm
```

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

1. Mac Worker 提交一个主实现 commit 和最多一个修复 commit；
2. 提交 delivery summary 和 evidence；
3. Integration 检查 write set/删除范围；
4. 先在 Mac barrier 运行定向 gate；
5. 合入 Integration；
6. Windows 执行 UARC-064 回归；
7. 两平台状态更新；
8. 不通过则保留 task active/blocked，不创建平台特例绕过合同。

## 10. 当前 macOS 状态

| 项目 | 状态 |
| --- | --- |
| 当前 UARC source 在 Mac 编译 | pending |
| Browser managed Provider | pending implementation |
| Attached Chrome Provider | pending investigation/implementation |
| Process/PTY | existing code, UARC revalidation pending |
| Computer/TCC | existing code, UARC revalidation pending |
| Agent Workbench visual | pending |
| New Agent Store clean cut | pending |
| `.app`/DMG | pending |
| Developer ID/notarization | credential-dependent, not run |

在这些项闭合前，任何 UARC 总结必须明确写“Windows implemented; macOS pending”，不能写“跨平台完成”。
