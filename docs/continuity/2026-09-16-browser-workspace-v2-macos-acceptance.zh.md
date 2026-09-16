# Browser Workspace v2：macOS 整合版本验收

结论：**验收未通过。macOS 原生 Browser 继续 unavailable，不能交付为已完成的跨平台 Browser。**

本轮接续工作 prompt、macOS handoff 和 native-preflight 记录，重新验证整合版本。
原生输入前置门槛的失败已复现，不是系统授权缺失。共享合同测试、独立 WKWebView probe、
产品安装包和真实模型属于不同证据，以下分别记录。

## 1. Git 基线与范围

- 开始时工作区干净，分支 `rf/agent-capability-platform-v2`，本地 HEAD
  `02c32a51d`，保留上个会话的 `061791dd2` 和 `02c32a51d`。
- fetch 后远程 HEAD 为 `36a57dd4f`，与本地分叉；指定分支 `pull --ff-only` 正确拒绝。
  使用普通 merge 无冲突整合，不 reset、不 rebase、不 force push。
- 整合与本轮所有代码/包验证基线：`6a2a30785951fbf098f561ba771341b9c01410ea`。
- 远程增加的是创作画布改动。本轮未修改 Browser 生产实现、平台 cfg、Windows wry 补丁、
  macOS 退出/PTY 清理、数据基线或用户数据。交付提交仅增加本验收记录。
- 最终记录提交以本文件的 Git 提交为准；文档提交不冒充安装包源码基线。

## 2. 环境

| 项目 | 本机实测 |
| --- | --- |
| macOS | 26.6.2，build 25G83 |
| CPU / 本轮目标 | arm64 / aarch64-apple-darwin |
| Xcode | 26.5 / 17F42 |
| rustc / Cargo | 1.96.0 / 1.96.0 |
| Bun | 1.3.14 |
| Tauri / wry | 2.11.2 / 0.55.1，仓库 wry child-close 补丁保留 |
| 系统 WebKit | 21624.5.1.11.3 |
| 输入 probe backing scale | 1；Retina 与多屏没有验收 |

证据目录：`dist/browser-macos-acceptance-20260916/`，不入 Git。

## 3. 已实际执行的定向检查

| 命令 | 结果 | 证据文件 |
| --- | --- | --- |
| `cargo check -p nomifun-desktop --bin nomifun-desktop --no-default-features` | PASS，已有 warnings | `baseline-cargo-check.log` |
| `cargo test -p nomifun-browser-platform` | 49 PASS | `browser-platform.log` |
| `cargo test -p nomifun-net --lib egress` | 22 PASS | `net-egress.log` |
| `cargo test -p nomifun-app --features browser-use --lib nomi_core_role_defaults` | 2 PASS | `role-defaults.log` |
| `cargo test -p nomifun-app --features browser-use --test browser_workspace --test system_browser` | 3 + 1 PASS | `app-browser.log` |
| `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check` | PASS | `agent-contracts.log` |
| `bun test --cwd ui src/renderer/pages/conversation/Browser src/renderer/pages/conversation/SystemBrowser src/renderer/pages/conversation/components/ChatLayout` | 98 PASS，0 FAIL | `browser-ui-tests.log` |
| `bun run check:browser-platform-boundary` | PASS | `browser-boundary.log` |
| `bun run check:desktop-ui-boundary` | PASS | `desktop-boundary.log` |
| `bun run typecheck` | PASS | `typecheck.log` |
| `bun run check:i18n` | PASS | `i18n.log` |
| `bun run build:mac arm --check` | PASS，仅工具预检 | `package-preflight.log` |

Cargo 按上述顺序串行执行；独立 UI 检查同时执行。没有把 doc-test 的 0 tests 算作 native PASS，
也没有将 Windows-only smoke 在 Mac 上的拒绝执行算作通过。依赖沿用上个会话已完成的 frozen install，
本次整合未改变 lockfile。

Role defaults 的两个测试分别证明：有宿主时重载保留精确 Browser 绑定；无宿主时不伪造 Browser。
应用集成测试使用测试宿主，不能代替真实 WKWebView 或真实模型。

## 4. 真实 WKWebView 复验

复用此前签名、已授权的同一个 probe 包，未重新请求权限、未改写签名包。
本轮开始与源码比较确认 probe/runner 未被整合改动修改。主可执行文件 SHA-256：
`6694e21378cbaee096a35ed97e9272cd91c5c6bc27ffd09174d32a304d16acf6`。

