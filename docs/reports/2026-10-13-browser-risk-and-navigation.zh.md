# 内置浏览器风控与重复导航排查方案

本文记录排查基线、因果证据与当前整改边界。基线缺陷不表示当前源码仍有同一缺陷；下面分别标明已修实现、已取得的验收和仍未证明的现场归因。完整验证命令、artifact 身份与尚未完成的两平台实机证据见 [实施记录](2026-10-13-browser-remediation-evidence.zh.md)。原始假设不作为网站已证实根因，历史验收不被后续修改改写。

用户已进一步确认产品约束：保持内嵌浏览器为主场景；系统浏览器仅保留部分场景功能，不作为主场景重构或自动故障回退。其他不改变该形态的优化继续执行。下面承载方案按此约束收束，系统浏览器对照成功不能替代内嵌原生验收。

**当前 Mac 本地整改边界（2026-10-15）**：真实 macOS 26.6.2 / 25G83 / arm64 从已发布 `6409993b776c95f14ebc4785b2eb4d2759a526f8` 继续，不再要求导入旧 portable patch。本轮已修补 queued/in-flight popup Pause 准入与同文档 History 的源码缺口，定向 unit 的已测范围另记；它们尚未成为百度现场根因或修复效果的实机证明。computer use 明确拒绝 Terminal，本轮没有换终端/shell 绕过；NomiFun 与两个 smoke 尚未启动，WK receipt、请求时间线、生产 GUI、存储重启与身份三臂对照仍未取得。系统 Safari private 窗口百度主页/搜索的独立可视正常记录没有 HTTP 计数，不能调整默认 Safari17 或代替内嵌验收。完整构建、首轮失败与剩余项见[本轮 Mac 记录](2026-10-15-browser-macos-local-remediation.zh.md)。

构建验收基准已 fast-forward 到包含该修复的 `5259ca8e53a0e15f08d725fa15ab3f39cd2cad56`。上游仅 6 个模板/i18n 文件，与本轮本地 task 路径无交集，Rust/browser/fixture 来源不变；最新静态 App、embedded mock gate 与 desktop boundary 均通过，最新来源快照零漂移。用户随后授权本轮 commit+push，接续应包含本轮 macOS 修复。此结果仍不提供原生运行或百度现场归因；旧 App、unit 与 fixture 证据保留各自版本和范围，不倒改历史。

**Windows Bili结论与同步状态（发布前历史，2026-10-15）**

Windows搜索子页持续加载已定位并作最小修补。应用在真实popup激活时过早把opener的WebView2 controller设为不可见；保持准确opener controller活性、HWND继续隐藏，能使首次child文档正常完成。相同原S1 Profile、相同内嵌静态UI的A/B/A中，旧行为77A在22.773s仍0%，C212修补的3次真实鼠标搜索均出现结果，撤回为E429又在25.616s仍0%。旧行为两次均在切回parent后约126–175ms收到正文/结束回调。C212三次准确Document loadingFinished距点击为1019.768/1294.166/1028.589ms；这些为host callback时间，不是线上TTFB。该对照确认本次Windows/WebView2 155的应用边界原因，不泛化未测得的SDK内部机制，也不直接归因macOS百度。

修补仅在NativeTab保存可选exact popup opener；活动child的首次文档pending期间，按同runtime、准确owner、可见面板和surface lease保活隐藏opener，commit/终态/Pause/后继尝试/关闭/lease失效后释放。child原44A遮罩不变，无parent像素/焦点暴露；没有snapshot副作用、新ledger、CDP救援、UA/代理变化或重导航。独立review、7项定向unit及main build均通过；C212源码和二进制已在反向对照后恢复并核验；最后PID28612真实鼠标搜索同样通过，准确Document loadingFinished距click1152.514ms（probe elapsed851ms），169.084s后真实窗口仍见同tab正常结果，期间没有parent切换、reload、Stop或新导航。最终app保持可视运行供用户查看，不记该PID已退出。完整artifact、62/30行安全样本及SHA见[实施记录](2026-10-13-browser-remediation-evidence.zh.md)。

程序启停现由Root使用computer use自行完成；main新增仅供编译指定测试data/work时启用的debug验收配置，X沿既有ExitCoordinator结算。已取得准确PID退出及部分cleanup milestones，不宣告全部资源逐项关闭。77A/C212/E429均用现有Tauri/custom-protocol静态UI，0.8.8/API30/build-id1f7e0b07-b245-4f04-aaec-69c66d8d0fa4，不依赖Vite5173；这是内嵌产品的现有资源承载方式。

UTC03:02 fresh fetch/ls-remote核对HEAD=origin/main=canonical remote为2d5aa62d3f7daf2869fe20be457174742431238a、0/0，无冲突。原250路径232逐字节保全、18重叠整合；额外main debug验收配置使当前为251本地路径，未commit/push，备份/stash保留。同步后Windows125项、Mac portable10、Darwin crate check、Workspace34及UI145等证明保留各自范围；它们不能替代新站点对照或Mac实机。Mac权限、快捷键、公开AppKit/LaunchServices融合合同保留，queued/in-flight popup Pause仍未实施；完整Host/WK、身份A/B、正式store及原始百度现场交接真实Mac，不宣告跨平台Goal完成。

**早期诊断及既有整改的历史范围**

A166为native基于3692、renderer基于2d5的混合基线：Enter popup在commit885ms/finish1549ms完成，相同URL普通tab有结果；鼠标popup在113.98s仍None/0%，同次于commit121462ms/finish122110ms完成200。当时632ms只取得响应头，正文完成未知；Page读取500ms未回不证明renderer hang/Debugger waiting。旧main覆盖运行中PID13864时报OS error5/EXIT101，这是真实历史失败，不是当前build状态。后续41D0补body计数，child-controller-only E94A候选仍重现并已撤回；当前opener修补与上方静态A/B/A独立记录。

确定的UI同批防重、Windows状态/诊断、外站初始化与旧百度验收标准已整改，Worker/UDF持久性有独立原生因果证明。9C6F和95E5的popup Pause/Resume、失败恢复、Session基础切换、880×600输入/显隐及History修补保留原范围；百度主页/搜索95.158/114.721秒、Bili主页136.237秒静置结果均为各自历史采样，不借用为当前版本或Mac通过。早期Bili只有provisional后人工Stop/close的记录不倒改；当前Windows原因已由后续对照推进。不能从局部成功推算风控故障率或承诺零风控。

范围覆盖共享 Browser UI、HTTP 命令、BrowserWorkspace、原生 Windows WebView2 和 macOS WKWebView、普通页与 popup、Profile 与站点数据、Agent 入口及独立 headless 消费者。排查开始时的源码基线为 `3692c8c14a2ff39fc5930c6c7a13a6333e6d9ab7`；本报告保留该基线的调用链与设计依据，当前整改状态以实施记录为准。排查开始时已有的其他暂存修改不属于浏览器排查。

