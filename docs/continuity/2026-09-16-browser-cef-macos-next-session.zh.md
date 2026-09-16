# macOS CEF 下一次接续 TODO

## 收工约定

用户在本轮末明确要求：暂停耗时的全面验证，把完整测试记录为后续 TODO，由用户另找时间继续。
因此本次保留开发检查点，不启动后续全量测试、真实模型或发布流程，也不设置定时/后台任务。

**当前状态是 CEF 核心适配和独立 Mac Host 已有原生证明，但整体跨平台开发尚未全部完成。**
下方分别列出剩余开发和后续验收，不能把开发缺口归类成“只是没测”。

## 固定架构与背景

- Windows 保留 WebView2、COM/CDP 宿主和 wry child-close 补丁；Windows 目标没有 CEF 依赖。
- macOS 使用 Tauri UI + 独立 CEF child NSView；复用 Workspace/Tab/RunGuard/Role 和语义算法。
- 用户看到的与 Agent 操作的是同一个原生页面；没有外部 Chrome、headless 替身或帧流方案。
- 不再回到 WKWebView 输入实验。本轮 CEF 决策已获用户确认，无需重新征求方案同意。
- Linux、系统浏览器 attach 仍后置；桌面边界仍为 880×600。
- 分支：`rf/agent-capability-platform-v2`；本轮开始时 HEAD：`025e6b881`。恢复工作时先检查实际 Git 状态。

详见[架构决策](2026-09-16-browser-platform-architecture-decision.zh.md)、
[实施与证据记录](2026-09-16-browser-cef-macos-implementation.zh.md)。

## 已完成且有证据的部分

- macOS 专用 CEF crate、进程内页面协议、主线程调度、沙箱 helper、五种标准 helper bundle、原生子视图关闭与全局 shutdown。
- 修复 CEF Rust 设置字符串丢失、缺少 typed helpers、消息泵调度和路径别名/越界创建问题。
- 同页可信 click/type/wheel/drag；Mac 编辑命令与单选菜单的原生 type-ahead 路径。
- 独立 Mac BrowserRuntimeFactory、共享 pending-work 所有权、RunGuard 停止后 settle 再解锁、原生网站 dialog 延迟回复。
- 同进程/跨站/嵌套 iframe、仿射/透视/缩放/滚动输入与焦点、遮挡、过期引用拒绝。
- Agent 的标准 input 和自定义按钮上传，同进程与 OOPIF、中文文件名/内容、授权文件快照、父页委托选择器、过期文档拒绝。
- 两个会话 Context 的 cookie/localStorage/IndexedDB/CacheStorage 隔离、标签重建保留、会话级跨 origin 清理与其他会话保留。
- 用户文件选择器尚未完成。已验证的 Agent 上传不能当作用户文件选择器完成。

最新原生检查：`dist/browser-cef-native-smoke/run-HeFI4F/native-result.json`，26 项顶层断言全部通过，
包含 iframe 输入与上传的内部断言；`shutdown_complete=true`。
该目录 `artifact.json` 明确标记 `productAcceptance=false`，架构 `arm64`。
fixture 主程序 SHA-256：`294304103b44ad7f21da4d748863ad0d5f26760cb91946734baf318a577106a5`。

其他已有检查：适配 crate 11 个单测通过，输入模块 23 个定向单测通过，Mac desktop 编译检查通过，
Windows desktop 目标依赖图中 CEF 条目为 0。没有用这些代替 Windows GUI 验收。

收工时未发现仍运行的本次 CEF fixture 进程。

## 下次优先完成的开发 TODO

