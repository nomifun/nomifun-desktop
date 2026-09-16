# Browser Workspace v2：macOS 原生前置验证记录

状态：**原生输入门槛未通过，尚未完成 macOS Browser Workspace 交付。**

本记录执行 `2026-09-16-browser-workspace-v2-macos-work-prompt.zh.md` 与 macOS handoff 的
“先做最小原生 spike，失败则明确 unsupported”要求。没有开启 macOS Browser availability，
没有把独立 probe 当成已接入 Tauri / Agent 的产品实现。Windows host、wry close 补丁、
macOS 退出与 PTY 清理保持原样。

## 基线与环境

- 分支：`rf/agent-capability-platform-v2`。
- 开始时工作区干净；fetch 与指定分支 `pull --ff-only` 后基线为
  `9d4d7b778cb5495a32730fb17c11967da49f8740`。
- 本记录、原生 probe 与 runner 的提交由本文件所在 Git 提交标识；不把基线 SHA 冒充最终提交。
- macOS 26.6.2，build 25G83；本机 arm64。
- Xcode 26.5 / 17F42；rustc 1.96.0 / ac68faa20；Cargo 1.96.0；Bun 1.3.14。
- 系统 WebKit：21624.5.1.11.3。
- 这次输入窗口的 backing scale 为 1。Retina、多屏边界尚未验收。

## 代码与重现方式

新增内容只用于原生前置验证，不是第二套生产浏览器：

- `apps/desktop/examples/support/macos/native_browser_probe.swift`：AppKit 主窗口中的真实 child
  WKWebView；复用既有 `examples/fixtures/browser_workspace.html`，检查页面端事件和最终状态。
- `scripts/validation/run-macos-browser-native-probe.mjs`：编译本机架构、macOS 14 最低目标，签名、
  启动本地 fixture HTTP server，通过 LaunchServices 运行应用，读取结果并传播失败退出码。
  构建/启动环境只传递必要的系统变量，不读取模型凭证。

```sh
node scripts/validation/run-macos-browser-native-probe.mjs --probe storage
node scripts/validation/run-macos-browser-native-probe.mjs --probe input --transport appkit
node scripts/validation/run-macos-browser-native-probe.mjs --probe input --transport pid
```

默认 ad-hoc 签名。可用 `--identity '<Developer ID identity>'` 指定本机签名身份。
运行 input probe 时窗口必须成为 active key window；激活超时返回 2，不将未准备好的窗口记为通过。
本次最后一次 AppKit 检查由自动化点击**窗口标题栏**激活，页面动作仍全部由 probe 的原生路径执行。

退出码：0 为该 probe 的断言通过；1 为原生一致性失败；77 为事件投递权限未授权；2 为准备/运行失败。
非 macOS 直接非零退出。runner 不把 `open` 的退出码或空结果当成验收通过。

只有获得用户确认后才能增加 `--request-event-access`，发起系统事件投递授权请求。
这个选项不修改 TCC 数据库，不自动替用户授予权限，也不授权个人浏览器接入。
截至本记录，授权确认问题已提出，尚未收到答复，**没有执行此选项**。

## 原生结果

| 检查 | 结果 | 边界与证据 |
| --- | --- | --- |
| 同一真实 WKWebView 的 native click | 前置检查通过 | fixture click handler 计数为 1；不是 JS click |
| 点击后的输入焦点 | 前置检查通过 | `document.activeElement.id == field` |
| 原生中文组合文本 | 前置检查通过 | `NSTextInputClient.setMarkedText/insertText`；收到 trusted composition/input，值为 `a中文` |
| 页面端 `isTrusted` | 前置检查通过 | 已观测的输入事件均为 true |
| 滚轮默认行为 | 前置检查通过 | 原生 `scrollWheel`；目标 scroller 的 `scrollTop == 150` |
| 局部输入门原型 | 有限检查通过 | 拒绝两个没有宿主令牌的入队点击；不是完整 RunGuard / 真实物理键盘验收 |
| 系统光标 | 前后坐标相同 | 没有调用全局 cursor warp、CGEventPost 或全局鼠标接管 |
| mouse button mask | **失败** | trusted pointerdown 的 `buttons == 0`，期望 1 |
| pointer-capture drag | **失败** | fixture `capturedMoves == 0` |
| PID 定向输入 | **未执行** | `CGPreflightPostEventAccess == false`，返回 77；没有发送 PID 输入 |
| 持久化数据隔离 | 通过 | 两个随机 UUID data store 的 Cookie/localStorage 相互独立 |
| 关闭后重建 | 通过 | 用同一 UUID 新建 WebView 后恢复对应数据 |
| 清理当前 store | 通过 | A 的 Cookie/localStorage 为空；B 保持原数据 |
| hide/show/resize | 有限检查通过 | B 对象 identity 和数据保持；尚非会话切换或 renderer slot 验收 |
| probe 自有 store 删除 | 通过 | 两个 store 的 WebKit 删除回执均成功；初始探索产生的两个 store 也已精确清理 |

因此，不能以“事件 trusted”“能输入中文”或“截图正常”开启 macOS Browser。
本机 AppKit 路径不满足鼠标按钮状态和拖拽合同。WebKit 公开源码中，普通输入读取系统的
`currentlyPressedMouseButtons()`；Automation 分支有单独处理。这是定位线索，不把 WebKit main
源码当成本机系统二进制的精确版本，也不调用其私有 Automation API 绕过约束。