```sh
node scripts/validation/run-macos-browser-native-probe.mjs --probe permission --reuse-signed-app --output dist/browser-macos-validation-20260916/reproducible
node scripts/validation/run-macos-browser-native-probe.mjs --probe input --transport pid --reuse-signed-app --output dist/browser-macos-validation-20260916/reproducible
node scripts/validation/run-macos-browser-native-probe.mjs --probe input --transport appkit --reuse-signed-app --output dist/browser-macos-validation-20260916/reproducible
node scripts/validation/run-macos-browser-native-probe.mjs --probe storage --reuse-signed-app --output dist/browser-macos-validation-20260916/reproducible
```

| 项目 | 本轮结果与边界 |
| --- | --- |
| 系统事件权限 | `eventPostingAllowed=true`，退出 0；`permission-appkit-1789554337573.json` |
| PID 定向输入 | **FAIL / 退出 1**；`input-pid-1789554356008.json` |
| AppKit 入队输入 | **FAIL / 退出 1**；`input-appkit-1789554398954.json` |
| 两条路径的失败断言 | `mouse_button_state=false`、`pointer_capture_drag=false` |
| 页面具体状态 | 三个 pointerdown 均 `isTrusted=true` 但 `buttons=0`；`capturedMoves=0` |
| 其他输入断言 | 点击计数 1、输入焦点 `field`、值 `a中文`、composition 事件 trusted、scrollTop 150 |
| 输入门原型 | 阻断两个未标记入队点击；未验证真实物理输入阻断或生产 RunGuard |
| 系统光标 | 两次均前后位置相同；未投递全局事件 |
| 数据隔离、重建恢复、hide/show/resize | 独立 probe PASS；不是生产 Conversation/renderer slot 验收 |
| 精确清理 | 两个自有 UUID store 删除成功，`removedOwnedStores=true`、`cleanupErrors=[]` |
| storage 结果文件 | `storage-appkit-1789554431851.json`，退出 0 |

原始 JSON 位于上面命令的 output；本轮输入与存储 JSON 也复制到本轮证据目录。
`native-pid.log`、`native-appkit.log`、`native-storage.log` 保存 runner 输出。
中文证据来自 `NSTextInputClient` 组合文本接口，不能称为物理中文 IME 候选窗通过。

公开 WebKit 源码与实测现象一致：普通事件从 `NSEvent.pressedMouseButtons` 获取按钮掩码，
Automation 分支另行计算。这是定位依据，不证明当前系统二进制与 WebKit main 完全相同，
也不构成“所有公开 API 永远无法实现”的结论。

- [WebEventFactory 普通与 Automation 输入分支](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/Shared/mac/WebEventFactory.mm)
- [PlatformEventFactoryMac 当前按钮状态读取](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/mac/PlatformEventFactoryMac.mm)

在本轮冻结合同下，不能用 DOM 合成事件、私有 WebKit Automation、全局鼠标接管或第二个浏览器
弥补这一缺口，也不能删除 buttons/drag 断言把失败改成通过。

## 5. 产品安装包

实际执行 `bun run build:mac --signed arm`，退出 0。release 编译约 11 分 21 秒；
产物为 **NomiFun 0.7.6、arm64、Developer ID 签名并通过 Apple 公证**，不是前置 probe 包。

| 检查 | 本轮结果 |
| --- | --- |
| App 公证 | Accepted，`ce0830ac-9d47-4bf6-ae79-14b655b22b9e` |
| DMG 公证 | Accepted，`37cd1b74-b5b8-450f-89b8-a7373b76bbc6` |
| App / DMG staple validate | 均退出 0 |
| 从 DMG 复制安装 | 只读挂载最终 DMG，复制到本轮 `installed/NomiFun.app`，随后卸载卷；未覆盖已有安装 |
| 安装后签名校验 | `codesign --verify --deep --strict --verbose=2` 退出 0 |
| Gatekeeper | `spctl --assess --type execute --verbose=2` 退出 0，`source=Notarized Developer ID` |
| 安装后主程序架构 | `lipo -archs` 为 `arm64` |
| 独立数据目录启动 | `NOMIFUN_DATA_DIR=dist/browser-macos-acceptance-20260916/installed-data`（实际传入绝对路径），未读取现有数据目录 |
| 健康检查 | 测试实例 PID 60215、端口 55689；`/health` 返回 `status=ok`、`version=0.7.6` |
| 原生主窗口 | CUA 实际观察到 `NomiFun` 窗口、完整首页和输入框 |
| 正式 Browser 入口 | 从首页点击“会话浏览器”，创建本轮隔离测试会话；后端 POST 返回 **501 / BROWSER_NATIVE_SURFACE_UNAVAILABLE**；界面显示“浏览器暂不可用”，导航控件禁用 |
| 正常退出 | Cmd+Q 后测试进程消失、健康监听关闭、无该 PID 的直接子进程；没有用 kill 强制结束 |

