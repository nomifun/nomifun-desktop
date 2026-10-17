# Browser Workspace 浏览器工作区

当前开发合同由本文、[Agent Session 当前架构](agent-session.zh.md)及源码共同定义。
内置侧栏浏览器是用户工作区；Agent Browser Use 是对该工作区或已授权 Chrome 的受限工具入口。

## 产品边界

内嵌浏览器保持产品的主要浏览场景。兼容性、导航防重、暂停、诊断和失败恢复均在现有原生面板内完成；不将系统浏览器作为主场景重构目标或自动故障回退。系统浏览器打开与已授权 attached Chrome 保留为用户明确选择的部分场景功能。外部浏览器可以提供验收对照，不能替代内嵌浏览器通过证据。

内置浏览器由 Browser domain 按已认证用户与同一个 canonical AgentSession 持有。点击会话右侧的浏览器按钮
即可人工浏览，不要求当前 Agent 启用 Browser Module、配置 Provider 或已有 Browser Resource binding。
打开、重试与用户地址栏命令不写入 Agent binding，不赋予 Agent 工具权限，也不新建第二个 Session。
旧全局浏览器管理页、设置 Tab、侧栏库存和浏览器设置重定向仍保持退役。

页面由原生嵌入 WebView 承载，不使用 iframe、截图流、canvas viewer 或 JPEG 传输充当浏览器。用户和 Agent 操作同一个真实页面。Agent 工作期间，原生用户输入及会改变页面的浏览器栏操作被锁定；必须先停止运行、等待在途操作完成清理，再恢复用户输入。没有用户接管或交还状态。

隐藏面板不销毁标签页。同一用户与 Session 只持有一个 managed 原生 runtime、Profile 和输入协调器；
Agent 的资源定义 ID 改变不会选择新页面或 Profile。`BrowserResource` 只封装 frozen Agent authority，
指向用户已经打开的同一个 `BrowserWorkspace`。每个 Agent operation 仍校验 frozen Actions、exact Provider、
typed Resource binding 和当前 BrowserRunGuard。没有该授权的 Agent 无法观察或操作用户网页。
Agent 选择 attached Chrome 不改变用户的内置浏览器，也不会把用户浏览器按钮变成 Chrome 连接授权。
不同 exact managed Provider 不能静默借用既有实体，必须明确重建原生浏览器。

共享桌面 renderer 的最低客户区为 880×600。聊天与 Browser/Preview 保持水平分栏，最小宽度分别为 360px、340px，另有 32px 会话工具栏。Browser 打开时，Layout 与 ConversationShell 按完整容器宽度及用户偏好的侧栏宽度派生临时让位状态，避免用侧栏收起后的子面板宽度判断恢复而反复收起/展开。主导航与会话列表只在空间不足时让位；窗口变宽、面板关闭、Session 切换或卸载后撤销空间请求并恢复原偏好。相关边框使用 border-box，使实际宽度与计算一致；缩窗仅约束当前显示尺寸，不重写保存的分栏比例或工作区宽度。

空间预留与展开侧栏后的自动关闭只注册给 Browser：关闭只隐藏并 detach 面板，保留网页、runtime 和输入所有权，不销毁标签或释放 Agent 输入锁。Preview 沿用原有侧栏折叠策略，不把会清空标签和未保存编辑的 `closePreview` 交给布局让位回调；Browser 与 Preview 互相切换时同步交接侧栏管理。布局、原生输入与运行中 Stop 的证据范围分别见[持续验收](../validation/browser-platform.zh.md)。

所有 Agent Turn（包括没有 Browser 工具的普通聊天及 attached Chrome Agent）均在 Runtime 预备阶段取得
同 Session 内置浏览器的输入锁。实体创建本身不创建原生网页；运行中首次创建原生 child 时输入已禁用。
canonical Turn 已进入 running、但输入锁尚未得到证明时，用户命令及首次打开明确拒绝，不能短暂显示 UserReady。
预备失败或取消必须等待 retained owner 排空输入操作；settle 阶段保持硬件输入锁，只有 exact Turn 终态成功持久化后才 finish 并恢复用户输入。begin 未发出 guard 时也分为排空与终态后 failed-start recovery 两阶段，终态写入或下游清理失败不能提前解锁。

