# Browser Workspace：Windows / macOS 架构决策

2026-09-16 用户确认 macOS 内嵌浏览器允许改用 Chromium/CEF，并进一步明确：
**Windows 和 Mac 使用不同架构。** 本决策覆盖此前 macOS work prompt、handoff 与 v2 规格中
要求使用 WKWebView 作为会话浏览器的条款。历史 WKWebView 失败记录仍保留为证据。

- **Windows**：保持现有 WebView2 原生宿主、COM/CDP 输入路径和 wry child-close 补丁。
  不切换到 CEF，不在 Windows 目标编译、链接或分发 CEF。
- **macOS**：独立 CEF 宿主，以真实 child NSView 嵌入现有会话 Browser slot；不改成外部窗口，
  不使用 windowless/帧流展示，不让 Agent 操作另一个隐藏页面。应用 UI 仍使用 Tauri。
- **公共部分**：复用 Workspace、Tab、RunGuard、Role、renderer slot、权限和工具协议；
  允许共享语义算法，不为了复用将 Windows 迁往 Mac 的底层实现。
- **输入**：宿主调用 CEF 的公开页面级接口，用户与 Agent 操作同一可见实例。Agent 运行时阻断用户
  对该页面的输入，Stop 后等待原子动作 settle 并释放按键/拖拽，再恢复用户输入；不干扰其他应用鼠标。
- **隔离与分发**：各会话使用应用拥有的独立 request context / 存储；网页不拥有 Tauri IPC 或密钥。
  CEF framework 和必要 helper 属于浏览器内核组件，须验证来源、版本、沙箱、签名、公证和退出。
- **上线门槛**：先通过 Mac 原生子视图输入 fixture，再接生产宿主和真实模型；测试、构建或静态 API
  存在不等于上线通过。Windows 继续执行其独立回归。系统浏览器 attach 与 Linux 不随此次变更启用。

开发基线：`025e6b881`。实现和通过证据须另行记录，本文只记录已批准的架构与边界。
