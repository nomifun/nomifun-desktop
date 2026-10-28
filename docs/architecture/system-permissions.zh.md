# 系统许可与执行身份

NomiFun 默认信任用户的本机任务，在操作系统允许授予的范围内充分开放能力。工作区是默认目录、
项目归属和项目观察范围，不是本机文件、进程 cwd 或系统操作的权限上限。用户不切换权限模式，
应用不另建命令白名单、授权数据库或 Agent 后端。用户明确的只读、不要执行和停止要求继续有效。

本文记录实际调用者和当前实现边界。Session、Turn、Event、Payload、Effect、原生 checkpoint
和取消仍沿[同一事实链](agent-session.zh.md)。共享命令与依赖入口见[外部工具](external-tools.zh.md)。

## 实际调用者

| 操作 | 正常身份与进程 | 系统事实 |
| --- | --- | --- |
| 文件、卷、普通程序、Shell、用户安装 | 登录用户的 App/backend 与受管子进程 | 属主、ACL、文件系统和 TCC 仍有效；绝对路径和工作区外路径正常使用 |
| 管理员文件、系统配置、安装器、服务 | 进程工具的原生系统执行路径；macOS 使用 ServiceManagement 注册的 XPC helper | 系统注册或认证成功后仍须核验实际结果；root 不代表 TCC、桌面或设备已就绪 |
| Computer 截图、可访问性、聚焦、鼠标键盘 | App 直接启动的普通用户 `nomifun-computer-worker` 与 vendored Cua Driver | 许可属于实际 App 代码身份与登录桌面；后台系统服务不冒充交互桌面 |
| macOS 内嵌网页与媒体 | 原桌面 App 内的 WKWebView 与系统 WebKit 内容进程 | 保持 macOS WKWebView、Windows WebView2 结构；网站决定与系统许可分别存在 |
| 语音输入 | 桌面 renderer 的 `getUserMedia` | 宿主麦克风 TCC 状态由同一 App 内 AVFoundation 查询；真实采集仍可能因设备或环境失败 |
| AppleScript / Apple Events | 普通进程工具启动的程序及其实际 responsible App | 正式 App 声明用途及 automation entitlement；授权按发送方、目标和操作发生 |
| 网络、监听、本地连接与已支持设备 | 各现役 Browser、MCP、SSH、Relay、Robot 路径 | 本地网络许可、设备协议授权、网络可达性和系统管理员身份相互独立 |

GUI 和 backend 不长期以 root 运行。明确要保留的服务、登录项和计划任务使用系统原生设施；
临时子进程仍须证明取消、超时、宿主退出和后代清理。系统服务启动成功不证明用户桌面可操作。

## macOS 许可库存

`crates/backend/nomifun-app/src/router/computer_permissions.rs` 提供现役库存：

- `GET /api/system/permissions` 读取实时状态，不申请权限、不下载依赖。
- `POST /api/system/permissions/request` 只申请明确的 microphone、camera、accessibility 或 screen_recording。
- `POST /api/system/permissions/open-settings` 打开对应隐私面板，包含 Full Disk Access 与 Automation。
- 读取要求安装 owner；会显示宿主 UI 的入口还要求本地桌面信任。Computer 只保留新的只读 `/api/computer/status`，旧权限路径已删除。

状态同时返回实际探针进程的 `process_id` 与 `app_bundle_identifier`；裸 CLI 没有 bundle 时为 null，
不把配置里的正式 identifier 冒充当前身份。验收仍须独立核对真实可执行文件、签名与 LaunchServices
启动，bundle 名称不能单独证明 TCC 的 responsible process 归属。

| 项目 | 实际探针 / 申请 | 库存含义与恢复 |
| --- | --- | --- |
| Accessibility | `AXIsProcessTrusted` / `AXIsProcessTrustedWithOptions` | true 为 granted；false 为 not_granted，无法推断首次或曾拒绝。返回 App 后复检，再获取新观察 |
| Screen Recording | `CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess` | 同上；实际截图仍须成功。先复检，系统要求或状态仍陈旧时重启，再继续任务 |
| 麦克风、摄像头 | AVFoundation authorizationStatus / requestAccess | 保留 granted、denied、restricted、not_determined、unknown；系统授权不证明设备存在、可用或已采集 |
| Full Disk Access / 受保护目录与卷 | 没有伪造全局探针；读取实际任务的文件结果 | 全局状态 unknown；设置入口可用。不读取 TCC 数据库或无关私人文件猜测就绪，不把 root 当作许可 |
| Automation / Apple Events | 实际目标应用的事件调用结果 | 全局状态 unknown。声明只允许发起系统许可，不能批准全部目标应用；拒绝后进入 Automation 设置再执行受阻步骤 |
| Local Network | 实际 LAN 操作的系统结果 | 全局状态 unknown。macOS 15+ 按操作触发原生提示，没有显式全局申请 API；loopback 成功不证明 LAN 已获准 |
| 通知 | Tauri 原生通知插件 | 库存外保留原通道，产品通知偏好与 OS 通知许可分别生效 |
| 网站媒体 | 可见 Browser 面板的原生 WK permission callback 与本次回复 | 宿主摄像头/麦克风状态实时显示；网站请求另行决定。系统批准不等于网站批准，反之亦然 |