终态提交与原生 final release 是不同的完成证据。终态已经写入而 release 失败时，SDK 重试必须核对
同一 root 的 canonical receipt、原 Runtime terminal、delivery、Snapshot 与 route，再只重试 finish；
不追加第二个终态、不把失败改成取消。执行代际为零的启动前取消仍使用独立的 exact cancellation witness。
完整 Runtime teardown 成功后，Kernel 资源上下文只按对应实例的 Arc 身份退出 Weak cache；即使旧 handle 仍被持有，
后继 Runtime 也不能复用已关闭上下文，迟到的旧 cleanup 不能驱逐新的实例。

正式内置浏览器 Profile 固定按认证 owner 与 Session 隔离。Windows 保留目录
`browser-v4/agent-sessions/<identity-hash>/`，namespace 为 `nomifun.browser.session-managed-profile.v2`。
macOS 14+ 使用公开 `WKWebsiteDataStore.dataStoreForIdentifier` 的独立持久 store：
从 `nomifun.browser.webkit.session-store.v1`、编译渠道、canonical 数据根路径、已有可信 `storage-generation`、
认证 owner 与 canonical AgentSession 以长度前缀 SHA-256 稳定派生 UUID。身份不含 Agent resource definition、
项目路径或 Agent persistence 参数。重启保持同一 store；开发/生产/测试及不同数据根不共享登录态。
移动或复制数据根会选择新的登录 namespace，站点数据不随 portable backup 搬运。恢复为新目标创建 generation，
不修改源安装的系统 store。重置使用原始数据根、已归档的可信 generation 与 canonical Session owner 推导旧 store，
等待原生删除完成后才由既有 dataset coordinator 完成重置，不建立第二套 Session 或 Profile 账本。
归档身份信息缺失或无效时拒绝继续；CLI 或无 Browser 构建缺少原生清理端口时，重置保持 pending，须在 NomiFun Desktop
完成。会话删除同样要求原生清理证明；站点清理等待 WK 公开回调，不能只删目录或由浏览器自行补建 generation。
WK 不继承 Safari 或旧 CEF Cookie；切换后允许首次重新登录。旧 CEF 用户文件保持原样，不读取、扫描、迁移或自动删除，
处理旧资料须由用户明确选择；切换浏览器不额外清空聊天、模型配置或凭证。
旧 Profile 的存在与内容不参与新版启动、身份派生、恢复或清理决策，不维护旧数据格式、双读、回退或专用迁移流程。
保留磁盘上的用户资料不构成运行时兼容合同；新版只管理当前 WK store。重置归档中的 canonical 表若存在，
必须满足当前身份字段合同，缺失字段不能当作“没有会话”放行。
独立原生 conformance fixture 可以使用 Ephemeral Profile，不改变正式用户侧栏的策略。
canonical Session 删除先关闭真实 runtime，再安全删除该 Session 的精确用户 Profile，即使它从未拥有 Agent Browser binding；
重启后也按同一身份计算，无第二份 Profile/授权账本。已结算效果引用的 Agent 资源定义仍按 canonical 契约保留。

内嵌浏览器不提供 F12、Inspect 或用户可见 DevTools 窗口。普通页、popup 与宿主维护 view 都显式
禁用该入口，边界扫描防止重新开启。底层 WebView2 协议调用仍用于 Agent 观察、原生输入、
Frame 与生命周期，但仅属宿主内部实现，不构成 DevTools 产品功能。

会话内嵌浏览器在 Windows 使用 WebView2，在 macOS 14+ 使用系统 WKWebView；均使用操作系统网络栈，不安装应用转发代理、IP/端口白名单或网络设置。
这保留系统代理、证书、localhost、LAN、WebSocket 和 HMR。最小边界是顶层导航仅限无内嵌凭据的
HTTP(S)，并且 Browser child 无 Tauri capability、local trust、backend credential 或任意文件权限。
后台 `web.research` / render 仍是不同消费者，保留严格公网 DNS/IP pinning，不与可见浏览器混用。