**一　当前调用链与消费者边界**

人工地址栏操作经过 `BrowserPanel.navigate` → `run(create/navigate)` → `browserClient.command` → Browser REST route → `BrowserWorkspace.user_command` → 当前原生 runtime。macOS 在 AppKit 主线程执行一次 `loadRequest` 或 `reload`；Windows 在已建立的 WebView2 child 上执行一次 `Page.navigate` 或 `Page.reload`。[UI 入口](../../ui/src/renderer/pages/conversation/Browser/BrowserPanel.tsx)、[HTTP route](../../crates/backend/nomifun-app/src/router/browser_workspace.rs)、[Workspace](../../crates/backend/nomifun-browser-platform/src/workspace.rs)、[macOS 派发](../../crates/backend/nomifun-browser-macos/src/engine.rs)、[Windows 派发](../../apps/desktop/src/browser_surface/host.rs)。

打开、挂载、重新挂载与面板错误重试执行 ensure、snapshot、attach 和布局更新；URL snapshot 只更新地址栏。标签 activate、resize、show/hide、前台恢复均不调用网页导航。已有 Runtime 在 Mutex 下复用，不因重新挂载而重建。慢加载的 10 秒 timer 只显示提示。

macOS 每 250 毫秒的 `schedule_metadata` → `Delegate.refresh` 只读取 URL、标题、进度与历史状态。函数名中的 refresh 不是 `WKWebView.reload`。进程终止只设置 Crashed、排空弹窗和在途任务，没有自动重载。[采样入口](../../crates/backend/nomifun-browser-macos/src/engine.rs)、[回调](../../crates/backend/nomifun-browser-macos/src/callbacks.rs)。

两个平台新建普通页都先建立空文档，再派发目标地址一次。popup 完成引擎原来的新窗口事务，没有另外再次导航目标 URL。Windows 快捷键同步设置 handled，并过滤按住键的重复事件。

后台 `web.research/fetch` 使用独立 HttpFetcher/SafeHttpClient；本地搜索和 render 使用独立匿名 headless Chromium。这些公网访问消费者不借用可见浏览器网络或 Profile，其公网 DNS/IP pinning、headless 专用代理、临时 Profile 和启动参数不用于可见内置网页。人工侧栏也不自动连接 attached Chrome。不能用这些路径的网络或自动化参数解释人工百度现象。[研究入口](../../crates/backend/nomifun-app/src/router/agent_wave1_host.rs)、[直接 URL 抓取](../../crates/backend/nomifun-knowledge/src/source_url.rs)。

**二　风控场景的原因与证据**

| 因素 | 基线及当前已确认的行为 | 能支持的结论 |
| --- | --- | --- |
| macOS 浏览器身份 | 普通页和 popup 首次请求前统一设置 `Version/17.0 Safari/605.1.15` | 全局兼容身份长期固定，需要真实站点和系统版本对照；尚未证明是百度风控原因 |
| Windows 宿主标记 | 基线外部 Tauri child 收到 Wry IPC 与 Tauri core 初始化；当前普通页、popup 和维护 view 使用 `.external_page()`，从创建源头省略应用初始化与 IPC | 原生 fixture 已证明主文档/subframe/popup 的宿主标记隔离；站点是否使用基线信号尚未验证 |
| Profile 隔离 | 正式人工与 managed Agent 共用 owner 加 canonical Session 的同一持久 store | 新聊天是新站点身份，没有外部 Safari/Chrome 的既有登录与验证凭证；这可能增加首次验证，不是每次刷新换 Profile |
| 存储生命周期 | 关闭、隐藏与普通重开不自动清 Cookie；ClearSiteData 为显式独立入口；Windows native UDF 表示与有效 getter 已修正 | Windows 同 Session 重开、应用重启与不同 Session 隔离已有生产 Host 原生证明；macOS 持久性仍待实机，不能从目录稳定推导登录态已保留 |
| macOS 人工脚本 | 直接创建 WKWebView，无全站 WKUserScript、Tauri IPC 或 Playwright 注入 | 手动百度场景不能归因为 Agent 语义脚本 |
| 可见浏览器网络 | 使用系统 WebKit/WebView2 网络栈，没有应用转发代理或目标站点预检 GET | IP、系统代理、DNS、证书与站点策略需要对照；不能归因于后台 headless firewall |
| 平台策略与环境覆盖 | WebKit 有跨站存储限制；WebView2 支持环境变量及企业策略覆盖 | 属于真实的平台分支，但当前用户现场是否命中未知 |

后续 Windows 当前证据补充：原有 iframe-only `Target.setAutoAttach` 在 renderer 子 Worker 启动前启用了全局等待，而 filter 没有把 Worker 交给宿主恢复。普通内嵌构造能执行相同脚本，生产 Host 中脚本已下载但没有执行；准确 session 的恢复与独立附着修复后，父 Worker、嵌套 Worker、iframe 均取得实际执行 witness。这是已确认的页面资源执行缺陷，不能据此断言百度使用了该信号进行风控。

另一个受控反事实只改变传入 WebView2 的同目录路径表示：Runtime `155.0.4283.45` 中，Windows verbatim UDF 输入未创建 Cookie 数据库，重开后 HTTP/SDK Cookie 缺失；普通绝对路径输入创建数据库并保留 Cookie。两组的目录对象身份、持久模式、HTTP 响应及 Close 流程已核对，localStorage/IndexedDB 两组均保留。生产输入规范化后，同 Session 重开、跨应用重启及不同 Session 隔离已有实际证明；有效环境覆盖仍须防止重新引入不支持的路径表示。登录态丢失可能使网站重新要求验证，但用户的 macOS 百度现场尚未因此得到归因。

