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
| Windows 实际 Desktop Quit 与 managed Browser parent-death | [真实宿主 runner](../../scripts/validation/run-windows-desktop-lifecycle.ps1) 与 [debug fixture](../../apps/desktop/src/lifecycle_quit_fixture.rs) |

参数与平台前提以对应 runner 的 `--help`、入口解析和源码为准。使用隔离数据目录，保留
源码基线、工作树差异、构建身份、运行参数、退出码与清理回执。每次运行重新确认实际进程、
动态端口和窗口；旧 PID、临时文件或另一份构建不证明当前实例身份。

后台 Chrome 引擎的 pipe EOF、无窗口启动、stale singleton lock 与 OOPIF attach 需要
对应平台的独立验证，入口见 [nomi-browser-engine](../../crates/agent/nomi-browser-engine/src/lib.rs)。
原生内嵌面板通过不替代后台引擎的生命周期与清理证明。

Windows 的实际 Quit runner 使用当前 debug Desktop bin，本轮先执行
`cargo build --locked --profile test -p nomifun-desktop --bin nomifun-desktop --features nomifun-app/browser-conformance`，再以真实构建输出路径运行：

```powershell
powershell -NoProfile -File scripts/validation/run-windows-desktop-lifecycle.ps1 -DesktopExe <实际构建目录>/nomifun-desktop.exe -Mode quit -KeepArtifacts
powershell -NoProfile -File scripts/validation/run-windows-desktop-lifecycle.ps1 -DesktopExe <实际构建目录>/nomifun-desktop.exe -Mode parent-death -KeepArtifacts
```

runner 自建带随机 sentinel 的直接 Temp 子目录，固定其 data/work 与 shell Profile；debug fixture 在任何数据初始化及
single-instance 注册前验证规范化路径，使用本次独立 identifier，跳过其协议系统注册，不连接日常 App、Chrome 或外部模型。
实际 AppServices 已装配的 managed Browser owner 创建一张持续活动的本地页面；关闭走正式 `app.exit`、
`ExitCoordinator` 与 `DesktopServer::shutdown_all_async`。fixture 通过原 WebView2 environment 的 `GetProcessInfos`
取得 shell 与 managed 的 browser、renderer、GPU 和 utility 库存，runner 在动作前持有每个准确进程的只读
SYNCHRONIZE 句柄，再分别检查宿主退出、coordinator 收据、原 loopback 关闭及这批原生进程退出。
parent-death 模式只终止本次新建的宿主 Process 对象，不终止树或浏览器，不把该模式写成正常 Quit 清理通过。
失败保留隔离现场及日志，不能以 PID 消失、旧回执或目录删除替代原生句柄证明；成功可保留回执，或在精确路径校验后删除临时现场。
runner 另持有其自建且位于 Desktop 树外的独立进程，检查退出前后均存活，最后仅清理本次独立进程。
这不等于已验收注册 OS 服务。此入口的构建和各次原生运行分别记账；库存之外的逃逸进程、
macOS/Linux、生产 GUI 或签名安装包不由该入口证明。

活动 Agent 的组合验收在同一 runner 追加 `-WithAgentResources -NodeExe <现有 Node 可执行文件>`。
该验收选项只创建本次 Temp 根内的请求标记；debug host 在原隔离验证后，使用其既有本地身份走正常
owner setup 和密码登录。原 trust 不离开宿主；侧车只取得本次私有 JWT，runner 在 finally 中删除该文件，
`-KeepArtifacts` 也不保留它。正常产品认证和 WebView2 环境守卫不改变。
[资源侧车](../../scripts/validation/desktop-agent-resources.mjs) 经普通认证 API 配置测试模型、MCP 和 Session，
由真实 Agent 各启动一次 Bun pipe、ConPTY 程序，并保持一次真实 stdio MCP `tools/call` 在途。
模型响应受控，工具准入、Effect、进程和 MCP owner 均为原生产实现；不提前返回 MCP 结果换取退出。
MCP 夹具继续读取 stdin，响应原确切取消并在原 stdin EOF 后退出。
runner 核对每个进程自身 marker、执行文件、创建时间和原宿主父链，再以只读 SYNCHRONIZE 句柄
把三个确切进程加入原 WebView2 退出库存；模型侧车与树外 sentinel 持续至观察完成，最后仅按各自
原 Process 对象清理。`native-exit-proof.json` 保存原生库存与二进制、driver、侧车身份，
`agent-resources-ready.json` 保存原 Session/Operation、两次原启动及一次 MCP 调用证据，
`agent-credential-cleanup.json` 只记录私有凭据清理结果。