浏览器菜单的“重新打开浏览器”需要明确确认：关闭所有网页并丢弃未保存网页内容，但不清空会话消息或项目文件。
后端持有同一 Session 的 operation fence，在 canonical 空闲边界校验浏览器代际。
没有原生 guard 时按普通空闲关闭处理；已经发出 guard 且 final unlock 失败时，必须确认同一 Operation 的
canonical 终态、已完成的 native settlement 与对应旧 Workspace 身份，才允许确认关闭 exact generation。
canonical running 或未完成的 settlement 仍明确拒绝，重建入口不取消活跃 Agent、不生成终态。
`turn/paused` 保留同一 active Operation 与 native checkpoint 恢复权，不属于这里的最终终态。
暂停时仍拒绝重建；用户须先经正式 Stop/cancel 终结该 Operation，并取得相应 Runtime 清理证明后再重开。
旧 Workspace 的真实关闭证明供 SDK 随后确认既有终态并重试 final release；新的用户 Workspace 是另一代实体，
不会被旧 owner cleanup 释放，模型不会重放、原终态不会改写。

菜单已有“打开系统下载目录”，默认保存对话框也使用宿主解析的 OS Downloads 目录，不再默认 Home。
用户另选保存位置仍由原生保存对话框决定，菜单不会声称该系统目录包含所有自选位置的文件。
此命令只携带当前 Runtime 代际，不接受路径/URL，不启动或更换浏览器；Agent 运行中拒绝、Agent 工具不可调用。
打开失败只显示轻量提示，不销毁当前网页、不自动重试。Windows 使用原生 OS handoff，macOS 使用公开 NSWorkspace；OS 接受请求不等于完整文件管理器视觉验收。

macOS 复用 Tauri 已建立的 AppKit 主线程与事件循环，创建真实可见的 WKWebView child view。
WK/AppKit 对象由主线程创建、操作和释放，不启动第二个 NSApplication，不下载或打包浏览器运行时，
不打包系统 WebKit.framework。WebContent 进程由系统 WebKit 管理；不保留 CEF helper、guardian、
framework preload、message pump、allocator 特例或隐藏回退内核。

## 浏览器身份与导航恢复

macOS 普通页与 popup 在首次请求前通过同一原生入口设置桌面 Safari 兼容身份，当前 profile 为 `macos-desktop-webkit-safari17-v1`。`Version/17.0` 是兼容身份基线，不是测得的 Safari/WebKit 运行版本；实际 OS 信息独立报告，未知的 WebKit 元数据保持 unknown。UA 不参与 Session/Profile 身份派生，不清除 Cookie，不打包 Safari 或额外内核，不通过 JavaScript getter 或单独请求 header 改写身份。Windows 保持其原生身份。

Page 统一处理请求、开始、redirect、commit、finish、取消、失败和内容进程终止。`load` 摘要携带导航控制序号、阶段、可显示内容状态、整数进度、安全错误及尝试/内容地址。内部 bootstrap 来源单独标记，不算用户文档；真实 about:blank popup 可以承载文档。取消/停止映射为 `Stopped`，不表示新页面已成功；`NSURLErrorCancelled` 按 domain 与 code 分类，替代导航与用户停止明确区分。原生导航对象及 dispatch receipt 保留精确关联，旧 Stop、迟到回调和旧 resize 不能影响后继页面。

面板挂载/布局、原生 child 可显示性和输入锁分别表达。没有内容的失败、停止和进程终止在原生主线程隐藏 child，renderer 显示明确状态及重新输入地址入口；保留旧/部分内容时继续显示网页并说明尝试地址与内容地址。每次 native show 重新应用当前 Page mask。Agent 恢复导航等待布局且保留原有授权/输入锁，不依赖旧文档可执行；无内容读取及时返回 `BROWSER_PAGE_FAILED`、`BROWSER_PAGE_STOPPED` 或 `BROWSER_PAGE_CRASHED`。有效旧/部分文档可以重新观察和截图，并携带相同安全加载摘要。执行不确定性保持既有语义，不自动重放 DOM、POST 或 Agent 操作。

用户下载的面板已呈现状态独立于网页内容遮罩：首次直接下载可以继续目的地确认，空内容隐藏不撤销选择；真正的面板隐藏、输入 owner 切换与进程终止仍撤销未完成选择。DOM、媒体权限与上传选择继续要求真实文档可见。