参考：[WebEventFactory](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/Shared/mac/WebEventFactory.mm)、
[PlatformEventFactoryMac](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/mac/PlatformEventFactoryMac.mm)、
[公开的独立持久化 store API](https://developer.apple.com/documentation/webkit/wkwebsitedatastore/init(foridentifier:))。

还发现一个清理前提：直接在尚未初始化 WK 对象的最小进程调用 removeDataStore，本机 WebKit 曾在
`WTF::RunLoop::dispatch` 崩溃。先初始化 nonpersistent WKWebView 后，两项旧 probe store 删除均成功。
正式 storage probe 在真实视图生命周期之后执行清理，不能把目录删除代替 WebKit 回执。

## 自动检查

| 命令/范围 | 本机结果 |
| --- | --- |
| `bun install --frozen-lockfile` | 通过 |
| 未改动基线 `cargo check -p nomifun-desktop --bin nomifun-desktop --no-default-features` | 通过；现有 warnings 保留 |
| `cargo test -p nomifun-browser-platform` | 49 通过 |
| `cargo test -p nomifun-net --lib egress` | 22 通过 |
| `cargo test -p nomifun-app --features browser-use --lib nomi_core_role_defaults` | 2 通过 |
| app `--test browser_workspace --test system_browser` | 3 + 1 通过 |
| `agent-v2-contract check` | 通过 |
| Browser / SystemBrowser / ChatLayout 定向 UI tests | 98 通过 |
| browser boundary / desktop boundary / typecheck / i18n | 通过 |
| `bun run build:mac arm --check` | 预检通过；不是安装包构建或安装验收 |

Cargo 串行执行。未运行 Windows-only `browser_workspace_smoke` 冒充 Mac 通过。
上述 Rust tests 为共享合同/模拟宿主证据，只有 Swift probe 的页面事件/状态属于真实 WKWebView 证据。

## 本地产物与证据

本机证据根目录：`dist/browser-macos-validation-20260916/`（不入 Git）。

- `baseline-cargo-check.log`、`browser-platform.log`、`net-egress.log`、`role-defaults.log`、
  `app-browser.log`、`agent-contracts.log`、`browser-ui-tests.log`、`ui-checks.log`、`package-preflight.log`。
- `reproducible/input-appkit-1789548152932.json`：激活窗口后的 AppKit 一致性失败，退出码 1。
- `reproducible/input-pid-1789547469574.json`：系统权限前置条件失败，退出码 77。
- `storage-conformance.log` 和对应 `reproducible/storage-appkit-*.json`：数据隔离和精确删除回执。
- `cleanup-initial-stores.json`：只删除本次初始探索的两个 UUID store。

测试包：`dist/browser-macos-validation-20260916/NomiBrowserNativeProbe-arm64.zip`。
它是**原生前置测试应用，不是 NomiFun 安装包**。Mach-O arm64；已用本机 Developer ID 签名，
`codesign --verify --strict` 通过；未提交公证。

- ZIP SHA-256：`7dafcc0e12875a33937d8f725d5bba3b699346c606f36fe9cc003841b93a22f7`
- 主可执行文件 SHA-256：`8cbe97c03fd05405d31f5200ca85b7f028e55a44e666bb255b3889787a420e16`

早期一次性 Swift 文件、旧 spike app 和自有 HTTP server 已清理；保留上述可重现 runner 与失败证据。

## 尚未交付的项目与下一步

当前阻塞点是可信鼠标输入的完整语义。下一步先获得一次系统授权确认，验证公开的进程定向输入是否满足
按钮状态、drag、光标不干扰及取消要求；授权不等于该路径一定通过。若仍失败，应保持 unsupported，
不要开启宿主、使用私有 WebKit API、伪造 DOM 事件或新增第二套控制平台补偿。

以下均未宣称完成：生产 Tauri BrowserRuntime 适配、生产输入锁/Stop settle、Retina/多屏、跨站 iframe/
Shadow DOM/stale ref、生产上传下载/dialog/popup/permission/crash/关闭退出、会话切换、真实物理中文 IME
候选窗、正式配置→Nomi Engine→真实 `step-3.7-flash` 闭环、Mac 产品安装包及其公证。
真实模型未运行是因为原生前置门槛未通过，不是声称本机没有凭证；没有读取或复制 Windows/个人浏览器密钥。
Coding Engine 没有获得 Browser 能力；系统浏览器 attach 仍为 unavailable；Linux 后置。

回 Windows 时仍需对最终整合版本执行：

```sh
cargo check -p nomifun-desktop --bin nomifun-desktop --no-default-features
cargo test -p nomifun-browser-platform
cargo test -p nomifun-app --features browser-use --lib nomi_core_role_defaults
cargo test -p nomifun-app --features browser-use --test browser_workspace --test system_browser
cargo run -p nomifun-desktop --example browser_workspace_smoke
cargo run -p nomifun-desktop --example browser_workspace_smoke -- --html-drag-only
cargo run -p nomifun-desktop --example browser_workspace_smoke -- --agent-only
bun run check:browser-platform-boundary
bun run check:desktop-ui-boundary
bun run typecheck
```

Windows 场景重点：同文档 drag、输入所有权与 Stop、浏览器聚焦时的停止按钮、child close、
popup/dialog/permission、上传下载、会话切换与站点数据清理。Mac 本轮不替代 Windows 验收。