包与审计文件：

- DMG：`dist/desktop/NomiFun_0.7.6_aarch64.dmg`。
- DMG SHA-256：`56262c648b4fad2a8b2e4f20a95f3f92e66008d403518a88e529aa16a63abe91`。
- 主可执行文件 SHA-256：`6bb550360c61af49eb534e24ddfeab7b84d92765d55b02e194a65542bac96764`。
- `dist/desktop/NomiFun_0.7.6_aarch64.release-lock.json` 固定源码提交、包、主程序和法律文件哈希；构建阶段 verify 通过。
- 本轮证据目录下 `package-build.log`、`package-install.log`、`package-verification.json`、
  `package-startup.json`、`package-startup.log`、`package-ui-evidence.json`、`package-exit.json`。

**包构建、签名、公证和应用 shell 启动通过，不等于 Browser 功能通过。** 未发出模型请求，
也没有在这次退出检查中开启 Browser host 或 PTY，不能把它记为这些资源的退出验收。
启动日志另有“已有 knowledge broker 正在运行”的 warning；本轮未停止其他实例或验收外部知识 MCP。

实机额外发现的 UI 问题：Browser 不可用时状态行仍显示“可手动操作”，并直接展示原始
`BackendHttpError` / 501 JSON。该显示与不可用状态不一致，记为未修复；不能将这个界面算作正常
Browser 操作验收，也没有因为禁用导航控件就忽略它。

## 6. 生产能力与未运行项

已检查实际代码：

- `apps/desktop/src/main.rs` 只有 `cfg(windows)` 注入 `DesktopBrowserHost`；Mac 仍为 `None`。
- `apps/desktop/src/browser_surface/mod.rs` 的 host/windows/automation 仍是 Windows 条件模块。
- app `router/browser_workspace.rs` 对无原生宿主返回 `NativeUnavailable` / HTTP 501。
- `ChatLayout/index.tsx` 的 SystemBrowser 仍使用 `available={isWindowsRuntime}`。
- `vendor/wry/NOMIFUN-PATCH.md` 和 WebView2 child close 补丁没有修改。

以下均 **NOT RUN / 未交付**，不是 PASS：

1. 生产 Tauri BrowserRuntime 的 Mac 接入、语义观察桥、完整可信 press/hover/右键/双击/select/drag。
2. 真实物理输入锁、Stop settle、崩溃/失焦/取消后释放按键与拖拽、浏览器聚焦时停止按钮。
3. Retina/多屏、同源与跨站 iframe、Shadow DOM、父层遮挡、旧 ref 失效、生产会话切换与 slot 生命周期。
4. 生产上传下载、dialog/popup/permission、Browser crash/close/shutdown 与站点清理。
5. 正式配置 → Nomi Engine → `step-3.7-flash` 真实前端闭环，以及生产 Mac 脚本 Agent fixture。
   未运行原因是原生输入门槛失败且生产宿主未实现；没有读取模型凭证，不称为凭证缺失。
6. Coding Engine Browser、macOS 系统浏览器 attach、Linux；没有通过启动临时 Chrome/profile 替代。
7. Windows 当前整合版本的原生运行；本机无法执行 Windows WebView2。

继续实现的前提是：先提供同一 WKWebView、公开原生输入路径、无全局鼠标干扰且按钮状态/拖拽
正确的可复现证据。当前没有此证据，按 handoff 保持 unavailable。若只能更换内核或改变产品合同，
需另行明确产品决策，不能在本次验收中默认放宽。

## 7. 回 Windows 必须执行

对包含本轮整合的最终分支执行，不能复用旧 EXE 的结论：

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

以下参数均已在当前 `browser_workspace_smoke.rs` 中核对，每个单独执行：

```powershell
$cases = @(
  '--runtime-locks-only', '--workspace-only', '--presentation-only',
  '--frame-input-only', '--frame-drag-only', '--popup-only', '--managed-popup-only',
  '--dialog-probe-only', '--dialog-close-only', '--permissions-only',
  '--permission-timeout-only', '--picker-only', '--upload-frames-only',
  '--user-files-only', '--user-downloads-only', '--agent-downloads-only',
  '--user-download-cancel-active-only', '--site-data-probe-only',
  '--crash-only', '--close-all-only'
)
foreach ($case in $cases) {
  cargo run -p nomifun-desktop --example browser_workspace_smoke -- $case
  if ($LASTEXITCODE -ne 0) { throw "Native Browser smoke failed: $case" }
}
```

另在真实 UI 核对浏览器聚焦时的停止按钮。真实模型通过正式配置执行，
与 `--agent-only` 脚本模型证据分开。Windows/macOS 两端都完成这些门槛后，才能签署跨平台通过。