在组合验收的 `-Mode quit` 上另加 `-StopAgentBeforeQuit`，可单独验证用户先停止原 Agent、
Runtime 自然注销、原 Session 后台进程继续存活，再正常退出应用。侧车从真实 Activity 的
`can_stop` 行取得原 `session_turn` target，走普通 Stop API；不缩短 operation、不新建任务或重跑工具。
`stop-agent-ready.json` 和 `agent-stop-observation.json` 记录原 Runtime、两条完整 ProcessInstance 身份、
MCP Unknown 与无自动重放的事实，`agent-stop-native-observation.json` 另记原 pipe/ConPTY 句柄仍存活。
MCP 资源是否已经退出按实记录，业务 Unknown 保留；最后仍须检查最初捕获的全部退出句柄。

2026-10-19，checkpoint `a9c41c38fa` 与 `origin/main@084c7dc8d0` 整合并冻结源码后，重新构建的隔离 Windows debug Desktop（SHA256
`661681a63e3b15fa4190ebefcf78268940b533c4b4f69cd36809426193bdf84a`，
构建输出修改时间 UTC `2026-10-19T06:14:41.4297828Z`，341702144 bytes）通过以下三个真实组合用例。
三轮均使用全新 Temp 数据集及 Profile，包含原 OAuth 010 和调整后的 Cron 011，不续用旧 Cron 010 数据库。
每轮动作前持有原 shell/managed 的 10 个 browser、renderer、GPU、utility 进程句柄，
以及真实 Agent 启动的 Bun pipe、Bun ConPTY 和在途 stdio MCP 进程句柄，共 13 个；退出后这批原句柄全部结束。

| 用例 | 宿主结果 | 原 pipe / ConPTY / MCP PID | 树外 sentinel | 原始回执目录（位于 `%TEMP%`） |
| --- | --- | --- | --- | --- |
| 活动 Agent 正常 Quit | 41888，exit 0，coordinator 清理通过 | 27480 / 45836 / 40880 | 41292 存活 | `nomifun-lifecycle-01a152cc-6c76-7435-8361-bacad5b56870` |
| 活动 Agent parent-death | 仅终止原宿主 10864，exit -1，无 coordinator 证明 | 47560 / 20940 / 28920 | 36192 存活 | `nomifun-lifecycle-01a152cc-cf06-7f4e-a97b-e5657662f37e` |
| Stop → Runtime 注销 → Quit | 7392，exit 0，coordinator 清理通过 | 4540 / 22236 / 20760 | 37880 存活 | `nomifun-lifecycle-01a152cd-2021-76e2-a06a-d2d591afa954` |

三个目录的 `native-exit-proof.json` 均保存各自完整的 10 个 WebView2 原生库存及三个 Agent 进程身份，
并保留 `ready.json`、`binary-identity.json`、`driver-identity.json`、日志及隔离 data/Profile；
正常 Quit 两轮另有正式 `terminal.json`。parent-death 没有正常退出协调器清理，不作为正常 Quit 证明。
三轮均核对原 loopback 关闭、持久数据保留、树外 sentinel 存活和私有 JWT 已删除。
各目录另存 `post-run-audit.json`：只后查原 13 个原生 PID 及本次 host、侧车、sentinel，
每轮这 16 个 PID 均已不存在；该后查补充原句柄证明，不替代它。保留的三个数据库分别为
3805184、3604480、3805184 bytes，shell Profile 和 fixture sentinel 均仍存在。