`not_granted` 是已证实当前缺权，不声称用户曾拒绝；`unknown` 不被当作成功，也不从错误文案
臆测缺失许可。无焦点应用、锁屏、无显示或不可访问的窗口不自动归类成 TCC 被拒绝。
当前 macOS WK 宿主只接通网站摄像头/麦克风的产品 permission callback；网站地理定位与网站通知
没有对应的现役产品许可回调，设置页不将其展示为已接通能力。它们是待实现/验证的 provider 功能，
不是缺少用户系统授权。未接通的硬件功能同样不能仅凭用途说明宣称已实现。
审查时 WebKit 的公开 geolocation permission delegate 标为 macOS 27.0，现有最低版本为 14.0；
旧定位及通知回调位于私有 delegate，不能用 SPI 临时拼成已支持功能。

`apps/desktop/Info.plist` 声明麦克风、摄像头、定位、本地网络与 Apple Events 的用途；
`apps/desktop/entitlements.plist` 声明音频输入、摄像头与 Apple Events。
开发和正式签名共用声明。helper 的独立签名、daemon 注册和管理员身份不继承 App 的全部 TCC，
也不能借 Terminal 已有授权代替正式 App 的探测。必须分别记录 bundle identifier、签名及实际运行者。

## 授权变化与任务继续

界面在进入和窗口重新获得焦点时刷新现役系统许可。Computer 的 `system_permissions` 来自同一冻结清单：无障碍要求、截图条件和截图默认值由元数据决定。宿主在派发前检查实际请求，不把 OS 就绪缓存进 Agent 配置。授权变化后重启需要刷新的驱动代际，再获取新观察，原输入和 unknown 不自动重放。

缺权沿原 `SYSTEM_PERMISSION_REQUIRED` 与 `system-permission-required.v1` 的 not_executed 载体返回，保留确切 Turn 和操作身份。打开设置或完成系统授权不等于动作已完成；它不建立新授权账本，也不增加 Agent 的动作集合。

macOS worker 由 App 直接启动，正式分发时与 App 同 Developer ID 签名并开启 Hardened Runtime；TCC 归属 NomiFun 的要求仍须在签名包验证，不能借 Terminal 或管理员 helper 的许可替代。Windows 不再提供提权桌面桥，普通完整性 worker 保留 UIPI/锁屏等结构化失败。Linux 的 portal、AT-SPI 和桌面 backend 按实际驱动结果表达，不用配置值伪造系统授权。

原生管理员组件继续复用系统允许保留的注册和认证上下文。服务未注册、需要用户批准、连接失败、启动结果未知、取消及清理分别保存真实结果；管理员设施不代表桌面或设备已就绪。

## 三平台工程状态

以下是源码边界，验证证据必须独立记录。macOS 的一次通过不能填充 Windows/Linux 的原生结果。

| 范围 | macOS 当前路径 | Windows 当前路径 / 未完项 | Linux 当前路径 / 未完项 |
| --- | --- | --- | --- |
| 文件与普通进程 | 共享 canonical 文件/进程与每次启动环境；原登录用户 | 共享合同适用；盘符、UNC、ACL、shim 须 Windows 编译和原生验证 | 共享合同适用；权限、卷、发行环境须 Linux 编译和原生验证 |
| 管理员执行 | 原生 ServiceManagement + XPC + 同一 ProcessSupervisor；须正式签名 bundle 及系统批准 | 原生 UAC、可靠执行与进程树回执、安装/服务路径待平台交接 | 原生系统认证、可靠执行与服务路径待平台交接 |
| Computer | App 直接启动同签名 Cua worker；TCC 和签名包待本次原生验收 | 普通完整性 Cua worker；提权窗口结构化拒绝 | X11/XWayland 与 AT-SPI；原生 Wayland capture 未启用，真实平台验收待补 |
| Browser / 媒体 | 现役 WKWebView；宿主媒体状态、用途和 entitlements 接通；设备调用另验 | 保持 WebView2；OS/provider 媒体许可与设备调用待 Windows 原生验证 | 保持既有真实能力；不以删除 unsupported 标记冒充桌面实现 |
| FDA / Automation / LAN | 全局不可可靠探测项保留 unknown，实际任务结果与对应设置入口 | 不套用 macOS TCC；系统完整性、账户和设备策略按实际操作报告 | 不套用 macOS TCC；显示会话、portal、SELinux 等按实际操作报告 |
| 后台与持续产物 | 临时进程 owner 与原生 launchd 产物区分；服务完整生命周期另验 | SCM/Task Scheduler、用户桌面与服务 session 区分待平台交接 | systemd/发行打包、桌面会话与系统执行区分待平台交接 |

上表中标注“待平台交接”的单元格为未完工程，按平台逐项接续验收；历史交接材料保留在 Git 历史，不代表平台功能已通过。

## 原生验收入口与证据

