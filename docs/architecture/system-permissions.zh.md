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
| Computer 截图、可访问性、聚焦、鼠标键盘 | 链接进桌面 App 的 `nomi-computer`、`nomi-a11y` | 许可属于实际 App 代码身份与登录桌面；后台系统服务不冒充交互桌面 |
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
- 读取要求安装 owner；会显示宿主 UI 的入口还要求本地桌面信任。旧 Computer 路径只保留兼容。

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

界面通过现役 `useSystemPermissions` 在进入和窗口重新获得焦点时刷新；过期请求不能覆盖更新的
状态。Computer host 在每次真实调用前重检该动作的精确许可，不在 Agent 配置里缓存 OS 就绪。
仅截图需要 Screen Recording；可访问性观察和输入需要 Accessibility；启动程序不依赖两者。

macOS 跨进程窗口聚焦先读取实际前台 PID 与确切 AXFocusedWindow；目标已经拥有键盘焦点时返回已确认的
`changed=false`，不再次投递 AXRaise。需要激活时先读取动作与属性可写性，使用公开的
NSRunningApplication 激活，并仅对已证明支持的 AX 属性/动作发送请求；最后仍核对确切目标窗口、
前台应用和 PID。系统接受激活请求不等于完成；已投递的原生变更报错仍保留 uncertain，不因后来
源码修复或新的观察改写首次不确定事实。

当前 App 自身窗口另走主线程 AppKit 路径：真实 CGWindowID 的 owner PID 必须等于当前进程，
在现役主队列再次核对编号、owner、bounds 与标题，再以公开 NSApplication / NSWindow API 按需
恢复并前置该确切窗口。所有 AppKit 访问和检查均在主队列；外部 worker 只传标量和元数据，以
独立队列任务核验实际 frontmost PID、active 状态、精确 keyWindow、可见且未最小化，允许主事件
循环处理异步激活。1.2 秒是验证阶段预算，已开始的队列任务仍继续 join，不把它当作总硬超时。
本进程窗口不再向 AX actor 发送同步 AXRaise，也不把 self AX handle 搬到其他线程。首次原生变更前
记录 submitted；已投递后无法确认或队列回复失败保留 uncertain，不能返回未执行或成功。

macOS AX 观察保留具有真实非空 AXValue 的静态显示，即使其没有标题、描述或可执行动作。
值仍由现役安全读取路径取得：密码角色或密码子角色不请求 AXValue，也不序列化其内容。
显示节点和按钮使用相同的原生几何、遍历预算、快照代次和实时内容校验；可观察的数值不表示
该节点支持点击。验收读取 Calculator 时先清除可能恢复的旧算式、观察实际 0，再取得新 ref 操作。

macOS `key` 动作的 ASCII 字符在原输入主队列上只读查询当前 TIS 键盘布局，通过公开的
UCKeyTranslate 取得真实无修饰键映射；主键区匹配优先于 SDK 定义的数字键盘重复键，只有没有
主键匹配时才采用真实数字键盘映射。解析后的键码继续使用 Enigo 公开 Key::Other，原修饰键、
反向释放和错误清理链保持一致。没有安全布局映射时明确返回未支持，不默认投递键码 0，不修改
用户输入源或 IME。独立 `type` 文字动作保持布局无关的现役实现；它不能证明 `key` 组合键通过。

确认缺权沿原工具结果返回 `SYSTEM_PERMISSION_REQUIRED` 与 `system-permission-required.v1` 的
`not_executed`，保留确切 Turn 和操作身份。它不是新的授权账本，也不是已经完成的动作。
授权变化后必须重新观察，再发起受阻步骤；旧物理输入与未知效果不能自动重放。
需要重启时保留原 Session 和事实，按现役恢复合同继续，不把“打开设置”当成任务终态。

原生管理员组件复用操作系统允许保留的注册/认证上下文。服务未注册、需要用户批准、连接失败、
启动结果未知、取消及清理分别保留真实结果；不能把没有管理员设施改写成普通用户操作成功。

## 三平台工程状态

以下是源码边界，验证证据必须独立记录。macOS 的一次通过不能填充 Windows/Linux 的原生结果。

| 范围 | macOS 当前路径 | Windows 当前路径 / 未完项 | Linux 当前路径 / 未完项 |
| --- | --- | --- | --- |
| 文件与普通进程 | 共享 canonical 文件/进程与每次启动环境；原登录用户 | 共享合同适用；盘符、UNC、ACL、shim 须 Windows 编译和原生验证 | 共享合同适用；权限、卷、发行环境须 Linux 编译和原生验证 |
| 管理员执行 | 原生 ServiceManagement + XPC + 同一 ProcessSupervisor；须正式签名 bundle 及系统批准 | 原生 UAC、可靠执行与进程树回执、安装/服务路径待平台交接 | 原生系统认证、可靠执行与服务路径待平台交接 |
| Computer | 现役截图、AX、聚焦、输入；实时 TCC 与原登录桌面 | 现役 UIA/输入；高完整性窗口、UIPI 和登录会话桥接待实现/验证 | AT-SPI 已有库代码；完整产品入口仍有 UnsupportedPlatform，X11/Wayland/portal 待平台实现 |
| Browser / 媒体 | 现役 WKWebView；宿主媒体状态、用途和 entitlements 接通；设备调用另验 | 保持 WebView2；OS/provider 媒体许可与设备调用待 Windows 原生验证 | 保持既有真实能力；不以删除 unsupported 标记冒充桌面实现 |
| FDA / Automation / LAN | 全局不可可靠探测项保留 unknown，实际任务结果与对应设置入口 | 不套用 macOS TCC；系统完整性、账户和设备策略按实际操作报告 | 不套用 macOS TCC；显示会话、portal、SELinux 等按实际操作报告 |
| 后台与持续产物 | 临时进程 owner 与原生 launchd 产物区分；服务完整生命周期另验 | SCM/Task Scheduler、用户桌面与服务 session 区分待平台交接 | systemd/发行打包、桌面会话与系统执行区分待平台交接 |