Windows 在 `content_state=None` 的所有阶段保持 controller 与 child HWND 隐藏，包括失败后新请求的 requested/provisional；新请求本身不清除 controller 中的旧错误像素。真实非错误文档提交后恢复同槽显示，不依赖 resize/show 重放。`CurrentDocument` 和 `RetainedDocument` 按原有面板及捕获合同显示，真实 HTTP 403 内容也不因状态码被隐藏。Pause 只控制当前标签；其他已打开标签继续运行，已有命名窗口的独立导航不等同于新的 popup 创建。

主文档导航诊断是 Page 所有的临时有界轨迹，关闭 tab 时释放：最近 32 次尝试、最多 128 条事件及 128 KiB 序列化事件预算。容量淘汰增加 dropped，关联证明失效单独表达。主 frame 响应 callback 没有 WKNavigation；无法证明关联时只保留未关联样本，不用同 URL/latest sequence 推断当前响应状态。此能力不代表 console、子资源或 Fetch/XHR 网络诊断可用，原有 unavailable 语义保持。

菜单的复制导航诊断由用户显式触发，经当前 owner、已有 Workspace 和精确 target 读取；不会初始化 runtime、创建 Agent grant 或自动上传。地址栏保留原地址，诊断/Agent 输出使用现有安全 URL 投影删除 userinfo、query 和 fragment，并限制文本长度；不收集 Cookie、认证头、正文、表单或 NSError 任意描述。网络/站点异常只提供显式单标签恢复和慢加载提示，不主动无限刷新，不绕过验证码，也不清理整个工作区。

两个平台均保留宿主 dispatch、原生 attempt 和公开 action 事实。dispatch 序号与原生导航序号独立；没有精确关联的样本不取得文档身份。Windows 的 NavigationId、成功/错误/取消/下载由原生 Page owner 维护，旧 Wry PageLoad 回调不再写浏览器业务状态。`counters` 在轨迹淘汰后仍保留累计数，关联损失与容量丢失分别表达。macOS 的 policy action 记录公开导航类型、method 和来源 frame/origin，但不能证明它属于最新 WKNavigation 或真实物理手势。

Windows 的 Back/Forward 推进当前控制 fence 并记录宿主 dispatch，同时保留真实文档的阶段、进度、问题和原生绑定；同文档历史切换不会等待新的完整文档加载。只有真实 NativeStarting 才创建新的 Loading 尝试，`Page.navigateToHistoryEntry` 的空回复不证明导航完成，SourceChanged(false) 只更新元数据。公开 `load.navigation_sequence` 可以在没有新原生文档尝试时推进，因此取消和轨迹关联仍使用准确 NavigationId 及其原 attempt 序号。后继回复不能证明迟到历史来源时继续保持 Unknown。BeforeUnload 保留原有精确命令/对话框 scope 和真实原生决定，不恢复快照或补造已取消的尝试。

macOS 的 Back/Forward 同样先推进控制 fence 并记录 dispatch，保留现有文档与原生 attempt 状态；只有准确 pending `WKNavigation` 的 `didStartProvisionalNavigation` 才将同一 dispatch 绑定到新文档 Loading。命令返回、URL/标题变化和 metadata 采样都不证明新文档已开始或完成。已结算文档的地址变化仍保守推进 document generation，使旧 DOM 引用失效，但不伪造主文档请求或新 attempt。Stop/取消 receipt 区分准确 History dispatch 与原 native attempt，不把最新控制序号重新绑定到旧导航。[持续验证边界](../validation/browser-platform.zh.md)单独记录；unit 通过不等于 WK 生产 GUI 已验。

`load.navigation_control` 表达可选的重复跳转提示与标签暂停。提示只看独立主文档 attempt 的原始地址等价性，在内存使用页内随机盐 hash；redirect、普通子资源或未关联 action 不作为刷新次数。提示不自动暂停。用户明确暂停后，原生 policy 阻止页面发起的后续主导航和 popup，保留可用内容；继续只解除 hold，不自动刷新。用户明确地址栏/刷新/历史命令按原有 guard 解除 hold。Stop 只停止当前加载，不能声称阻止未来网页 timer 或全部 Fetch/XHR。Stop/Pause 可以取消同标签当前的人工导航，但必须等待原操作原生清理后完成，不能取消 Agent operation、提前解锁或重放 Stop 到后继文档。