第三轮使用原 138-byte operation target，用户 Stop 后实际 Runtime 为 `released`，
原两条 ProcessInstance 的六个身份字段保持一致，pipe/ConPTY 的原句柄仍存活；原 MCP 进程已经结束。
原 MCP Effect 仍为 Unknown，`automatic_replay_authorized=false`，模型请求始终为 5、实际原启动始终为 2、
MCP 调用始终为 1。随后 Quit 才回收两条 Session 后台进程，并取得全部 13 个原生句柄的退出证明。
driver SHA256 为 `a738477db0427c40f725389a559cf38aaf37f4d42cb4bc62c9705df569995a70`，
资源侧车 SHA256 为 `f4ce30a7fd57a2e8cad601289dd09280d8cfbf44cba7a08759f87cd6fd538c56`。
本次构建包含 owned Execution 按原 ID 取消时的 CAS 重试、正式 Step／Attempt 与 Cron／Remote 来源结束的 Browser 退休交接，
并包含整合后的 MCP authentication 独立 retained shutdown worker。本入口不替代 OAuth 回调竞争的独立定向验收。
三轮均另以只读 CIM 观察现有
`LanmanServer` 服务：三轮各自的动作前后均为 Running，PID 5436、创建时间 UTC
`2026-10-17T02:50:20.4820180Z` 保持一致，证据为各目录的 `os-service-observation.json`。
仅申请 SYNCHRONIZE 的句柄访问被系统以错误 5 拒绝，未提权或启停服务，服务进程证据限于只读 CIM。
这只证明三轮期间现有 OS 服务保留，Agent 新建、交接独立服务的组合仍待受控环境验收。
进程编号只标识这些独立运行的见证，不能作为后续定位或清理依据。这批证据不替代其他资源、
其他 OS 服务、Computer 权限/UAC/锁屏、完整生产 GUI、签名安装包或其他平台验收。

此前独立构建的 managed fixture `--interaction-only`、`--dialog-close-only`、`--close-all-only` 均通过。
新增真实 trusted 用户编辑跨隐藏、任务完成和后继 Turn 保留；另一无保留责任的 task-only 页面自动退休，
准确 controller 和原隔离浏览器进程退出，用户编辑页继续存活。这是生产 Browser owner 的原生 fixture，
完整 canonical Session／模型链路另由 App 测试验证。

同日 Attached Chrome 的 opt-in
`real_chrome_pending_read_keeps_other_target_live_and_disconnect_joins_its_original_owner` **1/1通过**。
专用 Chrome 的一个 semantic world 以受控 Promise 挂住真实 CDP `CallFunctionOn` 读取；另一个
target 的读取、库存和释放先完成。断连关闭准入并收束原调用，原 Chrome、两页 nonce 和草稿
保留。注入只在本次隔离 world，断连前恢复原 prototype；这不是假协议响应，也不扩大为任意
renderer 主线程卡死或物理输入竞争均已通过。

Windows chooser 生命周期补验使用新 example 构建
`af410d71f65185644531ff5c58763f27477b5ee6525e40f6bf247b1d3b337826`（UTC23:22:28，333483008 bytes）。
`--user-files-only`、`--user-downloads-only`、`--picker-only` 三组均通过，含 OOPIF 原 chooser
session/frame/node、trusted activation、隐藏/导航/框架替换/Agent 接管/关闭和原 picker 退出 ACK。
前两轮发现 HEAD 既有 fixture 将 load 时缓存的 child 坐标用于后续布局，实际 trusted 点击落在空白处；
仅夹具改为点击前以新 nonce 读取当前 frame 几何，仍单次派发输入，不改生产权限/chooser 或延长等待。
证据 `%TEMP%/nomifun-chooser-native-71c2191005c043fa8231d46c54c79890/run3.json`，同目录保留失败诊断。
下载组只证明选择框和取消矩阵，未运行实际保存字节、已授权活传输或人工文件选择；不得扩大为这些组合通过。

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

