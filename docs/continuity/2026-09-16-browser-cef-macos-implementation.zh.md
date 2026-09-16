# macOS CEF 实施记录（开发检查点，全面验收已暂缓）

本轮按用户要求收工，后续开发与完整测试统一见[下一次接续 TODO](2026-09-16-browser-cef-macos-next-session.zh.md)。
未安排后台继续运行。

依据用户确认的[平台架构决策](2026-09-16-browser-platform-architecture-decision.zh.md)：Windows 保留 WebView2，
Mac 使用独立 CEF 原生宿主。**尚未达到上线条件；生产 `main.rs` 的 macOS Browser 注入仍未开启。**

## 当前代码

- `crates/backend/nomifun-browser-macos/`：仅 macOS 目标依赖 CEF。原生引擎、CEF 生命周期、应用自身
  NSApplication 子类的公开 CefAppProtocol 接入、输入门、隔离 request context、页内协议通道与 helper。
- `apps/desktop/examples/browser_cef_smoke.rs`：真实 Tauri 原生窗口中的 CEF child NSView。
- `scripts/validation/run-macos-cef-smoke.mjs`：串行编译、完整 helper bundle、签名校验、实际启动与非零失败。
- `apps/desktop/Cargo.toml` 仅在 macOS target dependencies 中引入适配 crate；Windows 保留 WebView2/wry；共享语义算法、pending work 和评估/截图通过 native ports 复用。

## 已定位并修复的接入问题

1. **设置字符串丢失**：`cef-rs 152.3.0+152.0.6` 将 owned/Clear 字符串字段转换到原生设置结构时丢弃字段。
   `text.rs` 使用有明确生命周期的 UTF-16 backing 和无析构的 borrowed fields，在同步原生调用中复制。
   回归验证 helper 路径、独立 profile 路径及中文路径在原生结构中保持原值。
2. **helper 不能只有一个**：macOS 的 CEF 会派生 `(GPU)/(Renderer)/(Plugin)/(Alerts)` helper 路径。
   Runner 使用标准 `.app/Contents/MacOS/<同名 executable>` 结构，并逐个签名；未禁用沙箱或进程签名校验。
3. **外部消息循环**：使用 CEF UI task runner 执行浏览器工作，GCD 只驱动主线程消息泵。
   按上游 external-pump 示例保留最大 33ms 的补充调度和重入保护；防止旧任务清掉更新的立即唤醒。
4. **子页关闭**：在 CEF DoClose 之后移除该 child NSView，等待 OnBeforeClose，再执行 CEF shutdown。
   不向 Tauri 主窗口发送 performClose，也不以“已发关闭命令”作为完成。
5. **权限边界**：helper 在加载 CEF 前清除非必要环境变量；页面无 Tauri IPC，无公开调试端口。
   用户输入门仅作用于所属 CEF 视图，保留应用 Quit，并记录跨边界拖动的输入归属。

公开参考：