Windows 的 NavigationStarting.Cancel 可能发生在 GET 已发出之后，因此暂停还需要精确 root Document 的 CDP Fetch 请求阶段拦截。首次明确暂停后，拦截器保留到该原生 controller 关闭；继续只转换逻辑状态，不调用 Fetch.disable 释放尚未送达回调的旧请求。原生 root frame、Fetch.networkId 和 Network 的最早创建时间共同证明请求归属；不修改 HTTP method、headers 或 body。当前共享时钟证明限定为报告 invariant TSC 的 x86/x64 CPU，其他 CPU 不提供 navigation_control，Pause 返回 UnsupportedAction，普通导航与 Stop 保持可用。该能力范围和实机通过范围分别记录在[持续验收](../validation/browser-platform.zh.md)。

Windows 普通页、popup 与站点维护 view 使用显式 `WebviewBuilder::external_page()` 创建模式。它在首次文档前跳过 Tauri core/plugin 初始化、应用 URI scheme、IPC 与应用全局脚本；默认 main/companion 行为保持。Wry 在没有 IPC handler 时不安装 `window.ipc`。同一 Tauri 窗口、事件线程和原生 WebView2 接口继续承载网页，不修改 getter 或加载后删除脚本。精确依赖补丁与许可证见 [vendor 说明](../../vendor/TAURI-PATCHES.md)。原生环境读取实际 Runtime 版本、Profile 与 UA；有效目录/持久性与所选 Profile 不一致时拒绝外部导航，不清理或修复 override 选择的数据。UA 是可选诊断：属性接口不可用、读取失败或值无效时保留 identity 缺失并记录固定原因，不阻断普通页或 popup 创建，也不猜测或截断为测量成功。诊断只提供进程内不可逆 Profile token 和覆盖是否存在，不导出用户路径或任意启动参数。

此处 external_page 表达原生内嵌 WebView 加载外部网站的初始化边界，页面仍在当前产品窗口中；该 API 不打开或选择系统浏览器。

Windows 仅在创建原生 view 时准备普通本地盘 UDF 路径，核对该 SDK 输入与原 ProfileStore 目录是同一目录对象；Session namespace 与清理路径保持原值。外部导航前，SDK 的有效 UserDataFolder getter 也必须使用支持的表示并指向该目录。原生对照已复现 verbatim 表示导致 Cookie 丢失，因此即使 override 指向同一目录，仍在使用不支持的表示时拒绝导航；普通匹配 override 保持可用。UNC、长路径及特殊名称无法安全转换时在创建网页前拒绝，本次不宣称支持这些数据根。关闭与应用重启后的 Cookie、LocalStorage、IndexedDB 保留、Session 隔离及 override 拒绝证据见[持续验收](../validation/browser-platform.zh.md)。

Windows 的 iframe auto-attach 保留启动等待，在首段 iframe 脚本前安装文件选择策略。Renderer 的 worker/worklet 使用独立归属，只通过准确原生 session 及时恢复，收到确认后解除调试附着；这些辅助 session 不取得 DOM 路由或 Agent 权限。辅助恢复失败保留有界归属，等待原生 detach 或 controller 关闭；辅助资源准入失败不使无关 iframe 路由失效。既有有界事件通道在丢失生命周期事件时仍明确报告关联失效。Worker 与嵌套 Worker 的实机通过范围独立记录在整改证据中。

macOS conformance 可以在初始化 Engine 时选择不可变的 `NativeWebKit` 对照身份，整个 Runtime 与 popup 继承同一策略；正式用户侧栏仍默认使用既有 Safari17 身份。实际原生 UA 通过公开隔离世界读取证明，读取前保持 unknown，不从 OS 或已安装 Safari 推断。身份 A/B 的结果和正式用户侧栏验收分别记录，未取得实机对照前不改变默认策略。

## 平台能力矩阵

