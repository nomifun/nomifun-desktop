# 原生浏览器验收

本文记录可重复的验收入口、证据边界与尚待完成的实机覆盖。产品合同见
[浏览器工作区](../architecture/browser-platform.zh.md)；源码和本次运行的回执决定实际结果。

## 选择验证范围

| 变更 | 最小验证入口 |
| --- | --- |
| 共享浏览器状态与布局 | 受影响的 `ui/` 测试、`bun run check:browser-platform-boundary`；renderer 变更另跑 `bun run check:desktop-ui-boundary` |
| 实际浏览器操作期间的占用 | 受影响的 [Browser Role owner](../../crates/backend/nomifun-app/src/router/engine_browser_tools.rs)、[coordinator](../../crates/backend/nomifun-browser-platform/src/run_guard.rs)、原生输入适配器、HTTP 与 BrowserPanel 测试；原生和生产 GUI 证据单独取得 |
| Rust 浏览器域与宿主接缝 | `bun run test:browser`，或按 [Rust 测试入口](../../scripts/test-rust.mjs) 选择受影响的 crate |
| Windows 原生导航、Profile、输入与弹窗 | [browser_workspace_smoke.rs](../../apps/desktop/examples/browser_workspace_smoke.rs) 及其 [support 模块](../../apps/desktop/examples/support/) |
| macOS 主线程与退出 | [macos_appkit_main_loop_smoke.rs](../../apps/desktop/examples/macos_appkit_main_loop_smoke.rs)、[macos_appkit_termination_probe.rs](../../apps/desktop/examples/macos_appkit_termination_probe.rs) |
| macOS WK 导航、权限与站点 | [macos_browser_compatibility_smoke.rs](../../apps/desktop/examples/macos_browser_compatibility_smoke.rs) |
| macOS 固定启动与最终退出回执 | [fixture 打包](../../scripts/validation/package-macos-native-fixture.mjs)、[application controller](../../apps/desktop/examples/macos_application_controller.rs)、[WK App 验收](../../scripts/validation/run-macos-wk-app-acceptance.mjs) |
| 生产窗口交互 | [browser-main-gui-fixture.mjs](../../scripts/validation/browser-main-gui-fixture.mjs)，配合真实 Tauri 窗口操作 |

参数与平台前提以对应 runner 的 `--help`、入口解析和源码为准。使用隔离数据目录，保留
源码基线、工作树差异、构建身份、运行参数、退出码与清理回执。每次运行重新确认实际进程、
动态端口和窗口；旧 PID、临时文件或另一份构建不证明当前实例身份。

后台 Chrome 引擎的 pipe EOF、无窗口启动、stale singleton lock 与 OOPIF attach 需要
对应平台的独立验证，入口见 [nomi-browser-engine](../../crates/agent/nomi-browser-engine/src/lib.rs)。
原生内嵌面板通过不替代后台引擎的生命周期与清理证明。

## 必须区分的证据

- 静态检查、unit、原生 smoke、生产 GUI、外部站点与签名发布各自报告。构建和签名成功不证明原生运行成功。
- 同一页面的 target、navigation sequence、attempt、document 身份与服务端主文档请求计数配套记录；URL 变化、SPA、子资源、重定向与主文档重复请求分开。
- Stop、Pause/Resume、queued/claimed popup、迟到 callback、失败后保留文档、同文档与跨文档 Back/Forward 按真实生命周期验证。Resume 不自动重放旧候选或导航。
- 浏览器占用与 Session 状态分别验证：实际 managed 浏览器工作取得锁，原生操作、按下输入的清理及适用的 Effect 结果记录完成后才局部释放。同一 Turn 继续思考或使用其他工具时，用户可浏览和关闭 runtime。等待中的对话框或 retained 原生工作在安全完成前继续持锁；请求取消或清理不确定不证明已经释放。覆盖按钮/快捷键关闭、exact generation 重开、占用与关闭竞争、调用方丢弃及迟到 callback，不能重放用户命令。必须覆盖 observe → release → 新 guard 下 act，确保安全清理不会让全部观察立即失效；页面导航、标签关闭和 runtime 换代后仍须拒绝旧引用。
- Cookie、localStorage、IndexedDB 覆盖关闭重开、进程重启、Session/store 隔离与临时 store 的原生清理完成；不能用导入日常浏览器 Cookie 代替。
- 网站对照固定同机同网与受控初始状态，分别记录内嵌身份和系统浏览器，独立静置至少 60 秒。正常、验证码、版本拒绝、持续加载与循环分别报告；不为通过而解验证码或反复刷新。
- 共享 renderer 仅验证桌面窗口，最低 880×600。组件模拟几何不能代替原生像素、输入或焦点证据。
- `CORE_PASS`、单个 exit 0 或 entry check 不能替代站点完成、准确 native exit、controller final receipt 与资源清理证明。