- [CEF macOS 应用结构](https://chromiumembedded.github.io/cef/general_usage)
- [CEF external message pump](https://github.com/chromiumembedded/cef/blob/master/tests/shared/browser/main_message_loop_external_pump.cc)
- [cef_application_mac.h](https://github.com/chromiumembedded/cef/blob/master/include/cef_application_mac.h)

## 当前证据

运行环境仍为 macOS 26.6.2 / arm64，CEF 152.0.6 / Chromium 152.0.7977.83。
CEF 绑定固定版本及 checksum 在 Cargo.lock 中。

```sh
CEF_PATH="$PWD/dist/browser-cef-macos-20260916/runtime" \
  cargo test -p nomifun-browser-macos --lib
CEF_PATH="$PWD/dist/browser-cef-macos-20260916/runtime" \
  node scripts/validation/run-macos-cef-smoke.mjs --identity '<本机签名身份>'
```

首个 Tauri + CEF 原生输入闭环通过证据：`dist/browser-cef-native-smoke/run-7m2fu9/native-result.json`。
点击、文本 `CEF 中文输入`、focus、输入事件 trusted、buttons、pointer-capture drag、wheel 均通过，
且 `shutdown_complete=true`。这是使用页面输入协议的 Agent 文本操作证据。
该早期记录中的 `input_gate_locked` 仅是锁状态检查；后续 `run-K8fR9f` 另有本地 AppKit 输入阻断/恢复断言。

此前独立 CEF 实验位于 `dist/browser-cef-macos-20260916/`，没有作为产品降级页使用。
其额外 Cocoa 组合文本检查记录了 `compositionend.isTrusted=false`；严格断言的失败保留，
不能把普通 Unicode 文本操作的通过冒充物理中文 IME 候选窗或全部组合事件通过。

## 后续原生验收进展

`dist/browser-cef-native-smoke/run-K8fR9f/native-result.json`：24 项断言均通过，`shutdown_complete=true`。

- 共享语义驱动的观察、可信点击、Unicode type、wheel、pointer capture drag；AppKit 本地排队点击锁定时被阻断，解锁后生效。
- 原生 prompt 的中文回复、过期/取消回复拒绝、confirm drain、停止期间的新弹窗抑制；这些不通过 DOM 合成事件实现。
- 独立 Mac `BrowserRuntimeFactory` 接入既有 RunGuard 与 pending work 算法；评估遇到网站 dialog 后保留原任务，Stop 取消并等待该任务，再恢复用户输入。截图仍来自该原生页面。
- 两个持久 Context 的 cookie/localStorage/IndexedDB/CacheStorage 隔离；关闭并重建标签后数据保留（不是重启应用持久化证明）。
- 维护视图只能在本 Context 的实际标签确认关闭后清理；取消请求与仍有同 Context 标签的清理被拒绝。
- 完整 `Storage.clearDataForOrigin(*, all)` 清掉会话 A 的两个 origin，而会话 B 的同类数据保持原值；随后等待缓存、HTTP 认证、证书例外、连接清理回执。
- Windows `cargo tree -p nomifun-desktop --target x86_64-pc-windows-msvc --no-default-features` 无 CEF/nomifun-browser-macos 依赖；这不是 Windows GUI 回归。

已新增 11 个适配单测：包括原生字段字符串保真、路径别名/逃逸、协议会话匹配、队列溢出、同步 frame 事件与 barrier、单请求拒绝、弹窗等待不耗尽协议期限。Mac desktop `cargo check --no-default-features` 通过。

### 固定 CEF 版本的清理问题

失败记录保留在 `run-KEUS7s`、`run-MIJDhh`、`run-6C50Fp`、`run-dakFHg`、`run-D2KdNe`。
`run-D2KdNe/native-result.storage-trace.json` 显示所有常规删除任务完成，而 `StoragePartitionImpl` 的
`data_type=16`（DeclarativePerformanceObserver）没有结束事件，完整 clear 命令因此超过 30 秒。
固定的 Chromium 152.0.7977.83 将此 API 标记为 experimental，且 base feature 默认开启：
[上游特性声明](https://github.com/chromium/chromium/blob/152.0.7977.83/third_party/blink/renderer/platform/runtime_enabled_features.json5)。

引擎在建立任何 Context 前通过公开 command-line callback 禁用 `DeclarativePerformanceObserver`。
未关闭 sandbox、签名验证、CORS 或网站安全校验；未缩减 `storageTypes=all` 的清理范围。
`run-K8fR9f/native-result.storage-trace.json` 不再出现该实验性任务，全部 native 清理回执和隔离断言通过。
升级 CEF 时须重新评估并验证此版本规避项；不要把超时改为成功。

### 最新扩展证据

`dist/browser-cef-native-smoke/run-HeFI4F/native-result.json`：26 项顶层断言通过，`shutdown_complete=true`。
在上一轮 24 项基础上，额外通过了共享 iframe 输入几何套件及 Agent iframe 上传套件。
输入模块 23 个定向单测与适配模块 11 个单测均通过。

- 通过跨站/嵌套/同进程 iframe 的可信 click/type/press/select、焦点和父 frame 遮挡保护、祖先导航失效。
- 通过旋转、独立 transform、透视、嵌套透视、原生浏览器缩放与 root scroll 后输入。
- 修复 Mac CDP 键盘编辑命令缺失；单选菜单采用原生键盘 type-ahead，而 Windows 保持原有方向键路径。
  原生 fixture 包含对已有文本的替换，最终中文值正确。菜单选择的完整边界矩阵仍在 TODO。
- 通过 OOPIF 首次观察前 chooser 拦截、同进程/跨进程标准和自定义 input、父页委托 chooser、中文文件名/快照内容、过期文档拒绝后重新观察成功。
- 修复 Mac 主页面/同进程 frame 的 chooser 拦截遗漏；native 输入冻结后先 drain 网站 dialog，再等待协议策略，避免 admission 被已有弹窗卡住。
- shared frame/upload conformance cases 通过 native ports 供两个平台使用；尚未重跑 Windows GUI。

## 仍需完成

- 生产 main.rs 注入/生命周期接入；已有独立 Mac Host 的 fixture 证据，不等于产品会话 slot、遮挡、切换和退出验收。
- 会话持久化、跨会话隔离、精确清理及失败回执；iframe/Shadow DOM/stale ref 的完整语义和输入路由。
- 用户与 Agent 上传下载、托管 popup、permission、crash、页面主动关闭/应用退出。当前未实现的 popup/文件/下载默认原生入口被阻断；不可以此替代完整功能。诊断采集尚标为 unavailable。
- 正式配置与 Role defaults → Nomi Engine → step-3.7-flash 真实前端闭环。
- Mac 产品包的 CEF framework/helper 分发、签名、公证与安装启动；当前仅 native fixture 包。
- Windows 依赖图隔离和原生回归清单；Linux、系统浏览器 attach 仍后置。

本轮保存为开发检查点，剩余实现与验收在接续 TODO 中分开列出。上一版产品 DMG 不含 CEF，也不能作为这次实现的发布包。