优先从正式或开发签名完整 App 的默认 Agent 入口执行本机文件与可取消进程回归，再独立执行
普通用户确实不可写目录的管理员子用例。目录和数据根均隔离；先核验原生访问控制，最后核验实际
内容、退出、清理和必要元数据。两者均不要求联网、第三方账号或新工具下载。

本轮正式签名、公证 App 的独立管理员七步验收已通过，覆盖真实 root cwd、只读指令发现、
原生管理员进程的运行与输出、取消、reap 和清理；GUI 仍以登录用户 UID 501 运行。新版 App
PID 69753 曾替换先前验收实例；完成后已协调退出，并通过签名 App 的原生 unregister 入口注销本次 helper，核验服务与原进程消失。各阶段实际 App、报告身份与未覆盖项的逐次证据保留在 Git 历史。

系统许可验收使用同一 App 身份读取库存，按真实设备和桌面调用截图、AX 观察、聚焦、输入、
剪贴板、网站媒体与 loopback/LAN；记录拒绝、撤回、重新授权以及必要重启后的任务继续。
会覆盖私人内容的截图不得提交。首次授权/撤回测试使用专用测试账户或 VM，不能为测试重置开发者
全部隐私设置。只读库存或 Terminal 探针不能替代上述产品实测。

Browser 原生入口包括
`apps/desktop/examples/macos_browser_compatibility_smoke.rs` 的
`--media-permissions --media-user-ready`（可见网站媒体请求）。后者必须经过现役 native fixture
打包/启动合同，不能把裸 executable 当成 App 许可验收。独立 fixture 通过只证明该 fixture 身份。

本轮媒体另用 `scripts/validation/global-os-media-fixture.mjs` 的 nonce loopback 页面，在正式 App
可见 Browser 中由真实按钮发起独立音频/视频请求；它只保存采集与释放的状态元数据，不保存或
上传音视频内容，也不代表 Agent 自动导航通过。最终 e4 签名 App 的实际 PID 为 85132、GUI
UID 为 501；用户完成系统麦克风许可并解锁后，宿主 microphone 库存为 `granted`。在正式
WK Browser 中点击仅音频按钮、网站选择仅本次允许，真实采集取得 1 条音轨、48000 Hz；
音频图为 running 且时钟实际前进，结束后轨道为 ended、AudioContext 为 closed。麦克风的
宿主、网站、设备采集与释放闭环已通过，媒体页随后已确认关闭，当前没有待用户 Allow 的请求。

同一轮独立视频请求在 43 ms 返回 `OverconstrainedError`、0 条轨道。原生设备库存为 camera
suspended=1、connected=1、inuse=0，IOPM 机盖闭合为 true；App camera 许可仍为
`not_determined`。摄像头成功采集尚未通过，不能把该设备环境下的失败写成用户拒绝或 TCC denied。
隔离用户的首次授权、拒绝、撤回和恢复矩阵仍未验证，该边界不改变已接通的媒体 provider 实现。

此前 PID 51317 的 microphone、camera 均为 `not_determined`、v2 尚无 attempt 报告，以及
旧页面等待原生许可、锁屏未继续的记录均保留为历史；该实例已正常 SIGTERM 退出，旧页面和请求
随之销毁。CUA 对 UserNotificationCenter 的工具安全拒绝记录也保留，未换渠道绕过；后续系统
麦克风许可由用户完成。首次合并采集的 `OverconstrainedError` 与当时摄像头 suspended、合盖
诊断独立保留，不覆盖为后续麦克风成功，也不沿用历史零报告描述当前采集结果。

Computer 本次实现与实际平台证据见[专项进展](../handoffs/computer-use-rebuild.zh.md#12-单份执行进展)；已退役驱动的数字、ref/generation 和桥接验收不作为新实现的证明。

`bun scripts/validation/macos-host-signing-fixture.mjs --report /ABS/NEW/report.json` 提供无需账号、
联网或管理员的真实 Mach-O 初签与增量重签回归，逐次校验 strict seal 和三项宿主 entitlement；
成功后删除临时 App，失败保留 fixture。它不证明正式分发公证、helper 注册或任何 TCC 许可。

每次交付分别列出源码实现、编译、确定性测试和原生产品实测，保留首次失败与修复后证据。
未提供外接/网络卷、设备、测试账户、正式签名或认证条件时，明确记为未覆盖；不能写成 OS 不支持。

官方依据：[Apple Events entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.automation.apple-events)、
[Apple Events 用途说明](https://developer.apple.com/documentation/bundleresources/information-property-list/nsappleeventsusagedescription)、
[TN3179 本地网络隐私](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)、
[WebKit 公开 WKUIDelegate](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKUIDelegate.h)、
[NSRunningApplication](https://developer.apple.com/documentation/appkit/nsrunningapplication)、
[NSApplication](https://developer.apple.com/documentation/appkit/nsapplication)、
[NSWindow](https://developer.apple.com/documentation/appkit/nswindow)、
[AX 动作库存](https://developer.apple.com/documentation/applicationservices/1462053-axuielementcopyactionnames)。