原生组件、完整生产 GUI、真实模型、外部站点与安装包分别计证。上述 Windows fixture 不证明安装包行为、
外部站点兼容或每个子进程退出。macOS 本轮源码、构建身份、实际回执、首次失败和未运行范围统一保存在
[生命周期方案第 10.5 节](../handoffs/agent-lifecycle-reliability.zh.md#105-macos-接续2026-10-19)，本文只维护当前入口和证据边界。

macOS `macos_browser_compatibility_smoke` 的默认入口保留导航 Stop、queued/claimed popup Pause、
同文档及跨文档 History、失败保留文档和原生 Save chooser 取消／页面关闭矩阵。跨文档 Back/Forward
必须等该命令之后准确 native navigation sequence 的 Finished 回执、当前 exact target 及真实 DOM nonce
共同成立，不能从 URL 元数据加旧 Finished 状态判定新文档完成，也不重发 Back/Forward 来获得通过。

独立的 `--lifecycle-case empty` 与 `--lifecycle-case nonempty` 使用同一固定 compatibility audit scope，
各自保留独立新数据根和最终回执。前者验证非空用户页面保留，原生 CloseAll 与 hide 确认后的空载体自动退休；
后者使用原 Agent Create 的原在途 owner、正式 Agent 自动呈现、真实输入锁确认，以及同一 run 内的 hide ACK／
准确 attachment 释放，验证使命结束后非空 task-only 载体自动退休。默认人工／未知呈现仍保守保留用户现场，
fixture 不能绕过 BrowserWorkspace 直接调用 Runtime surface 或把人工用途改称 Agent 用途。
两种独立 case 还核对原用户文档／语义输入草稿保留、准确 Confirm Stop、旧 request ID 对后继页面无效，以及
pending Confirm 中的原 Browser service／factory shutdown。语义输入不是人工可信键鼠输入；使命完成由组件
fixture 提供，不代替 canonical Session／Execution 的正式结算和完整 Desktop coordinator 验收。

这些入口必须先打包准确构建，再通过现役 controller 的 LaunchServices admission 启动；打包签名不证明运行。
`package-macos-native-fixture.mjs --kind compatibility` 在 `--` 后传递独立 case 参数。退出成功同时要求原
controller 的 authenticated cleanup receipt 与准确 kernel normal-exit 证明；页面隐藏、native view 数量归零或
单个 exit 0 均不替代这两项。关闭时，已经 yield 真实 dialog 的原工作先在 native drain 后取得自己的 completion，
再销毁 view；其他可能依赖 Page close 才能结束的原生工作保留原 close／join 顺序。未知结果仍保持未知，不能以
销毁 view 冒充脚本已结算。共享 WebContent 不按名称或 PID 扫杀。

生产窗口的地址栏、标签、菜单、显隐和恢复、完整持久 store 重启／Session 隔离，以及 Safari17、NativeWebKit
与系统 Safari 的外部站点对照仍各自需要独立证据；适用本轮生命周期专项的已验／未验范围只在上述单份进度记录。
完整生产覆盖还须区分浏览器调用完成后继续思考或使用无关工具、释放后的人工输入及 runtime 关闭、
结算／解锁失败和 exact terminal 恢复。安全清理须保留仍可用的观察，旧代际清理不能影响替代实体。

最低支持系统、Intel、Developer ID 签名与公证发布没有由 Apple Silicon 开发构建自动覆盖。
工具限制需要如实记录；无法启动的用例继续标为未运行，不改变启动、权限或清理合同来取得通过。

## 历史记录

日期化施工报告、交接单、逐次日志和截图从工作树退休，通过 Git 历史追溯。先使用
`git log --all -- docs/reports docs/handoffs` 找到删除前提交，再使用
`git show <commit>:<path>` 读取相应文件。历史记录只证明其中明确写出的基线与范围，
不充当新源码的验收或当前开发要求。