macOS 的 `Intel Mac OS X 10_15_7` 和 `AppleWebKit/605.1.15` 不能作为“内核过旧”或“Apple Silicon 伪装冲突”的证据。官方 Safari 26 桌面 UA 也保留这些兼容 token；值得测试的是固定的 `Version/17.0` 以及实际能力与身份策略之间的兼容性。[WebKit Safari 26 说明](https://webkit.org/blog/17333/webkit-features-in-safari-26-0/)。macOS WKWebView 默认 UA 及公开版本识别存在限制，不能直接把 OS 大版本、安装的 Safari App 版本或 605.1.15 当作正在加载的 WK runtime 版本。[WebKit 版本识别讨论](https://bugs.webkit.org/show_bug.cgi?id=278284)、[当前身份策略](../../crates/backend/nomifun-browser-macos/src/identity.rs)。

基线 Windows 网页里的宿主标记来自 Tauri child 的创建路径和 Wry 初始化：Tauri 2.11.2 原路径在 `prepare_pending_webview` 中加入 core 与 metadata，Wry 的 IPC handler 路径注入 `window.ipc`。该平台的 document-created 脚本也会进入 subframe，不能假定 main-frame-only 标记能隔离它们。当前锁定版本的 vendored 创建分支已经增加 `.external_page()`：在应用初始化、plugin hook 和 IPC 安装前分流，拒绝外站 startup script；没有 IPC handler 时 Wry 原生设置禁用 web message。生产普通页、popup 和维护 view 使用该分支，主应用 view 保留原路径。[依赖版本](../../Cargo.lock)、[创建分支](../../vendor/tauri/src/manager/webview.rs)、[Wry IPC 安装](../../vendor/wry/src/webview2/mod.rs)、[生产 Host](../../apps/desktop/src/browser_surface/host.rs)。这不是加载后删除 globals，也没有伪装 WebView2 自身的 `chrome.webview`。基线 JS 标记本来也不等于网页得到后台权限。上游 dialog 初始化与当前 first-party notification 的范围，不能作为本次外站页仍有初始化泄漏的证据。[插件封装](../../apps/desktop/src/native_api_plugins.rs)。

正式 Profile 没有 random-on-navigation 行为。Windows 目录身份由 owner 与 Session 稳定计算；macOS 系统 store UUID 由渠道、规范数据根、可信 storage generation、owner 与 Session 稳定派生。变更 Resource binding 不改变它；新 Session、不同安装渠道或不同数据根会形成新登录 namespace。[Profile 定义](../../crates/backend/nomifun-browser-platform/src/runtime.rs)、[正式人工 Profile](../../crates/backend/nomifun-app/src/router/browser_workspace.rs)。不应通过自动复制 Safari/Chrome Cookie、跨 Session 共用 store 或频繁清空 Profile 来处理风控。

WebKit 的跨站 Cookie/storage 限制可能使某些跨站登录或验证往返失败，但百度的同站子域不应直接当作第三方域。必须先取得真实的验证目标域和访问结果。[WebKit Tracking Prevention](https://webkit.org/tracking-prevention/)、[WKWebView 平台策略](https://webkit.org/blog/10882/app-bound-domains/)。Windows 的环境变量与注册表策略可能覆盖 runtime 路径、启动参数和用户数据目录，代码传入正确目录不等于运行时一定使用该目录。[Microsoft 环境配置合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/webview2-idl)。

嵌入浏览器和自动化环境本身可能只被站点有限支持。Cloudflare 的官方资料列出嵌入环境兼容限制，并说明挑战循环可能涉及网络、配置或检测结果；这是一般网站挑战的参考，不表示百度使用 Cloudflare，也不证明百度本次的判断规则。[支持环境](https://developers.cloudflare.com/cloudflare-challenges/reference/supported-browsers/)、[挑战循环说明](https://developers.cloudflare.com/cloudflare-challenges/troubleshooting/challenge-solve-issues/)。

**三　重复刷新场景的基线缺陷与整改状态**

1. **基线共享 UI 同一事件批次可以重复下发命令；当前已补同步准入。** 基线 `run()` 先按 React `busy` state 准入，再 `setBusy(true)`；`commandSequence` 只忽略旧结果，不撤销第二次发送。隔离探针中，普通独立 Submit pending 时发 1 条，同一个 `act` 的 Submit burst 发 2 条，同批 bridge shortcut burst 发 2 条 Reload；Mount、URL snapshot、resize 和 visibility 发 0 条导航。当前 `commandFlight` 同步占用，Session/client/host 变化与 unmount 使旧 scope 失效，只有准确 token 能释放；Stop/Pause 使用独立 interrupt 通道。相应 UI 测试已通过。基线结果证明同批准入缺口，没有证明正常人工双击必然重复，也不能独自解释百度持续循环。macOS 没安装 Windows shortcut bridge，shortcut 探针不能直接套到 macOS。[run 入口](../../ui/src/renderer/pages/conversation/Browser/BrowserPanel.tsx)。

2. **基线 Windows 把导航完成等同于成功；当前状态已有独立原生 owner。** 基线 Wry PageLoad 将任何 `NavigationCompleted` 变成 Finished，Host 随即写 Ready，未保留 NavigationId、IsSuccess、WebErrorStatus。失败可能没有 ContentLoading，旧完成也可能晚于新开始，形成伪成功或状态覆盖，与[原生事件合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/navigation-events)冲突。当前 Host 已移除该 PageLoad 状态写入，独立 owner 读取原生 NavigationId、成功/失败、下载与进程终止；准确取消终态不被迟到失败覆盖，真实迟到 commit 仍保留文档事实。DF6539 原生矩阵已有相应证明，但尚未证明基线状态错误曾触发某个站点的重复请求。[当前 owner](../../apps/desktop/src/browser_surface/windows_navigation.rs)、[Host](../../apps/desktop/src/browser_surface/host.rs)。

3. **基线 Windows 忽略 CDP 失败字段与控制尝试关联；当前已修。** 基线 `navigation_command` 仅检查协议调用 Err，丢弃成功返回 JSON；`Page.navigate.errorText/isDownload` 不能都当作成功提交。当前解析结果并保留准确 receipt，用户控制携带预期 navigation sequence，原生线程核对准确尝试；旧 Stop/Reload 不改后继，取消还等待原派发 settlement。DF6539 原生竞态已有证明。[Host 命令](../../apps/desktop/src/browser_surface/host.rs)、[原生 owner](../../apps/desktop/src/browser_surface/windows_navigation.rs)、[CDP 合同](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Page.pdl)。

4. **基线 Windows 没实现复制导航诊断；当前提供有界跨导航报告。** 基线继承不支持的默认接口，console/network 异常日志又在新文档清空，不能解释主文档、302、iframe 或 XHR。当前 Host 已实现 `navigation_diagnostics`，准确原生尝试与独立网络样本分别保留，导出安全 URL、统计和证据损失；未知关联保持未知。实际主界面已通过产品菜单复制报告，但长报告工具读取被截断时只归档 prefix/tail，不声称完整事件数组已取得。[接口](../../crates/backend/nomifun-browser-platform/src/runtime.rs)、[Host](../../apps/desktop/src/browser_surface/host.rs)、[原生轨迹](../../apps/desktop/src/browser_surface/windows_navigation.rs)。

5. **基线 macOS 页面导航字段不足；当前源码已补公开 action 信息，实机待验。** 基线 `decide_action` 没采集 navigationType、HTTP method 与来源 frame。当前 policy callback 已采集这些公开字段、main/source frame、来源 origin 与准入决定；`PageNavigation` 仍包含链接、脚本、reload 及 other，action type 不证明物理用户手势。Requested 统计必须同时过滤 source，HTTP redirect 仍属于同一尝试。Response callback 没有 WKNavigation 关联，未关联样本保持未知，不靠 URL 或 latest 绑定。[WK 回调](../../crates/backend/nomifun-browser-macos/src/callbacks.rs)、[Reducer](../../crates/backend/nomifun-browser-macos/src/navigation.rs)、[公开 navigationType](https://developer.apple.com/documentation/webkit/wknavigationtype?language=objc)。当前 pure/cross 检查不代替 macOS 原生实机。

6. **Stop 的原生能力只针对当前加载；当前新增显式 Pause/Resume。** Stop 不能阻止网页下一次 timer 导航，这个 API 边界仍然成立；基线缺少标签暂停入口。当前 macOS 在公开 policy 层保持 hold，Windows 在原生导航与主文档请求阶段建立 hold，Resume 不重放旧请求。Windows 原生及实际主界面已证明循环主文档计数冻结与保留 nonce，macOS 实机待验。Hold 不承诺阻止全部 iframe/XHR 或其他标签的活动，也不证明百度使用 reload timer。[WK policy](../../crates/backend/nomifun-browser-macos/src/callbacks.rs)、[Windows 请求准入](../../apps/desktop/src/browser_surface/navigation_request_gate.rs)。

7. **旧百度 smoke 的通过标准过宽；runner标准已修，Windows后续分项实测与Mac暂缓分别记录。** 基线只访问 `/s?wd=nomifun`，锁人工输入，把验证页计为 entry pass，且只约束 host Requested=1；它不能证明手动主页无刷新。当前 runner 分别测试百度主页、搜索与 Bilibili，解锁静置 60 秒，独立记录正常、验证、版本拒绝、原生失败和持续导航；验证码不计正常通过，手动输入/点击/后退/刷新/显隐与最终文档证据另行要求。分类 unit 已通过；后续Windows真实网站的分项观察见第八节，Mac真实网站仍按用户要求暂缓。旧验收不能倒推为现行主页已修复。[当前 live smoke](../../apps/desktop/examples/support/macos_browser_compatibility_live.rs)、[历史范围](../architecture/browser-platform.zh.md)。

8. **实际GUI的popup/恢复反例已修，原生和9C6F GUI各有准确范围。** AA94中暂停父页仍可新popup，无内容恢复pending露旧错误页；源码补opener准入、pending清理与mask。64CB/34DDD原生证明准确claimed/published候选Close ACK先于Pause ACK、首次prompt可见/可信输入等。9C6F真实GUI随后核对：两种paused popup与Resume旧请求的stable计数均1；新点击后为2/delta1，父页/输入保留。首次连接拒绝有失败提示且无native fixture文档，120000ms恢复pending仍无文档，finish后同槽有/slow及可写输入。GUI可访问性/截图和HTTP计数没有冒充COM getter，没有穷举调度或改写旧AA94失败。[popup](../../apps/desktop/src/browser_surface/popup.rs)、[原生准入](../../apps/desktop/src/browser_surface/windows_navigation.rs)、[presentation](../../apps/desktop/src/browser_surface/windows_presentation.rs)。

共享 HTTP 桥没有 navigation network/timeout 自动重试。只有 WebUI 的 CSRF 拒绝 mutation 会更新 token 后重试一次，首个请求在执行前被拒绝；桌面不进入该分支。不能把这次受限重试说成网页重复加载的通用根因。[HTTP 桥](../../ui/src/common/adapter/httpBridge.ts)。

**四　统一导航事实与防重方案**

共享层复用现有 BrowserLoadSummary、BrowserNavigationTrace 和 Runtime 生命周期。由单一原生 Page owner 写导航状态，renderer 只投影；不建立新的 Agent Session 或持久化浏览器命令账本。

| 事实 | 作用 | 关联要求 |
| --- | --- | --- |
| 用户或 Agent 命令与 dispatch 标识 | 证明宿主有没有重复派发 | 短生命周期 ID，绑定现有 runtime 和准确命令种类 |
| Native navigation ID 与尝试序号 | 区分一次导航的 redirect 和多次新导航 | Windows NavigationId；macOS WKNavigation 精确身份 |
| 文档 generation 与 commit | 防止旧 Stop、旧完成和旧观察作用于后继页面 | 尝试和已提交文档分别表达，不靠 URL/latest 推断 |
| Main frame、公开 action type、method、initiator origin | 区分主导航、链接、form、reload、history 和笼统 other | 不把 native navigationType 当作可靠的物理用户手势证明 |
| Response、失败、取消、下载、进程终止 | 提供准确状态与恢复依据 | 无法证明关联时保持 Unknown，不写进当前 load |
| Dropped 与 routing_lost | 表达采集覆盖和关联证据损失 | 已知旧回调与无法解释的 token 分开处理 |

保留最近 32 个 attempt、128 个事件、128 KiB 的既有有界轨迹；计数使用独立的有界统计字段，不能从可能已淘汰的 events 反推总数。轨迹跨同一 Page 的导航保留，关闭 Page 时释放。网页正文、Cookie、认证头、表单与请求正文不进入诊断；URL 沿用删除 userinfo/query/fragment 的安全投影。需要比较原始 URL 是否相等时，仅在内存保留随机盐派生的 equality key，避免把不同搜索 query 的安全投影误算为相同 URL 循环，不导出原始 query 或长期关联标识。

Windows 在首次外部请求前安装独立 native navigation owner，采集 NavigationStarting、ContentLoading、SourceChanged、NavigationCompleted 与 ProcessFailed。新的尝试立即进入 requested/provisional，真实提交后再更新内容事实。读取 IsRedirect、IsUserInitiated、IsSuccess、WebErrorStatus 及可用的 HTTP status。普通页与 popup 共用这条实现。删除旧 Wry PageLoad 回调对 Browser 状态的写入权，避免双 owner。协议提交结果保留 errorText/isDownload 与准确 receipt；提交确认不等于加载或业务成功。Stop 在原生 UI 线程核对当前 attempt，Reload 使用准确 native/loader 关联保护。

macOS 保留现有 WKNavigation 和 receipt 安全路径，在公开 action callback 补足 action type、method、main/source frame 和 policy decision。不能通过 patch `location.reload`、插入 page-world getter 或改写网页 fetch 来采集。WK 公共 callback 没有完整 Fetch/XHR/子资源网络信息时，明确报告该范围不可用，不伪造完整 HAR，也不为诊断引入应用代理或另一次请求。

UI 当前使用同步 in-flight ref：首次命令进入时立即占用，后续同批次调用拒绝；busy state 负责可见反馈。只有持有该 token 的 finally 能释放；Session/client/host 变更和 unmount 使旧 token 失效，旧 Promise 不得释放新会话的 gate。Stop/Pause 保留明确的独立 interrupt 通道，不能扩大成所有修改绕过互斥。Link consumption 仍以稳定的 request 对象去重，没有已证实的自动重放；不能把基线曾提出的 `(Session, linkRequest.id)` 方案写成已实施的第二套命令记录。

当前不增加 HTTP 失败自动重试。若后续确实增加传输恢复，再给同一次命令配 request ID 和 runtime 内有界结果缓存：同 ID 同 payload 复用执行结果，同 ID 不同 payload 冲突，每次仍校验现有 owner、run lock 与 runtime fence。显式第二次刷新使用新 ID。进程重启或结果无法证明时保持不确定性，不能重放网页 POST、点击或 Agent effect。这层不作为此次排查的默认持久化工程。

**五　网站循环的检测与恢复方案**

当前检测使用准确关联的独立 main document attempt：10 秒内至少 5 次同 URL 新导航或 A→B→A 往返产生提示，URL 比较只保留进程内带盐 equality key。宿主命令和原生明确 `user_initiated=true` 的操作重置观察窗口，redirect、未关联 action 与资源样本不累计刷新次数。来源为 Unknown 但有准确主文档事实时可以参与提示，仍保持 Unknown，不能改称页面脚本。一次 302 链、图片、iframe、Fetch/XHR 轮询均不按域名请求总数判循环。阈值是当前 advisory 参数，需要真实登录/SSO 样本继续校准，不是认定网站风控的证据。[检测实现](../../crates/backend/nomifun-browser-platform/src/navigation.rs)。

达到可疑条件后显示“页面正在反复跳转”，给出宿主命令次数、页面导航次数和继续/暂停入口。默认先提示，避免粗暴拦截合法 OAuth/SSO、验证码回跳及明确的人工导航。用户选择暂停后，在原生 policy 的标签级 hold 阻止后续未经明确继续的主文档导航，并停止当前加载、保留可用内容。网页 timer 的下一次导航也必须被挡住。Hold 只控制导航，不能声称已经停止所有网页 Fetch/XHR；需要彻底停止该页网络活动时保留关闭标签入口。

Windows 当前请求阶段控制只在可证明 invariant TSC/QPC 时钟对应关系的 x86/x64 上提供；其他架构或无法建立该证明时省略 `navigation_control` 并拒绝 Pause，保留普通导航与 Stop，不把未建立的网络 hold 表达为成功。该能力边界和popup准入各批次的准确证明范围，均需在平台交接中保留。

暂停是独立状态，不伪装成 Failed 或 Crashed。用户在宿主栏明确继续、输入新地址、刷新或历史导航后解除 hold。暂停期间网页内 linkActivated/formSubmitted 不能自动被视作可信解锁手势，因为公开 action type 不是严格的人类手势证明。Agent 运行时仍遵循既有输入锁；Agent 遇到验证或连续不确定结果后应停止同症状重试，通过原有 Stop 和 settlement 流程恢复人工处理，不另加交还授权系统。

验证码、403、429、版本建议和循环分别报告，HTTP 200 也可能承载验证页，不能只看状态码；403 也不能一律称作验证码。保留网页供用户处理，不把全 Workspace 重建、清 Cookie、自动换 UA 或反复刷新作为恢复动作。已有“在系统浏览器打开此页”作为显式出口继续可用；OS 接受打开请求不等于页面成功，且不会共享内置 Cookie 或创建 attached Chrome 授权。

**六　浏览器身份与承载方案**

| 方案 | 改动与作用 | 限制 | 采用条件 |
| --- | --- | --- | --- |
| 保留原生 WebView，修导航与可观测环境 | 复用当前系统内核、Profile、输入与下载实现；跨平台修确定缺陷 | 部分站点仍可能不支持嵌入环境 | 当前首选 |
| 系统浏览器显式打开，Agent 使用已有授权 attached Chrome | 保留已存在、由用户明确选择的部分场景能力 | 页面和登录态独立，Agent 需已有正式授权 | 不作为内嵌主场景整改、重构目标或自动回退；仅作既有功能及验收对照 |

macOS 身份先做现有 Safari17 baseline 与原生 WK 默认 UA 的对照；如两个方案都存在站点版本识别问题，再验证候选兼容 baseline。身份在创建时选择，贯穿普通页、popup、iframe、fetch/image 的整个 runtime，不能在一个验证过程里切换。候选策略须标明 revision、实际测试的 OS 与站点范围，未知 runtime 版本继续保持 unknown。不能盲目把 Version/17.0 改成 OS major 或 Chrome，不能做随机 UA、webdriver getter、Canvas/WebGL 指纹 patch。现行兼容身份已有版本建议场景依据，未经对照不直接撤销。

Windows 优先保持原生 UA 与 Client Hints 一致，检查实际 Runtime 的版本和 channel、有效 Profile 位置、是否存在环境/企业策略覆盖。对外只输出安全版本信息、覆盖是否存在与不可逆 Profile token，不输出原始参数、用户路径或 Cookie。微软说明 custom UA 可能改变 UA Client Hints，因此只改一条 header 或 UA 字符串不能当作通用修复。[Microsoft UserAgent 合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings2)。

Windows 已选择范围限定的 vendored `.external_page()` 创建选项，从 Tauri core/plugin 与 Wry IPC 安装源头隔离外部 Browser view。保留同一 Tauri 窗口、主线程、BrowserWorkspace 和原生 COM 端口，没有第二个 App event loop。上游 Tauri 2.11.2 原 builder 没有此选项，因此本次是明确的局部依赖分支；不能只删 capability 或 ordinary 初始化参数达到同一效果。DF6539 已验证主文档、subframe、Worker、popup 环境，独立 dialog/文件选择/Agent 上传下载链取得各自范围证明；64CB popup/presentation 及 34DDD 初始原生 prompt 可见/可信、准确 claimed/published 候选的在途暂停也实际通过。9C6F 后续正式 GUI 与真实站点已取得下文限定范围证据；后续95E5 main已包含History及本轮review修复，对应History实际GUI已通过，880×600输入、显隐与宽屏恢复也已在本轮实际通过。没有加载后删除 getter 或伪装 `chrome.webview`。消除宿主初始化降低环境差异，仍不能承诺特定站点不风控。

保留 owner 与 Session 的 Profile 隔离。是否让多个聊天共享网站登录属于独立产品决策，不为这次风控修复自动改变。网站失败时保留 store，重启/重开持久性通过无敏感值的 conformance 证明。

**七　整改分批依据与完整通过标准**

第一批覆盖确定缺陷：共享同步防重、Windows native navigation owner 和协议结果、两个平台统一诊断、旧 smoke outcome 修正。这些源码已实施，只补必要的 transient navigation 数据，没有修改 canonical Agent Session storage 或引入命令历史表。

第二批提供循环提示/Pause/Resume、恢复和环境conformance；Windows相关原生与正式GUI的popup/恢复已有证明，880×600布局失败经修复后也取得实机回归。两Session仅证明基础切换/输入，强busy-gate交错另保留；真实网站按主页/搜索各自观察窗口记录，不从fixture推导网站风控规则。Mac身份与现场仍按用户要求交接。

第三批仅在站点证据支持时调整兼容身份，继续保持原生内嵌产品形态。不存在所有站点“零风控”的可验收承诺；验收目标是没有应用诱发的重复、状态真实、证据足够、用户能停止循环和继续工作。

| 验收组 | 场景 | 必须取得的证据 |
| --- | --- | --- |
| UI | 普通与同批 Submit、同批 bridge shortcut、慢请求、Session 切换、旧 Promise 回来 | 同次操作只派发一次，旧完成不能解锁新操作，Stop 可用 |
| 被动 UI | Mount、snapshot URL、resize、show/hide、前台恢复、复制诊断 | fixture 主文档请求数不增加 |
| 正常导航 | Create、Navigate、显式 Reload、popup | 每个明确操作只有预期的一次主请求；popup 不克隆发请求 |
| Redirect | 两跳 302、跨域往返 | host count 为 1，redirect 与新的 attempt 正确区分 |
| 页面循环 | JS reload、replace、meta refresh、A→B→A timer | 显示正确来源；Pause 挡住未来主导航；Resume 可恢复 |
| 普通资源 | 大量图片、iframe、XHR、WebSocket | 不误判成主文档刷新；能力缺失明确报告 |
| 错误状态 | DNS、TLS、拒绝连接、Stop、beforeunload、下载、进程退出 | 不显示伪 Ready，旧或部分内容与尝试地址分开表达 |
| 导航竞态 | A/B 重叠、didStart 前替代、迟到完成、旧 Stop/Reload、未知 token | 不影响后继，无法证明关联时保持 Unknown |
| 诊断 | 超容量、回调丢失、跨导航与关闭 | dropped 与 routing_lost 准确，安全 URL 无 query/认证数据 |
| 环境 | 正常与 override WebView2、本地 page/iframe/worker 身份和 API | 实际 Profile 与配置相符；无外部页宿主脚本；UA 一致 |
| 存储 | 第一方 Cookie/localStorage/IndexedDB、同站子域与跨站、关闭与重启 | 同 Session 持久，不同 Session 隔离；仅报告成功/失败 |
| 真实站点 | 百度主页与搜索分别手动操作，其他代表性站点，两平台独立 | 正常页、验证页、版本拒绝、持续跳转分别记录 |

真实站点对照在同机、同网络、相近时间做，先用临时独立 Profile 进行一致预热，随后对受测现有持久 store 单独验证，不复制用户 Cookie。macOS 对比 Safari、现行 WK 身份、原生 WK 默认身份；Windows 对比 Edge、普通 WebView2 fixture 与 NomiFun。外部浏览器正常而内嵌异常，可缩小到内嵌差异；普通 WK/WebView2 也异常而完整浏览器正常，可缩小到 WebView/站点兼容；所有环境同样异常，再验证网络或网站规则。分别固定 UA、Profile warm state 和网络，避免一次更改多个因素。

真实百度验收至少持续观察 60 秒静置，再包含地址栏输入、主页到搜索、结果链接、后退/刷新以及显隐切换。计数使用真实主文档 witness 与 native attempt，不以 domain 总请求数替代。最终页面截图和安全导航时间线对应同一 runtime、tab 与当前文档。验证码可以作为 observed outcome，但不能被计为正常访问通过；不能为通过而自动解验证码或反复刷新。

**八　同步前基线与95E5历史验证范围**

排查基线曾运行 `BrowserPanel.test.tsx`：99 项通过、581 次断言；client、keyboardShortcuts、localBrowserLink 三组另有 6 项通过。四个临时探针确认同批缺口和被动更新不导航，随后已删除。该统计属于整改前证据，不能作为当前已修合同的通过数。

同步前记录的UI定向验证为BrowserPanel与client contract合计143项、862 次断言（其中 BrowserPanel 141 项），该95E5 review批次Windows生产模块unit为121 passed/0 failed/123 filtered（较早popup批次112项）；macOS 实际 reducer unit 为 9 项，portable 站点 outcome 分类为 7 项。Popup/presentation example 的 SHA256 为 `64CB7E6F0B426A16FC7C6A7BA32AD8B55A21A6E2010FAD244B95FC567DD9397C`；授权恢复后 `resumed-popup-mask-navigation`、`resumed-managed-popup`、`resumed-agent-download` 均实际 EXIT0。各自 254 个来源路径的前后快照已独立比较，差异均为 0，result/exit 与实际 hash 现场核对。汇总 `resumed-popup-mask-native-summary.json` SHA256 为 `0C81D89D5B5AB15B9888FA843E9DB3A37D86B66BE4F9B08235ECDCA95052D396`；具体证明与剩余范围见[实施记录](2026-10-13-browser-remediation-evidence.zh.md)。浏览器边界和桌面 UI 边界检查通过，各范围不能互相替代。

后续补证 artifact SHA256 为 `34DDD55E8B3D95B7524D6708F24CAB3094C4AB82516A06C876FD484EF6D2CE9D`，claimed popup 暂停与 initial dialog 两个 native 批次、production unit 再次均 EXIT0；各自 255 个来源路径前后差异 0，实际 result/exit/hash 已独立核对。汇总 `claimed-popup-initial-dialog-native-summary.json` SHA256 为 `694D2D404F366CF864EC68043F9ED3CD672B940837B0973D8F8A8A60F38820E9`。它补出准确在途 stage 和真实首次 prompt 回复后可见/输入证明，不能替代新 main 的真实 GUI。

新 main 实际构建 EXIT0，磁盘 SHA256 为 `9C6F261A219E0BA81910F425AC25168DC2D42F33EF361862AF278BF0546F1431`；前后 255 路径零差异。`final-native-and-gui-build-summary.json` SHA256 为 `EBEE4923E4A61A32C0B3C85777B4660EC47814BD0352F3AA224B2F4D49043244`。第二 Session `01a13543-dbf4-7150-9d1e-fc42706eb8cc` 已正式 API 创建并正常清理，第一 Session 不变，Browser binding/grant 与 Turns 均为 0，保留唯一合法非 Browser capability `agent.tool-discovery`；这只是准备，未证明 GUI 切换。

此前工具启动被自动审批在执行前拒绝，返回 `rejected: blocked by policy`、更细原因 unavailable；原拒绝记录保留，没有换接口绕过。用户亲自在新 PowerShell 设置 owned DATA/WORK 并用分号分隔命令后启动成功；此前 AmpersandNotAllowed 是粘贴缺分号，不是产品startup/DLL证据。fresh exact executable/SHA9C6F/PID34236、owned port59127与实际health200/ok/0.8.6已核对；启动manifest SHA256为 `05B156643104D503EDF2AB2733F5804CDA4D7DAD84B32794D62707C5BE929077`。真实唯一NomiFun窗口25629038/外框1282×834与第一Session无Browsergrant入口基线随后取得，打开owned51378/popup时mainDoc请求1；基线SHA256为 `5267F6AE016570BF6010CF9DA0BE0FDEBF0C0133C93C5ED92A6E1BC6EFA0E603`。这解除启动阻碍，不代表其余功能或三个真实站点通过。实际日志仍DATA/logs，LOG输入/CLI/Sky隔离边界未改，详细证据见[GUI交接](../handoffs/2026-10-13-browser-windows-gui.zh.md)。

随后只修正中英暂停提示为“暂停会保留可用页面内容” / “Pausing retains usable page content”，避免首次加载 None 也宣称可见文档；未改 renderer/Host 逻辑。9C6F 的 255 路径构建快照不包含 locale，Vite dev renderer 使用当前字串，字串修改不能被记作新 GUI 通过。当前桌面 UI 边界为 2094 sources/min880×600 PASS，browser boundary 也已实际通过。

后续正式GUI证据已核：`gui-evidence/9C6F/`的两Pause count与Resume旧count均stable:document=1，新popup为2（delta1）；首次Failed/None、pending加载与恢复后native文档分别有真实窗口记录，S1输入另有trusted HTTP witness。S1 60000ms场景在14:45:43.642Z捕获，S2在14:46:53.487Z捕获，相隔约70秒；基础切换可用，但不能称强oldfinally/newBusyGate GUI竞态已证明。

最低布局历史失败保存在 `root-closeout-9C6F/minimum-outer-882x602-after-layout.json`（SHA `E15012024D359EBB82A90DBC833E9C778D788516A79BFDCB15A95AC5824CA263`）。修复后的源/客户区880×600（外框882×602）实机有Browser/地址栏/菜单与S2 `GUI9C_MIN_NATIVE` trusted witness；hide/detach后globe重开、再宽屏恢复仍保留文档/输入，stable计数始终3/delta0。final-layout SHA `5D9A4758394471301C4931C62FC191D3D34E30788729DB875FC8FF187B452EF8`，after-input/final计数 SHA `C2F480238816468EECFEABEDDA9F0A7738EE062FAFE19B4134CF06BD8FD8B6F8`。零Turn场景没有active Agent Stop实际点击；额外916宽度拖拽执行前被Sky拒绝，不计916通过。完整9C GUI汇总已独立核对九个冻结production renderer文件source_drift=[]，该实机范围对应9C Host与当时Vite；后续E584编译不倒改此证据。

Windows百度主页95.158秒静置正常，summary seq4/docgen2 Finished/200/CurrentDocument、loopfalse/repeated1；完整安全trace只有一次主页host Requested和一次native Requested，后续commit/response/finish。真实输入“浏览器”和点击搜索后，普通搜索结果自身静置114.721秒也正常，使用同文档SPA，native content_url=/s但仍seq4/docgen2、无新主文档事件。trace仍degraded/loss=true/drop0，user2/agent0/page0/unknown2，不能把unknown改成页面脚本或把主文档无循环说成全HTTP零请求/完整HAR。主页窗口/诊断SHA分别 `FD70634A3BA292F9BFB1DC4BF8682E1807DF08545E71EDA3F99F534454BC425C` / `6EC3916858780210441ACE7A5F9EBADBC27E6C724A0FAE4AB43FEC05C11EAE5F`；搜索窗口/诊断SHA分别 `0FC49D996512FB04993BB0824E1E49A1D45BBD5601F5876DFA8B0280998369BD` / `5DDA6931163D4A91E97FDC0AB8127196AA86EBB68741FA50E7BBD39E535B543F`。原200属于主页尝试，不代表搜索全部HTTP；原生Chromium155/Edge155未改，dirty draft不作为URL同步缺陷。

百度首次Back实际页面退到前一/s历史项，summary却seq5 Requested/Retained/0%，新增仅dispatch3/back_forward，没有native Starting/Completed；完整异常诊断SHA `BD6B2005052D3FD4429996114B1A77564EF44E69BE1AD6B1719F296838F49670`。这是生产同文档历史状态结算缺口：owner复核synthetic Requested后ACK与SourceChanged(false)仅更新URL，这是9C6F版本的历史失败；D74源码修复及下文60C/72E定向native现已通过，当前95E5对应History真实main GUI已通过，880×600输入、显隐与宽屏恢复也已在本轮实际通过。旧SPA conformance只验URL/history不证明loadphase。第二次full Back回主页、Reload及显隐正常不抹去该FAIL，9C6F不计完整百度交互PASS。

Bili公共主页136.237秒静置正常，仅login promotion不当challenge。真实搜索点击打开新tab，子页seq1 Provisional/None/0%、Requested+Started/page_navigation/user_initiated=true，尚未观察content/commit/completion；supported/lossfalse/drop0/user0/page1。Native Response是在NavigationCompleted才采样，字段缺失不证明HTTP未返回。原因未知，不与无NativeStart的SPA Back混因，也不归为pagefilter/worker等待或网站验证码/风控；人工Stop后Cancelled/None、close子页回parent输入保留，无自动reload。主页窗口/搜索pending诊断SHA分别 `3EE34388976027D5073D0B346BA6F9F102DB19EF433D5503D1C2BE962DEF0134` / `C1E753DEB7B649A7F98274765C0352CC8D2E7F79DF65D10C08C49419216C7084`。Parent跨文档Back/Forward、Reload、hide/show均有finished/正常观察，搜索业务结果仍不PASS。

最新GUI汇总 `final-9C-main-gui-acceptance-summary.json` SHA `F7DC2177849386B076A6977459ABF9494A6140079ED19191C84544AD4EA71441` 记录于2026-10-13T16:15:19.756Z（本地10/14），九个冻结renderer文件source_drift=[]，使用9C debugexe+localhost5173，不代表bundle/installer。最终parent诊断完整49events已通过UIA selected_text17690chars取得并parse，原tree Value7172chars只前缀；SHA `6B0B683BD4CB10778324D1FE36DFD453B26B78AC8BFE473A89E031EC27E66CC0`，最终seq17/docgen8 Finished/200/CurrentDocument。User9含一次same-doc Back及八个显式full动作，native Requested8、agent/page0、unknown8/redirect0/drop0，显隐后无额外Requested、无重复Host派发；sourceunknown/loss仍保留，不说具名绑定或全HTTP零请求。文件名保持10/13，后续60C/72E History原生与E584构建单独记录，不改写本批真实GUI结果。

同文档History的后续修复绑定冻结生产 `windows_navigation.rs` SHA `D74E74C22BCC3067371ECEDA13AB16B206D15DF72D2D469ABE111124FFEF354A`。Back/Forward只推进当前控制fence并记录宿主dispatch，保留真实文档状态；公开 `load.navigation_sequence` 不等同于原生主文档attempt序号。只有真实NativeStarting才创建Loading，空 `Page.navigateToHistoryEntry` 回复或 `SourceChanged(false)` 不补造Finished，不用JS/URL猜测、第二份ledger或新的关联替代原NavigationId/attempt事实。

`same-document-history-native-summary.json` SHA `AA62D42A8C64A3B01F1FF5A64FDA9BD1CA08CDA8B616018BA7C764D289AEBEA8` 已独立核对：production Windows120/0/123通过；60C的Back/Back/Forward/Forward四次同文档均Finished/Ready/100%，原docgen/nonce保持、主文档HTTPdelta0、fence推进；run-lock、stale/current Reload及跨文档真实native生命周期均通过。60C dialog-probe只是generic Navigate→Stay；独立72E才证明History Back命令已回复后的真实BeforeUnload false deferral/Stay，文档Finished/Ready和nonce保持、目标HTTPdelta0、无NativeStarting或伪造Finished，cancel audit为空，不造已结算命令的取消关联。三个native批次各256来源路径0差异、EXIT0，准确artifact/raw result见实施记录，不从此推出网站全部HTTP或全部交错通过。

旧9C6F由用户正常Quit，原34236/59127已关闭；E584 main实际build EXIT0/19.73s、来源2577路径0差异（含2320 renderer），exe SHA `E58484E937E40E6444A4F02FF236426140F94F438721F639BEFEEB2A28F63E4A`，汇总 `same-document-history-final-main-summary.json` SHA `1ABD55F9919341CF169553D3F9E2D1F7CA2A0F0B1A16D6B5FAF0B9625E4DCCF3`。此为E584历史构建回执；E584后续实际启动并正常teardown，95E5实际GUI收口范围见下文；新编译不改写9C6F历史Back失败，也不倒改9C6F+其冻结9 renderer文件的最低窗口PASS。Bili搜索未知原因仍单独记录，不增加popup过滤/等待假设或新的必验矩阵。

较早 DF6539 artifact 的 Windows 完整导航、生产 Profile 五阶段、Agent 原生链和 canonical 完成、site-data 维护均取得实际原生通过；64CB 增补后的完整导航、managed popup 与 Agent 链再次通过，其中 Agent 为精确同 Session/Operation 的 completed seq 142、11 次本地脚本模型调用。DNS 本机输出仍不是 DNS 失败。AA94 正式主界面已完成被动更新零额外主文档请求、显式刷新各一次、Stop、同槽恢复与循环 Pause/Resume 的部分验收，同时取得上述 popup 和 presentation 反例；AA94 的磁盘 hash 仍为原值，64CB 与其112 unit不会改写这些 GUI 反例或表示新修补已进入 AA94。

本轮review已修复两项P2：Browser独占布局预留/dismiss，只隐藏其surface；Preview保持原有侧栏策略，未保存内容不会被布局回调的closePreview清空，切换时交接管理。UA读取改为可选identity诊断，失败仅固定reason和None；UDF/Profile实际目录、持久性与InPrivate仍必须验证，不以UA降级放宽存储身份。用户已认可保留tauri/tauri-runtime-wry两份vendor，保持external_page创建前隔离，不改变内嵌产品形态。

已核对review回执：Windows模块121 passed/0 failed/123 filtered、EXIT0、256来源路径0差异，stdout SHA `2CFEBF140EC26EBDEE5CE6E8C0B74993EB151A9839F5CE5A6D73C99D51E743A6`；95E5 main实际build EXIT0/43.48s、2577来源路径0差异（含2320 renderer），exe SHA `95E5B839FA12F0D03316215EBDFE649D4DAEB46D80E1E30CD8635E2C42524EC7`，artifact回执SHA `83AC5DD085F90BE5BDDDE757C29DEDA430E4D78DEE260852B4EEAAFDCAF14DE4`。E584后续已启动并正常teardown，旧“等待人工启动”已过时；95E5本轮真实GUI现有下述最终manifest，不由build或组件fixture推导通过。

focused汇总 `review-closeout-validation.json` SHA `AF3820116AC0642D0DE37077AFF9511AF7C55E2C7D0C9914E58234F074BC6052` 记录Root实际typecheck/browser boundary通过及UI owner提供的Space6/56、Resize6/100、desktop2095 sources/880×600通过；汇总不是UI raw log。95E5 PID38196/backend59312与frontend均200、该时点GUI in_progress，进程接续本身不算功能GUI完成；后续最终manifest另记实际结果。

95E5实际主GUI已复验对应History反例：canonical第一Session真实百度搜索后，首次Back从tb_enter退tb_pre、Forward恢复，两次fresh记录均无持续loading、可手动操作。Back完整3842chars诊断已经产品Copy及未提交地址草稿读取并parse，SHA `007AC4C32571526E520A47AE58D85DC8F4EEE2ADCA93D3DECD279D3F94BCEED7`；load控制3/Finished/CurrentDocument/100%，trace7events仅原native attempt2/docgen1及Back host dispatch2。user2/agent0/page0/unknown_native1/redirect0/drop0/routing_lost=true；原200保持原主页尝试的意义，不是Back新HTTP响应。Back/Forward窗口证据及来源见实施记录，最低窗口和原生输入的本轮实际证明见下文最终汇总。

95E5本轮最终GUI汇总 `review-closeout-95E5/final-95E5-gui-closeout-summary.json` SHA `31262E2825F2313B6B699E1234F6B2B702E0418ADBE87E87D094E82D382E4BB5` 已读取，52个附件hash核对一致。百度主页86.241s、原生输入/Return搜索88.385s静置正常；同文档Back/Forward反例已闭合。准确客户区880×600/外框882×602下Browser、地址栏、聊天输入可见，原生 `GUI95E5_MIN_NATIVE` 截图与HTTP trusted=true witness均取得；收起重开、1920×1032宽屏恢复后nonce `e0a6d0f4-8f6e-45c2-82da-8771046aedba`及输入保持，184/300侧栏偏好恢复，stable主文档累计1、delta0。计数SHA `84F288BCF80918D8E418615CA8C16A629881408B360C26777F5B73A3BF800B63`，GUI后2577来源漂移0的回执SHA `ED605A2EC43246AD05C8968CC6245DF5E21F1E4CF2DA9A779F7968E5C159515B`；具体快照见实施记录。

旧E584 localhost拒绝连接的恢复回执 `service-recovery-20261014.json` SHA `9F588FB2A0016130F1779A980F17C4451B49146472068958DBA4A066828D5224` 记录native14512/backend60728仍存活、Vite5173缺listener，恢复后frontend/backend200且未改源码；旧Vite退出原因unknown。后续UTC01:01:10正常teardown日志保留，不能将该开发服务现象称为已证实引擎崩溃。95E5最后main38196/backend59312与frontend200、Browser保留运行；实际范围为debug/Vite，非installer。

该95E5批次Windows/shared两项P2及定向GUI已收口。9C6F历史FAIL及原始证明不倒改；当时Bili搜索仍pending、原因未知，后续当前Windows原因与修补见文首静态UI A/B/A。强Session竞态、零Turn中的active Agent Stop及未执行916只标该批未覆盖，不扩成新增发布gate或无限矩阵。Mac按用户要求交接，原用户发生时trace仍缺，不估算风控/故障率、不宣告跨平台Goal完成。