| 能力 | macOS WKWebView | Windows WebView2 |
| --- | --- | --- |
| 可见页面、标签页、导航、历史、刷新、停止、普通用户输入 | 保留原生工作区 | 保留现有实现 |
| 页面观察、文本、元素引用、截图 | 主文档语义观察；公开原生 snapshot | 保留协议观察与原生截图 |
| 普通点击、填值、选项选择、滚动、表单提交按钮 | `semantic_dom`；点击仅单次左键语义操作，输入替换文字 | 保留 `browser_input` |
| 持续拖拽、hover、组合键、右键/中键/双击、pointer capture、复杂 Canvas/编辑器 | 明确 `BROWSER_UNSUPPORTED_ACTION` | 保留既有能力及原有 frame 限制 |
| iframe、closed shadow DOM | 未覆盖内容不产生可操作引用；显式 `unobserved_frames` | 保留已有 Frame 路由 |
| 用户文件选择与下载 | 公开 WK/AppKit 原生流程；不按 HTML `accept` 预筛选 | 保留现有原生流程与筛选 |
| Agent 自动上传/下载、开发者 evaluate、全量 CDP 网络诊断 | 首版明确不支持 | 保留已有能力及授权 |
| 独立持久登录态、会话删除与站点清理 | macOS 14+ 独立 WKWebsiteDataStore | 保留目录 Profile |

`semantic_dom` 不伪造 `isTrusted`，也不是 user activation；DOM API 或事件分发完成不代表网站已完成业务操作，
Agent 必须重新观察结果。网站要求真实用户激活时由用户按既有 Stop → 等待清理 → 恢复输入流程处理。
工具观察与用户工作区携带 `interaction_capabilities`，工具 schema 的动作全集不代表每个平台均支持。
不支持错误明确禁止同动作重试；单项不支持不会关闭普通浏览能力。浏览器菜单同时说明常见限制。

公开 `WKOpenPanelParameters` 只提供多选、目录等选项，不提供 HTML `accept` 的 MIME/扩展名提示。
因此 macOS 文件选择器不预筛选文件类型，用户须选择网站接受的类型；此限制不扩大文件访问权限，
也不改变 Windows 原有的文件筛选能力。

`browser/observe` 默认只返回语义观察；显式 `screenshot:true` 获取同一原生标签页的可见截图。
工具入口先校验当前精确模型路由的 ImageInput 能力，返回 typed image part，不把 base64 当作模型文本。
截图与观察的目标代际必须一致；网站对话框阻塞时返回 `screenshot_status=awaiting_dialog`，不制造图片。
此截图不使用 Computer 屏幕录制权限、不使用 Headless 渲染，也不作为 renderer 网页显示路径。

## 文件选择与下载

macOS Agent 上传仍为已批准 WKWebView 功能降级中的明确 `UnsupportedAction`。这是实现缺口，不是系统权限拒绝；人工文件选择走自己的原生路径。

Windows 的可选 `browser.upload` 只消费已观察的可见标准 file input，或由当前页真实原生 chooser 事件产生的输入；实际调用继续校验已冻结的上传 Action。全局文件来源改造尚未完成：`engine_browser_tools.rs` 仍把 Session 默认 workspace 交给 `BrowserUploadScope`，其 `root: Dir`、`relative_components` 与 no-follow walk 拒绝本机绝对路径、`..`、链接及 junction。这是待 Windows 设计、实现、编译及原生验证的应用限制，不是新产品合同，也不表示操作系统不支持。[Windows 接续任务](../handoffs/global-os-windows.zh.md)要求支持 Session 默认目录相对路径、本机绝对路径、合法链接、UNC 与跨卷来源，并验证目录句柄并发替换、ACL、来源身份和读取前后 metadata；regular-file 类型检查、取消和快照保留必须继续可靠。宿主继续保留名称、字节和修改时间的快照；单次最多 16 文件/64 MiB，每 Runtime 保留最多 128 文件/256 MiB，直到真实原生关闭后释放。工具返回或页面重置不能使仍被网页持有的 File 对象失效。

WebView2 通过 `DOM.setFileInputFiles` 交付，不伪造 input/change；空列表拒绝。Frame/document/Session/当前请求身份先于交付校验，新 iframe 首段脚本前安装 chooser 策略。未知路由、策略失败或失效观察不授予文件权。目录上传、File System Access、Agent 下载/沙箱发布及完整异常退出回收仍须独立交付与验收。

