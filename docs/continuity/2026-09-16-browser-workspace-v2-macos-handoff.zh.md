# Browser Workspace v2：macOS 实施移交

> 状态：Windows 完成后执行；本文不声称 macOS 已支持。
>
> 基线：`docs/specs/2026-09-13-browser-workspace-v2.zh.md` 与
> `docs/specs/2026-09-15-system-browser.zh.md` 的 2026-09-16 冻结合同。

## 1. 不可改变的产品合同

- 会话 Browser 必须是同一个真实原生 `WKWebView`；用户看到的页面就是 Agent 操作的页面。
- 不使用 iframe、JPEG/PNG 连续帧、screencast、Canvas 远程桌面或另一个 Headless 页面冒充。
- 只有 `UserReady` 与 `AgentRunning` 两态。Agent 工作时用户不能操作；原子动作 settle 后才恢复用户输入。
- 不增加 takeover/交还、测试步骤、问题、终端、控制台、DevTools/F12、popout/归位或 Browser 设置中心。
- `nomi_local_websearch` 继续使用隔离 Headless Search owner；不得读取会话 Browser 登录态。
- `nomi_system_browser` 在没有经过验证的 macOS attach-only 路径前必须显示 unavailable；不得扫描、复制或启动用户
  默认浏览器 Profile 作为降级。
- macOS 失败时明确 unavailable；禁止退回 DOM `click()`/`dispatchEvent()`/直接改 value 或帧流。

## 2. 最小实现切片

1. 在 Tauri 主窗口内挂载真实 child `WKWebView`/`NSView`，复用现有 Conversation Workspace、Tab、revision、
   RunGuard 与 renderer slot 合同。页面 WebView 不获得 Tauri capability、local trust 或 backend credential。
2. 为每个 Conversation 建立应用拥有的网站数据边界。先用最小签名 app 实测目标 macOS/WebKit 的持久 data-store
   能力，再固定实现；不能悄悄共用系统 Safari 数据，也不能借用默认全局 store 后声称会话隔离。
3. 实现宿主内部语义观察桥，只返回当前已绑定页面的有界 accessibility/DOM 语义。WKWebView 没有可假定的公开 CDP；
   不把 Web Inspector 当生产控制接口。
4. 用 AppKit/WebKit 可公开验证的原生事件路径实现 click、hover、type、press、wheel 与同文档 drag。每种操作先在
   真实 fixture 证明默认行为、focus、IME 和 `event.isTrusted`；无法证明的动作返回 unsupported。
5. 实现 native input gate：run accepted 后立即阻断用户鼠标、键盘、拖放与触控板输入，同时保留 Agent 的宿主输入
   路径；Stop 期间保持锁定，settle 后再恢复 first responder 与用户输入。
6. 接入 popup、网站 dialog、permission、file chooser、download、crash、close 与 app shutdown。只实现首发流程所需
   的最小集合；没有证据的边界明确拒绝，不建设第二套管理平台。
7. Interactive Browser 沿用系统网络、DNS、代理、证书、localhost/LAN 与 WebSocket/HMR；仅校验顶层无凭据
   HTTP(S) URL 和拒绝特权 scheme。后台 Search/Render 继续使用自己的严格公网出口边界。
8. 站点数据清理必须只针对当前 Conversation 数据边界，先关闭该会话页面，再等待 WebKit 清理回执；失败不得显示成功。

## 3. 必须先做的原生 spike

- 签名最小 app 中：真实中文输入、组合文本、Tab/快捷键、hover、右键/双击、wheel、pointer/drag 默认行为。
- 同源 iframe、跨站 iframe、Shadow DOM、父层遮挡、页面导航后的旧 ref 失效。
- Agent 输入期间用户真实鼠标/键盘无法进入页面；停止/崩溃/窗口切换后不会留下按键或拖拽状态。
- hide/show、resize、Conversation 切换不重建页面；最小窗口仍遵守 880x600 桌面合同。
- 网站数据隔离与清除不会影响另一个 Conversation，也不会接触 Safari Profile。
- F12/Inspect/Web Inspector 没有产品入口。

若任何“原生输入”只能靠页面脚本合成事件完成，停止该动作的实现并记录 unsupported；不要扩大架构补偿。

## 4. 验收矩阵

- 复用 Windows fixture 语义：navigate/observe/click/type/press/wheel/select、同文档 drag、Stop、dialog、permission、
  popup、上传、下载、crash、close、shutdown 与站点数据清理。
- 验收必须记录页面侧 trusted 事件、默认行为、焦点与最终 DOM 状态，不能只看截图。
- 跑 renderer 定向测试、Rust 平台测试、`bun run check:desktop-ui-boundary`、
  `bun run check:browser-platform-boundary`、typecheck、i18n 与 macOS 签名包 smoke。
- 完成前 UI/文档保持 macOS unavailable；Windows 证据不能替代 macOS 证据。

## 5. 明确后置

- `nomi_system_browser` 的 macOS Safari/Chrome attach；只有官方、用户授权、attach-only 且不迁移 Profile 的路径
  通过后再启用。
- Linux/WebKitGTK。
- 跨进程 HTML drag、popout、DevTools、测试工作台与浏览器高级网络设置；这些不是本次移交任务。

## 6. 移交结果格式

接手方应返回：目标 macOS/WebKit/Tauri 版本、每个 native fixture 的通过/失败证据、明确 unsupported 的动作、
签名产物哈希与未运行项。不得用“编译通过”代替真实 WKWebView 输入和数据隔离证据。