- [ ] **生产组合与启动退出**：当前 `main.rs` 仍只在 Windows 注入 Browser factory。Mac 需要在 Tauri `build` 后、`run` 前初始化 CEF，注入独立 Mac Host，并让退出协调器先完成 Browser teardown 再 shutdown CEF；保留已有 PTY 清理。缺少有效 bundle 时明确 unavailable。
- [ ] **托管新窗口**：CEF `on_before_popup` 目前拒绝所有未托管弹窗。实现 Conversation Tab admission、opener/context 关系、首个脚本前输入策略、页数限制、取消/关闭边界及失败回收；不得把取消 popup 后另行导航作为完整 popup 实现。
- [ ] **用户文件选择器与下载**：当前 CEF 默认文件对话框被取消，下载被拒绝。补用户原生 picker、选择期间锁定/隐藏/关闭时取消与回执、用户下载历史/取消、Agent PreparedBrowserDownload 生命周期和发布。
- [ ] **权限**：当前 native permission handler 明确拒绝。补 UserReady 的可见活动页请求和回复、组合 camera/mic 的完整披露、导航/隐藏/RunGuard 开始时失效、超时和持久授权边界。Agent 不得继承用户的设备授权。
- [ ] **其他命令**：Mac Host 的 OpenExternal、OpenDownloads、CancelDownload 和 Permission 分支仍是 UnsupportedAction，需补原生实现与调用权校验。
- [ ] **诊断与恢复**：Mac 页面诊断当前标记 unavailable。补同页及子 frame 的有界诊断路由；补渲染进程 crash 后的重建/恢复、失效标识与旧句柄拒绝。
- [ ] **生命周期收尾**：页面主动关闭已有 native close watcher，但未单独完成产品场景验证；审查关闭中的创建、surface/layout 取消、Context 释放以及 pending native callback 的收尾。
- [ ] **生产打包**：CEF framework + 五类 helpers、许可证、版本/校验记录、cmake/ninja 构建依赖、签名顺序/公证/安装路径。当前 runner 只是 fixture 打包器。明确 arm64、Intel 与 Universal 的支持策略，不能仅 lipo 主程序就宣称 Universal。
- [ ] **最终整理**：整理新增代码和模块边界、删除废弃实验源/替身；保留失败证据摘要与必要回归。不要删除开发机真实数据或其他会话改动。

## 用户安排时间后再做的完整验收 TODO

- [ ] 正式产品会话内 Browser slot：880×600、Retina、resize、遮挡、hide/show、会话切换、焦点与状态保持。
- [ ] 原生输入与 Stop 全矩阵：用户实际键鼠/拖动/快捷键/辅助功能、浏览器之外和其他应用不受干扰；按键/拖拽半途取消、dialog 中 Stop、迟到回调和输入门失败。
- [ ] macOS select 完整矩阵：单选、多选、中文/空/重复或前缀相似标签、动态 options、取消键事件、保留/清除禁用选项。已通过的 iframe `Two` 选项不能替代全部选择控件语义。
- [ ] Shadow DOM、页面跳转/iframe 替换导致的 stale、HTML drag-and-drop、跨 frame 拖拽、快捷键和单次截图的视觉内容。
- [ ] 持久数据跨应用重启与进程重建、临时会话无落盘登录态、多个 origin/分区存储/ServiceWorker、清理取消/失败/重试；现有“标签重建”证据不是“应用重启”证明。
- [ ] 用户与 Agent 上传/下载、原生 picker 取消与迟到确认、popup/opener/window.close、beforeunload、权限、崩溃、关闭退出、无残留 helper。
- [ ] 正式配置 → Role defaults 重载 → Nomi Engine → 已授权 `step-3.7-flash` 的真实前端任务。凭证走现有配置链，不进入构建环境/命令行/日志；没有运行真实模型就记录未执行。
- [ ] Mac 产品包签名、公证、安装后启动/实际 Browser 操作/退出，以及旧安装升级；当前旧的 `NomiFun_0.7.6_aarch64.dmg` 不含本次 CEF，不能作为新发布包。
- [ ] Windows 独立 WebView2 原生回归；共享语义/测试代码有调整，依赖图隔离不等于 Windows 原生通过。
- [ ] 发布前再执行必要的整体验证和最终 Git/产物证据。不要现在重复运行已通过的全套检查。

## 需要保留的两个事实

1. CEF 152.0.6 / Chromium 152.0.7977.83 的完整存储清理曾卡在实验性 DeclarativePerformanceObserver 的 `data_type=16`。
   当前通过公开启动 callback 禁用该实验性特性，仍执行 `storageTypes=all`；`run-D2KdNe` 失败 trace 与 `run-K8fR9f` 修复 trace 已保留。
   升级 CEF 时重新评估这个固定版本规避项。
2. 早期额外 Cocoa IME 实验的 `compositionend.isTrusted=false` 失败仍存在于证据记录。
   普通 Unicode Agent 输入通过，不等于物理中文 IME 候选窗/所有组合事件验收通过。

## 下次可直接使用的提示

继续 `docs/continuity/2026-09-16-browser-cef-macos-next-session.zh.md` 中的工作。
先读取实施记录并核对 Git 状态，保持 Windows WebView2 / Mac CEF 两种架构。
优先完成剩余生产开发，不重新争论 CEF 选型，不把已通过的 native fixture 当作正式产品验收。
全面测试和真实模型测试由本次新任务的明确范围决定；本次收工没有安排后台继续执行。