用户 Open/Save 对话框由隔离 helper 通过有界私有协议拥有。只允许可见、活动、闲置标签申请；隐藏、导航和 Agent 入场撤销未确认选择，取消等待真实进程树退出。HTML `accept` 在 Windows 只提供安全解析的原生筛选提示并保留所有文件选项；macOS 公开 WK API 不提供相同筛选信息。筛选不成为文件授权或内容校验。

用户下载沿原生 DownloadStarting/下载 owner 执行；每标签有界，最多四个在途请求、一个待选 Save。Agent 入场后已获授权的传输可继续，新下载仍拒绝；关闭标签取消并等待原生终态，清理失败保留重试所有权。默认下载 UI 不得绕过输入锁。实际网络失败、重定向、权限和平台矩阵见 [持续验收](../validation/browser-platform.zh.md)，不从已有局部结果推导完成。

## 独立网页搜索与渲染

`web.research` 是独立可选能力，不授予 Browser 自动化，也不要求模型原生搜索。匿名 `headless_page` owner 使用临时 context、受限来源转发与精确进程/Profile 清理，不读取可见标签或登录态。搜索只把固定引擎域交给固定公网 DNS；任意内容渲染沿系统 DNS 的严格公网校验、连接 pinning 与 GET-only 转发，保留浏览器 CORS/Referer，不放行 Fake-IP 或私网。

Windows/macOS 从已知位置发现 Chrome 120+ 并冻结完整版本/文件指纹，实际创建页面前复核。发现不扫描任意 PATH、不执行或安装浏览器；无合格依赖明确 unavailable，不阻断桌面启动。macOS 指纹包含真实 framework、helper、Libraries 与 Resources，不只固定 launcher。

Knowledge 冻结 exact Provider/source，通过 Kernel 非 Agent operation 和独立 HeadlessRenderRuntime 逐次准入，不伪造 Session/Snapshot/workspace。输入是 URL，输出为 `final_url/html/html_truncated`；HTML 最多 256 KiB，转换由 Knowledge 自己负责。最多两个活动任务、16 个总在途/排队任务及30秒排队期限；取消保留所有权直到 join，清理失败或 panic 关闭准入并保留 shutdown 失败。缺少 runtime 或来源不安全时不回退普通 HTTP。

可见 WK/WebView2、Attached Chrome 与后台 Chrome 引擎分别验证；任一路径通过不能补证其他路径。实际公网渲染、真实模型与设备覆盖的未完成项见 [持续验收](../validation/browser-platform.zh.md)。

## 系统浏览器与退役边界

Attached Chrome 使用显式授权连接。非观察动作期间的临时 debugger 绘制 override 持续到 run settle 后撤销；不能在单次 wheel ACK 后立即释放。启停不确定时关闭自己的连接，不关闭用户浏览器或页面，不修改 Profile、启动参数或 OS 窗口。

宿主通过明确 `BrowserRuntimeTarget` 及持久 owner/source/cron/execution 归属分类，不从缺少 Workspace 猜 Headless。普通聊天可不选 Browser；选择后缺少原生宿主明确失败。渠道、定时和执行步骤的后台自动化仍无现役 v2 owner，不能恢复旧 Hub/Lane、租约、私有引擎、共享 vault 或旧 Profile 导入。独立搜索与 Knowledge render 具有自己的 owner，不受此限制替代。

## 源码与验收

- [用户工作区 owner](../../crates/backend/nomifun-app/src/router/browser_workspace.rs) 与 [Agent 工具接缝](../../crates/backend/nomifun-app/src/router/engine_browser_tools.rs)。
- [Knowledge 装配](../../crates/backend/nomifun-app/src/router/knowledge_browser.rs)。
- [平台接口](../../crates/backend/nomifun-browser-platform/src/lib.rs) 与 [引擎](../../crates/agent/nomi-browser-engine/src/lib.rs)。
- [持续验收方法与未完成事项](../validation/browser-platform.zh.md)。

改动需运行受影响的原生、平台、Runtime 或 UI 定向检查及现役 Browser 边界检查；renderer/UI 规则改动运行 `bun run check:desktop-ui-boundary`。历史实验日志、截图和执行次数由 Git 留档，不作为新的兼容合同。
