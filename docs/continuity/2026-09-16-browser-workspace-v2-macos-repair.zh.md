# Browser Workspace v2：macOS 后续修复与输入定位

本轮承接 `d05c6e9ab` 的验收失败，修复了 Browser 不可用时的产品状态，并进一步定位 WKWebView
输入问题。**尚不具备跨平台上线条件；没有开启 macOS Browser availability。**

## 已修复

`ui/src/renderer/pages/conversation/Browser/BrowserWorkspacePanel.tsx`：

- 初始宿主响应前显示“正在打开浏览器”，不提前显示“可手动操作”或引导用户输入地址。
- 宿主失败、原生 unavailable 事件、输入门失败时显示“浏览器尚未就绪”。
- HTTP/Tauri 错误映射为本地化产品提示，不再把接口路径、会话 ID、JSON、原始异常展示给用户。
- `BROWSER_NATIVE_SURFACE_UNAVAILABLE` 明确解释为当前设备暂不支持，不提供无效重试。
- 临时连接/操作失败保留用户主动重试，成功后恢复就绪；站点数据清理失败继续使用原有重建流程。

新增四项回归，覆盖初始化等待、501 不支持、临时失败重试恢复、原生 unavailable 后隐藏页面；
既有输入门失败测试增加“不显示可手动操作”的断言。

本轮检查：Browser/SystemBrowser/ChatLayout **102 PASS / 0 FAIL**；desktop boundary、browser boundary、
typecheck、i18n 和 `bun run build:ui` 均通过。没有改 Rust 生产宿主或 Windows wry 补丁。

## 新的原生定位证据

probe 在整个手势期间保留一个公开 `CGEventSource`，使用公开 `CGEvent.setSource`，仍只向自身 PID
投递事件。新增选项 `--event-source private|session` 和逐事件诊断，未修改任何页面断言。
这里的 `private` 指公开的 `CGEventSourceStateID.privateState`，不是私有 WebKit API。

```sh
node scripts/validation/run-macos-browser-native-probe.mjs --probe input --transport pid --event-source private --reuse-signed-app --output dist/browser-macos-validation-20260916/reproducible
node scripts/validation/run-macos-browser-native-probe.mjs --probe input --transport pid --event-source session --reuse-signed-app --output dist/browser-macos-validation-20260916/reproducible
```

首次运行这些新选项前需重新构建 probe，并保持此前授权对应的签名身份。runner 会检查结果中的
eventSource 是否匹配请求，拒绝把不支持新选项的旧签名包当成已执行新场景；无效参数在构建前退出 2。
本机用原 Developer ID 重新构建并验证签名，没有重新申请系统权限。

| 公开事件源 | 本机实际结果 |
| --- | --- |
| privateState | down/drag 到达时 `sourceLeftDown=true`，但 `NSEvent.pressedMouseButtons=0`、`sessionLeftDown=false` |
| combinedSessionState | down/drag 到达时 `pressedMouseButtons=0`、`sessionLeftDown=false` |
| 两者页面结果 | `buttons=0`、`capturedMoves=0`；鼠标与拖拽断言失败，退出 1 |
| 两者其他断言 | 点击、焦点、组合文本、trusted 事件、滚轮、局部输入门原型、光标前后不变通过 |

结果文件：`input-pid-1789556614577.json`（private）、`input-pid-1789556660672.json`（session），
原件在命令 output 目录，也复制到 `dist/browser-macos-repair-20260916/`。
本轮日志、构建结果与主程序哈希见该目录 `repair-evidence.json`、`input-private.log`、`input-session.log`、
`probe-build.log`、`probe-argument-checks.json` 和 UI 检查日志。

这排除了“没有保留独立事件源导致手势状态丢失”这个具体假设：独立源已正确记录手势状态，
而 WKWebView 所读的系统按钮状态仍不同。不是增加权限或延时就已修复，也不能据此断言所有未来公开 API
都不可能实现。生产宿主、Stop/settle、真实模型和完整原生矩阵仍然未完成。

## 待明确的内核选择

[工作 prompt](2026-09-16-browser-workspace-v2-macos-work-prompt.zh.md) 明确要求“真实原生 WKWebView”；
[handoff](2026-09-16-browser-workspace-v2-macos-handoff.zh.md) 要求输入不能满足时返回 unsupported，
不得通过私有 API、DOM 事件合成或第二个页面补偿。因此，没有把“修复至可上线”自动解释为允许更换内核。

已向用户提出是否允许评估并改用 **原生 child NSView 中的 Chromium/CEF**，尚未据此修改生产架构。
公开 API 初审支持将其作为候选，但还没有实机通过证据：

- [CEF macOS window API](https://github.com/chromiumembedded/cef/blob/master/include/internal/cef_mac.h) 提供 child view 挂载；候选方案不用 windowless/帧流。
- [CEF BrowserHost API](https://github.com/chromiumembedded/cef/blob/master/include/cef_browser.h) 提供页面级鼠标/键盘/滚轮输入，以及无需打开 DevTools 界面的协议调用。
- [CEF RequestContext API](https://github.com/chromiumembedded/cef/blob/master/include/cef_request_context.h) 提供独立上下文，需实测会话存储隔离与清理。
- [tauri-apps/cef-rs](https://github.com/tauri-apps/cef-rs) 声明支持 macOS arm64/x86_64，并提供 framework/helper 打包示例。

若允许，先证明 child NSView、同页可信输入、按钮/拖拽、中文、用户输入门及停止释放，再接入已有
Workspace/Tab/RunGuard/Role。代价是新增内核 framework/helper 的分发、签名、公证与维护；
不能承诺仅更换库就能通过上线验收。Windows 现有 host 和 wry 补丁应保留。

本轮没有重新生成产品安装包；前一轮签名 DMG 对应 `6a2a30785`，不包含这次 UI 修复，不能作为最终上线包。