Windows 和 Linux 的接续范围见[Windows 交接](../handoffs/global-os-windows.zh.md)与
[Linux 交接](../handoffs/global-os-linux.zh.md)。交接文档只承接未完工程，不代表平台功能通过。

## 原生验收入口与证据

优先从正式或开发签名完整 App 的默认 Agent 入口执行本机文件与可取消进程回归，再独立执行
普通用户确实不可写目录的管理员子用例。目录和数据根均隔离；先核验原生访问控制，最后核验实际
内容、退出、清理和必要元数据。两者均不要求联网、第三方账号或新工具下载。

本轮正式签名、公证 App 的独立管理员七步验收已通过，覆盖真实 root cwd、只读指令发现、
原生管理员进程的运行与输出、取消、reap 和清理；GUI 仍以登录用户 UID 501 运行。新版 App
PID 69753 曾替换先前验收实例；完成后已协调退出，并通过签名 App 的原生 unregister 入口注销本次 helper，核验服务与原进程消失。各阶段实际 App、报告和未覆盖项见[本轮验证记录](../validation/global-os-2026-10-16.zh.md)。

系统许可验收使用同一 App 身份读取库存，按真实设备和桌面调用截图、AX 观察、聚焦、输入、
剪贴板、网站媒体与 loopback/LAN；记录拒绝、撤回、重新授权以及必要重启后的任务继续。
会覆盖私人内容的截图不得提交。首次授权/撤回测试使用专用测试账户或 VM，不能为测试重置开发者
全部隐私设置。只读库存或 Terminal 探针不能替代上述产品实测。

现有原生入口包括 `nomi-computer/examples/macos_fixture_probe.rs`（仅隔离窗口聚焦）以及
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

`scripts/validation/global-os-native.mjs --scenario desktop --template assistant.general` 使用已正常
启动的完整 App、隔离 owner 的正常登录 JWT 和现役默认 Agent，运行真实 Computer 截图、AX、
聚焦、按钮点击、组合键与退出。仅在系统 Calculator 未运行时启动本次实例；计算清除按钮由实际
AX 语义和新 ref 选取，严格读回 0 后再输入。现代 Calculator 的当前完整算式逐字核验为
`12+`、`12+3`、`12+34`，最终必须出现独立数值 `46`，不能用算式子串、历史数字或按钮文字通过。
脚本供应器在输入回执后有界等待异步 AX 通知，再执行新观察；过期代次仍返回真实失败。
报告只保留截图 PNG 尺寸与字节数、所需代次/ref 和本次计算的核验值，不保留屏幕像素、原始 AX
或其他窗口正文。最后经 Agent Cmd+Q 与宿主只读 PID 检查确认退出。该本机脚本供应器的产品路径
证据不代表外网模型推理，也不覆盖许可首次授予、拒绝或撤回后的恢复。

同一入口的 test-only `--desktop-background-focus --native-data-root /PRIVATE/MARKED/native-data`
只验后台激活。正常 owner JWT 读取当前 endpoint 的系统许可 caller PID，与 private dataset 公告
及 bundle identifier 绑定；宿主仅只读 CGWindowList 和专用验收 App 的实际 bundle URL，确定它的
唯一已有主窗口。默认 Agent 先真实激活该确切窗口，再以新 list 证明本次 Calculator 已在后台，
随后真实激活 Calculator、核验新 AX 归属并退出。两个方向均拒绝将 already-focused 回执算作
已激活；不再依赖 CUA 的 App 内事件来推断操作系统前台变化。历史 CUA setup 未达成和首次 self
AXRaise 崩溃均独立保留。本轮 self-window 修复后的正式签名、公证 App 已独立通过两个方向的
真实后台激活，回执均确认 actual activated；同一 App 的完整截图、AX、点击、原 `shift+=`、
计算结果独立 `46` 和退出闭环也已通过，对应报告时间标识为 `1792174996658`、`1792175068351`。
最终 e4 签名 App 在 PID 85132、UID 501 下又直接通过完整 Desktop 闭环，报告为
`desktop-assistant.general-1792199279748-e3fa2d37-8dae-423a-860f-2acbe7744fff.json`；
原先因锁屏未执行该最终版本回归的状态已成为历史。
这些原生结果与下述源码、SDK 和单测证据分别记录，不结算旧数据中的未知或未完成 Effect。

本轮 self-window 修复的定向 `nomi-computer` lib 为 127 passed / 8 ignored；`nomi-a11y` lib
为 35 passed / 1 ignored。公开 AppKit 调用已用 arm64、x86_64 的 macOS 14 target 做 SDK ABI
和 availability compile-only 检查；这些检查没有链接、执行或控制窗口，原生产品结果另记。

定向验证使用 `cargo test -p nomi-computer -p nomi-a11y --lib`、
`cargo test -p nomifun-app --lib --features computer-use computer_permissions::tests`、
权限 model/hook/Settings 交互测试，以及 `bun run typecheck`、
`bun run check:desktop-ui-boundary`。签名与打包另验 Info.plist、entitlements、helper、
`codesign --verify --deep --strict`；公开分发还须公证和 Gatekeeper 检查。
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