## 尚待完成的覆盖

截至 2026-10-15，Windows 的导航、Profile、弹窗恢复、同文档 History 与最低桌面窗口已有
历史定向实机记录；这些结果不扩大为所有平台或所有活动 Agent 场景的通过。

2026-10-18，使用 `cargo build --profile test -p nomifun-desktop --example browser_workspace_smoke`
构建后，隔离 Windows WebView2 的 `browser_workspace_smoke --interaction-only` 实际运行以 exit 0 通过。
本地临时 fixture 覆盖 observe → release → 新 guard 下使用前次引用 act、单次可信原生点击、
原生用户策略恢复和调用间用户命令。等待中的链式对话框保留原始 callback，并在最终回复解锁前拒绝
用户关闭及销毁。用户关闭后，原生 controller 与可用协议端点均已销毁；只读 OS 句柄证明准确的浏览器
主进程在关闭前存活，并在关闭后的五秒观察上限内退出。同一 Session 随后能够创建新 runtime generation，
临时原生 profile 清理也已通过。进程见证不保留 COM environment，等待前释放测试 view 句柄，且不终止浏览器进程。

两个平台的生产 GUI，以及 macOS 的本次原生交接，仍待验收。上述 Windows fixture 不证明安装包行为、
外部站点兼容或每个子进程退出。完整生产覆盖仍须包含浏览器完成后继续思考或使用无关工具、
释放后的人工输入及 runtime 关闭、等待的导航/对话框完成、结算或解锁失败与 exact terminal 恢复。
在生产窗口重验 observe → release → act，再由用户导航或关闭浏览器后在同一 Turn 调用；
安全清理须保留仍可用的观察，旧代际及旧 owner 清理不能影响替代实体。
当前源码的后端/unit、真实原生输入及生产窗口证据分别记录。

macOS 最新的 popup Pause、同文档 History 与启动回执修复已有源码、定向检查及构建记录，
仍缺融合后 WK 原生运行和生产 GUI 的完整证明。后续需要：

1. 实际运行当前完整静态 UI App 与两个 smoke，核对固定 descriptor、LaunchServices admission、构建身份、native exit 和最终清理回执。
2. 在确定性 queued/claimed 阶段执行 popup Pause/Resume，核对准确 child 关闭、活动标签、父文档 nonce 与主文档请求计数。
3. 完成 Stop、失败恢复、旧控制与迟到 callback、输入锁、同文档及跨文档 Back/Forward 的实机覆盖。
4. 补持久 store 重启、Session 隔离和临时 store 清理证明。
5. 在生产窗口操作地址栏、标签、菜单、暂停、显隐与恢复，保留安全导航时间线和截图。
6. 对 Safari17、NativeWebKit 与系统 Safari 做独立站点对照；原始百度重复导航现场仍未完成复现、归因或修复证明。

最低支持系统、Intel、Developer ID 签名与公证发布没有由 Apple Silicon 开发构建自动覆盖。
工具限制需要如实记录；无法启动的用例继续标为未运行，不改变启动、权限或清理合同来取得通过。

## 历史记录

日期化施工报告、交接单、逐次日志和截图从工作树退休，通过 Git 历史追溯。先使用
`git log --all -- docs/reports docs/handoffs` 找到删除前提交，再使用
`git show <commit>:<path>` 读取相应文件。历史记录只证明其中明确写出的基线与范围，
不充当新源码的验收或当前开发要求。
