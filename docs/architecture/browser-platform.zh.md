# Browser Workspace 浏览器工作区

状态：Browser Workspace v2 实施中。[批准的设计](../specs/2026-09-13-browser-workspace-v2.zh.md)定义完整目标，[实施记录](../specs/2026-09-13-browser-workspace-v2-progress.zh.md)列出已取得的证据与剩余工作。

## 产品边界

浏览器属于会话，通过该会话工作区的“浏览器”按钮打开。旧全局浏览器管理页、设置 Tab、侧栏库存与轮询、旧浏览器设置重定向均已移除；不为这些 UI 路径保留迁移或兼容入口。

页面由原生嵌入 WebView 承载，不使用 iframe、截图流、canvas viewer 或 JPEG 传输充当浏览器。用户和 Agent 操作同一个真实页面。Agent 工作期间，原生用户输入及会改变页面的浏览器栏操作被锁定；必须先停止运行、等待在途操作完成清理，再恢复用户输入。没有用户接管或交还状态。

隐藏浏览器面板不销毁标签页。更换 Provider 时，不能静默复用绑定于另一份 exact Provider 的 Workspace。

内嵌浏览器数据按已认证用户与会话隔离，不按项目共享。持久目录为 `browser-v2/conversations/<identity-hash>/`，
临时或无 workspace 会话仍使用临时数据目录。用户入口和 Agent 入口调用同一个目录选择函数；项目路径不参与
目录身份计算，也不要求项目目录存在。不另建 Profile 元数据文件，不迁移或读取旧项目共享目录。
这是内嵌浏览器的边界，不改变独立 `nomi_system_browser` 使用用户已登录系统浏览器的能力。

内嵌浏览器不提供 F12、Inspect 或用户可见 DevTools 窗口。普通页、popup 与宿主维护 view 都显式
禁用该入口，边界扫描防止重新开启。底层 WebView2 协议调用仍用于 Agent 观察、原生输入、
Frame 与生命周期，但仅属宿主内部实现，不构成 DevTools 产品功能。

会话内嵌浏览器使用 WebView2/操作系统原生网络栈，不安装应用转发代理、IP/端口白名单或网络设置。
这保留系统代理、证书、localhost、LAN、WebSocket 和 HMR。最小边界是顶层导航仅限无内嵌凭据的
HTTP(S)，并且 Browser child 无 Tauri capability、local trust、backend credential 或任意文件权限。
后台 `nomi_local_websearch` / render 仍是不同消费者，保留严格公网 DNS/IP pinning，不与可见浏览器混用。

浏览器菜单的“重新打开浏览器”需要明确确认：关闭所有网页并丢弃未保存网页内容，但不清空会话消息或项目文件。
后端在会话空闲边界校验浏览器代际，先退出缓存的 Agent，再销毁旧 Workspace；正在启动/运行/收尾的 Agent 不会被这个入口取消。

菜单已有“打开系统下载目录”，默认保存对话框也使用宿主解析的 OS Downloads 目录，不再默认 Home。
用户另选保存位置仍由原生保存对话框决定，菜单不会声称该系统目录包含所有自选位置的文件。
此命令只携带当前 Runtime 代际，不接受路径/URL，不启动或更换浏览器；Agent 运行中拒绝、Agent 工具不可调用。
打开失败只显示轻量提示，不销毁当前网页、不自动重试。Windows 原生 OS handoff 与页面保留已有 smoke 证据，
不将 OS 接受请求等同于完整 Explorer 视觉验收；macOS 实现仍后置。

## 当前实现

- 会话菜单已接用户专用站点数据清理：明确确认后关闭此会话网页，在相同 Profile 的短生命周期空白原生 controller
  上等待清理回调，再销毁该 controller；无其他站点浏览/工具能力，也不进入 Tab 列表。未知任务不按超时放行，
  恢复关闭前仍等待原生 Pending 收尾。原生双持久 Profile 对照、预分发取消、代际/Agent 拒绝和 UI 确认回归通过；
  主 GUI 清理点击及完整清理中崩溃/关闭矩阵仍待验收；最新独立 conversation 预览已包含本次代码，但尚未手动启动。
- Windows 主 GUI 已取得真实模型计数器闭环证据：从工作台选择能力，Browser 真实点击复现 +2，Read/Edit 修改
  app.js，重新加载后真实点击得到 1/2/3，结束后用户同页点击到 4。首轮请求未提供的 Glob 被拒绝，补充提示后
  才继续完成，不是一次无干预成功；尚不覆盖完整框架启动/构建或完整 GUI 交付。具体失败与回执见实施记录。
- 搜索已通过主应用两条接线路径：没有原生搜索 trait 的 Chat 模型可调用独立本地搜索；Responses 模型同时选择
  本地搜索、原生搜索和 citation.render 时，两种工具名/Provider/引用 URL 保持独立。共存用例的本地搜索使用真实
  Chrome 公网结果，原生搜索使用本机协议夹具，不是远端 OpenAI 服务可用性或 GUI 验收。
- 桌面宿主创建原生 child view，拥有视图生命周期和输入门。
- BrowserWorkspace 将 Runtime/Tab 权限限定到已认证用户与 Conversation。
- BrowserRunGuard 来自权威 Agent turn 生命周期，不接受 UI 或模型 JSON 创建/解除。
- 元素引用包含 Runtime、文档和观察代际；原生操作消耗观察引用。
- Nomi Browser Tool 受冻结的 capability set 和 exact Provider 限制。
- 原生宿主在编译 Agent 配置前，从已物化的 bundled Provider 注入 Browser Role v2 的 exact 默认绑定；不让用户配置内部角色，不写迁移或旧安装绑定表。
- renderer 负责浏览器栏和原生视图位置，不绘制网页帧。

可选 `browser.evaluate` 已接正式 Browser 工具与原生运行锁，未选择时不暴露 operation。
Windows 仅允许当前 root 页为 HTTP(S) localhost、127.0.0.1 或 [::1]，使用独立开发者 JS world 读取/修改 DOM，
不访问页面全局变量或语义观察 world。请求绑定 Tab/Runtime/文档代际，执行前后校验，单次 nonce 防止上下文编号复用。
脚本使用 browser 内部 5 秒执行超时、64 KiB 源码/128 KiB 返回值上限；不使用可能影响下一次执行的全局终止指令。
返回 `execution_kind=developer_script`，不是 BrowserInput；执行后必须重新观察元素。网站 dialog 沿现有 PendingWork
保留原调用，回复携带脚本结果，不重放表达式。脚本异常与原生无法确认完成均明确失败，改动不回滚。
此能力只等待同步表达式，Promise 返回为错误；不是任务调度器，脚本自行创建的页面事件、定时器/网络效果不承诺撤销。
工作台以“浏览器开发者脚本”说明该边界；正常前端用户操作验证仍用 browser.act。

Windows 用户快捷键已接原生 AcceleratorKeyPressed 与既有浏览器栏：Ctrl+L/T/W/R、F5、Alt+左右。
原生回调先确认 handled，再异步通知主应用；不在同步输入回调中等待跨进程操作。事件携带会话与完整页面目标，
原生/renderer 复核运行锁与文档，旧会话、旧文档、重复按键不变成新页面命令。AltGr、Shift/Windows 组合不拦截。
地址焦点与新标签草稿在浏览器栏处理，刷新/关闭/历史复用已有 user_command，不增加 Agent 能力或第二套导航执行器。
原生注册跟随 controller 的确认关闭清理。已有编译/纯按键映射与 UI 测试；真实主 GUI 已验证原生页焦点下 Ctrl+L
聚焦并选中地址，以及 Ctrl+T/Ctrl+W 空标签草稿不丢失原页面。其余物理快捷键仍按既有命令/原生测试覆盖；
macOS 的 Command 键/AppKit 接线后置，不能从这些 Windows 代码推定已支持。

Windows WebView2 已有真实输入和生命周期 smoke 验证。主应用 UserReady 的浏览器展开、网站对话框、中文回显和等待中关闭已有实际视觉验证；真实 StepFun 已完成同页点击复现 +2、Read/Edit 修复代码、重载并点击验证 1/2/3、结束后用户继续到 4。完整前端框架启动/构建、macOS 原生执行及正式发行矩阵仍未完成。

Windows 当前支持同一个 native protocol session 内的真实 HTML drag/drop。WebView2/Chromium 公共虚拟输入在跨
OOPIF 时不能对真实源 renderer 完成 `dragend(move)`，因此不同 native frame session 的 drag 在 mouseDown 前
明确返回 unsupported。不使用 DOM 合成事件、系统全局鼠标或关闭站点隔离伪造支持。

Windows 主界面进一步通过本机确定性模型验证：普通聊天运行（没有 Browser Tool）也锁定已打开原生网页，真实点击
不能改变计数；模型正常完成或用户点击聊天 Stop 后，原页恢复点击，计数与中文表单状态保留。该验收不向外网发送
模型请求，不代替在途 Browser action 的 Stop GUI 或真实模型自主前端开发验证。

应用插件初始化不得改写网页标准 API。dialog 插件保留显式 native API，但不注入全局 alert/async confirm 替换；
notification 初始化只在第一方顶层应用文档中运行，外部浏览器页保留原生 Notification。IPC ACL 不变。此边界来自
主应用实际发现的 confirm 返回 Promise 问题，原生 smoke 现使用相同适配层，不能再以未装配应用插件的示例代替验收。

Windows Tab 现已在首次导航前自动安装原生网站对话框处理，popup 则在绑定前安装。统一的在途任务注册表保留被
dialog 打断的输入、导航、观察及截图；Agent 答复经过当前 run guard，空闲用户使用标签页内对话框和 user gate。
暂停的观察不提供可操作元素引用，暂停的截图返回 dialog 元数据而非假图片。初始文档 prompt、beforeunload 接受/取消、
异步 dialog 及 popup 首屏 dialog 已有正式宿主 smoke 证据；等待期间可精确关闭单个标签，销毁后只收束其作用域内工作，不取消整个 run 或其他页面的 dialog，并行关闭按标签串行化。
完整 frame/创建/取消竞态和主应用视觉验收仍需继续。

网页 iframe 坐标使用原生 `DOM.getBoxModel` 内容四边形与投影变换，不再手工解析 CSS transform。
同进程祖先先从协议 session 根坐标还原到父文档坐标，再逐层验证命中；OOPIF 保持各自 session 边界。
输入前复查四边形与 viewport，退化、非有限或已变化的几何不产生输入点。静态透视、父元素 perspective、独立 3D
属性、motion-path、同进程嵌套透视点击/中文输入与父层遮挡已通过真实验证；不代表所有动态变换或跨进程拖放均已完成。
80%、125%、150% 的原生 WebView2 页面缩放已验证主页面远端按钮、嵌套投影点击和可信中文输入；每档读回 ZoomFactor
并等待实际 CSS 视口匹配。150% 下的大幅页面滚动也保持正确输入位置。这是页面缩放验收，不是设备/手机仿真或新增产品模式。

`cargo run -p nomifun-desktop --example browser_workspace_smoke -- --agent-only` 使用隔离的正式 DesktopServer、配置 API、
能力编译和 Nomi turn，配合本地确定性模型协议端点，驱动可见的真实 WebView 完成观察、输入、点击和诊断读取。
它验证输入门、普通会话回复和退出清理，但不是外部真实模型的自主开发/修复能力验收，也没有新增产品测试界面。

同一用例还覆盖停止等待中的模型、丢弃迟到操作、下一轮重新观察同一文档，以及取消实际在途导航。
应用退出先永久关闭 Agent runtime 构建入口，等待已入场构建和现有 runtime 的退出，再关闭 Browser 和数据库；
退出失败保留原实例及资源供重试，不以发送 kill 请求或隐藏页面当作退出证明。

网站权限已接入会话内的用户提示和原生 deferral，仅允许 UserReady 的可见活动标签做一次性决策，Agent 不能授权。
权限超时与隐藏/恢复后的拒绝已通过真实验证。自动取消的请求在当前文档内保持拒绝，刷新后可重新申请，拒绝记录不写入 Profile。
其他设备权限与真实主应用的完整视觉/交互验收仍待完成。

## 文件上传边界（实施中）

Agent 工作台可以选择 `browser.upload`。Windows 原生 Browser Tool 的 `upload` 操作接受当前工作区内的相对文件路径，
针对 fresh observe 得到的可见标准 HTML 文件输入控件，或在当前页面触发 HTML 文件选择的可见按钮执行；
按钮会收到原生点击，动态/隐藏 input 仅通过这次原生 chooser 事件定位。`browser.act` 本身不授予本地文件上传权限。
文件从已授权工作区的目录句柄逐层 no-follow 打开，拒绝链接、junction、目录、特殊设备路径和越界路径。
宿主生成保留文件名、内容与修改时间的上传副本，单次最多 16 个文件 / 64 MiB；Runtime 保留量最多 128 个文件 / 256 MiB，
副本在原生关闭证明后清理，而不是 Tool 返回或输入框清空时删除。网页可以正常消费、保留或自行重置 File 对象。

文件选择使用 WebView2 的 `DOM.setFileInputFiles`，标注 `browser_protocol`，不合成 input/change 事件。
空列表明确拒绝：[Chromium 的路径设置实现](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/core/html/forms/file_input_type.cc)会忽略空列表。
清空通过网页自身的清除/重置控件执行普通真实点击。Agent 运行时拦截新 HTML chooser，既有 iframe 路由也同步策略；
chooser 数据不含文件授权，提交前仍校验所属 Session、Frame、文档及请求有效性。取消或重复请求不获得新文件。
新 OOPIF 在 native auto-attach 时短暂停在启动边界，事件 worker 递归建立所属路由、应用文件选择策略后立即恢复，
不等 Agent 下一次观察。策略失败不无保护地恢复执行；路由保持不可用，交由原生关闭/恢复处理。
Agent 入场同步更新已有和随后创建的 iframe 策略；用户空闲态保留用户策略。真实用例已验证新 OOPIF 在第一次
观察之前的原生点击仍能产生被拦截的 chooser；配置完成屏障和失败失效也有独立测试。
同进程/OOPIF 的标准与动态临时输入框已有真实文件数据验证；父页面按钮可以委托已观察的 iframe，iframe 导航后必须
重新观察。文件选择目标仍须匹配原生 Session 和该文档的隔离执行上下文，不接受未经观察的替代文档。
目录上传、File System Access API、其余文件矩阵及非正常退出后的
副本回收仍待继续，不能把当前路径当作完整文件交互已交付。

Windows 用户文件选择器已有独立辅助进程基础：同一桌面可执行文件的私有子命令在应用初始化前分流，
使用原生 IFileOpenDialog；取消等待该选择器专属进程树退出，不依赖 `Close` 的成功返回值。
辅助进程仅继承 Windows Shell 必要环境变量，管道协议有长度、路径与单选/多选校验，不启动后端或读取应用数据。
`--picker-only` 验证原生显示、隔离取消、最后一个调用方退出及提前取消；内部交互验收
`--picker-selection-only` 已用真实 Windows 窗口验证单选、多选、中文/空格文件名和用户取消。
这些测试不新增产品界面。UserReady HTML chooser 已接入应用选择器，目标来自原生事件和所属 Frame 路由，
文件仅由该选择器返回；回填使用原文档的隔离对象，并在原生 UI 提交前再次校验运行锁、可见性与取消状态。
`--user-files-only` 已真实验证隐藏、主文档导航、OOPIF 文档替换和关闭时取消，以及 Agent 入场等待选择器退出。
无关 iframe 的导航不取消当前选择。临时 Profile 在全部 controller 关闭后，最多等待 2 秒处理 Windows 文件占用释放；
仍失败则保留清理权，不吞掉错误。正常选择后的网页数据回填已接线，但 `--user-file-selection-only` 的交互式验收
仍待用户确认本机临时文件选择，未作为通过证据；目录和完整取消事件语义等仍待完善。

隔离选择器现以显式 Open/Save 模式区分 IFileOpenDialog 与 IFileSaveDialog，内部协议为 v2，不保留旧 multiple-only
请求别名。Save 模式验证建议文件名、保持系统覆盖确认，并复用同一进程退出证明；原生窗口取消与中文文件名保存选择
已经通过本机临时目录验收。UserReady 的同一 WebView 现已接 DownloadStarting deferral 与原生下载操作：只有可见、
未关闭且 Agent 未运行时才创建保存请求，选择后在原生 UI 提交处复查文档/运行状态。每个 Tab 最多四个在途请求，
一次只打开一个保存窗口；隐藏、导航和 Agent 入场取消未确认选择，已授权传输可继续，关闭 Tab 则取消并等待原生终态。
默认下载浮层被隐藏，避免形成运行锁外的可交互窗口；清理失败保留重试权，不把 Cancel 调用成功当作完成。
真实 HTTP 附件字节、中文路径、取消矩阵及进行中下载跨 Agent 入场后关闭已有本机证据。Agent 自动下载/沙箱发布、
用户下载状态菜单与完整网络失败/重定向/权限矩阵仍未完成；Agent 工作期间发起的新下载当前仍拒绝。

网页 `accept` 已连接到 Windows 原生文件筛选：支持扩展名、已知 MIME 与 image/audio/video 通配类型，离线展开为
经过校验的扩展名；去重并限制长度，不把网页字符串直接拼入 Shell 通配表达式。原生窗口默认显示匹配项，并保留
`*.*`（所有文件）；这是选择提示，不是文件授权或服务端内容校验。`--user-file-filter-only` 已通过 computer-use
实际观察 TXT/CSV 默认可见、PNG 默认隐藏，切换所有文件后 PNG 出现，随后取消且未向网页交付文件。

## 独立的本地网页搜索

共用的 `headless_page` 引擎已支持受限检索和匿名 HTML 渲染快照。检索继续使用精确搜索引擎来源及固定公网 DNS；
RenderContent 使用系统 DNS 的严格公网校验与连接 pinning，仅代理 GET，保留浏览器计算的 CORS/Referer，并等待
在途网络请求和有界 DOM quiet。输出最多 256 KiB UTF-8 HTML，附带截断标记，返回前仍需证明进程/Profile 清理。
NomiCore 的 Knowledge 已通过 typed port 接入 Kernel 非 Agent operation 和独立 HeadlessRenderRuntime；
只有安装版本验证成功才装配该 Provider，缺少运行时时仍明确 unavailable，不回退普通 HTTP。
本机公网渲染测试因系统 DNS 把 example.com 映射到保留的 Fake-IP 而被阻断，没有放行该网段；本地检索公网回归通过。

`nomi_local_websearch@1.0.0` 是 Agent 工作台“网页”分类中的独立可选能力。它不映射为 `web.search` / `web_search`，不要求模型原生搜索能力，也不隐含 Browser 自动化权限。

其 Headless owner 使用匿名临时 context、限定 origin 的请求转发与 exact process/profile 清理，不读取会话标签或登录态。Runtime/adapter 身份冻结在能力元数据中，会话构建和请求执行时再次校验。

Windows 默认桌面启动已接入 Chrome 120+ 的安装版本发现和文件指纹装配，找不到合格安装时保持不可用。
发现过程只读取已知安装目录的 PE 版本信息和文件，不运行 Chrome、不发送搜索请求、不读取旧浏览器偏好。
运行版本在真正检索时、创建检索页面之前再次核对；版本变化会拒绝旧绑定。真实公网检索与目录接口已验证。
`local_websearch_agent` 集成测试通过正式 DesktopServer、工作台配置 API 与普通 Chat 模型协议完成会话检索：模型
没有原生搜索 trait，仍取得精确的 `nomi_local_websearch` 工具及真实公开来源/引用标识。该测试使用确定性模型端点，
未供应内嵌 Workspace 或系统浏览器连接；完整 GUI、真实模型自主检索和引用渲染验收仍待完成。

本地搜索结果在现有聊天工具记录中显示查询、来源标题/域名/摘要及明确的错误状态，不是独立搜索页面。
来源文字按纯文本处理；只有用户点击有效 HTTP(S) 来源时才调用既有系统浏览器打开入口。检索执行本身不打开
交互标签。该消息组件已通过实际 React 路径的交互测试，原生主程序视觉验收仍待完成。
隔离检索仅把固定引擎域名交给 Google Public DNS（HTTPS）解析，搜索词交给 Bing；不通过公开 DNS 解析任意用户网址。
解析结果仍必须全部为公开地址，并固定到实际连接；没有为 Fake-IP 添加私网豁免。其他 HTTP 消费者保持原有系统 DNS 策略。

Knowledge 在装配时冻结 exact Browser Provider/source，逐次调用使用当前 registry generation/digest 重新准入；
Provider 变化不会静默重选。调用方是 Knowledge service principal，不伪造 AgentSession、Snapshot 或 Workspace 资源绑定。
输入仅接受 URL，输出固定为 final_url/html/html_truncated；HTML→Markdown 仍由 Knowledge 负责。
Headless 渲染最多同时运行 2 个任务，总在途/排队上限 16，排队最多 30 秒，以支持既有的四来源并发抓取。
取消保留任务所有权直至 join；清理失败或执行 panic 在释放并发槽前关闭准入、取消其余任务，保留失败记录供 shutdown 报错。
启动恢复任务在 Kernel/Provider 装配完成后开始。测试引擎已验证实际 Knowledge 快照落库；真实 Chrome 已验证私网来源被阻断且
不回退 HTTP，但公网渲染的正向验收仍受本机 Fake-IP DNS 阻断。Fresh-v4 AgentSession 没有 ConversationId，不能伪造
BrowserWorkspaceKey；该宿主不声明 Browser owner，相关 Role binding 不加载或持久化，调用明确返回 RoleProviderNotBound。

## 系统浏览器运行期间的绘制状态

系统浏览器与内嵌 Workspace 独立。已授权的现有 Chrome 页即便被 `Page.bringToFront` 激活，后台状态仍可能令
滚轮命令等待绘制 ACK 超时。驱动从本轮首次非观察动作到 run settle 临时保持该页绘制，收尾时移除自身 debugger
override；不修改启动参数、Profile、浏览器设置或 OS 窗口，也不关闭用户的浏览器/标签。
不能在单次 wheel ACK 后马上释放：默认滚动可能尚未执行。启用或释放无法确认时，关闭 NomiFun 自己的连接。
真实 Chrome 全组 54 项及主应用两项、每项 25 次模型调用通过；不等同于个人登录网站和完整 GUI 的验收。

## 退役边界

Nomi 工厂现在使用明确的 BrowserRuntimeTarget，而不是从 `Workspace == None` 推断 Headless。宿主始终根据持久
Conversation 的 owner、source、cron 与 execution 归属分类，即使未安装原生 Surface 也运行分类。普通交互会话没有
原生宿主时，未选择 Browser 的聊天可以继续；已选择 Browser 的构建明确失败，不创建外部 Chromium。频道、定时与
执行步骤属于后台消费者；旧 Headless Provider 已移除，后台 Browser 自动化当前明确不可用，不能回退旧 Hub 或私人引擎。
已单独接入的新本地检索与 Knowledge 渲染不受该退役影响；其余后台自动化仍需新的 v2 owner。

前端 v1 管理、设置、DTO 和全局管理/登录 API 已移除。旧 Headless Hub 的应用构造、租约签发 Provider、恢复入口、
资源采样/生命周期循环、库存事件与专属测试均已物理删除。边界检查禁止所有应用入口重建 Hub，而不是允许唯一旧构造点。
Nomi manager 的 Lane binding、租约终止分支及其测试已物理删除，工厂 binding 模块与旧配置字段不再存在。
原生 Workspace 的 settle→terminal→finish 顺序和独立的 SSH/MCP/进程清理仍保留。应用与 Agent crate 已移除旧
nomi-browser 直接依赖及 nomi-agent/browser-use 特性传播；新 native/search/render 路径继续使用 v2 平台与引擎。
旧 nomi-browser crate、低层 bootstrap 的 Browser 注册/截图定位适配、旧 feature/提示词参数，以及 BrowserConfig
及其合并规则、旧数据目录 API 已物理删除。平台旧 Hub/Lane、租约、调度/资源和身份模型也已删除；当前平台保留
原生 Workspace、run guard、上传下载边界及独立系统浏览器合同。

无生产消费者的旧 `CdpBackend`/`CdpHostRuntime`、Host/Lane target 路由、`BrowserEngine` trait、旧动作/evaluate/
注入管理/身份快照执行器和专属测试、夹具已物理删除。共享键盘/真实 CDP 输入、可执行下载检测、vendor 语义脚本、
连接协议与精确进程/Profile 清理原语仍保留。新原生、附着系统浏览器及隔离检索/渲染均不依赖被删除的执行器。
边界扫描拒绝旧文件恢复和旧执行器类型搬到其他引擎文件。原旧执行器的延迟对象回收 dispatcher/worker 及
跨 Host task 事件订阅层已删除；当前连接和单个订阅者的事件数量/字节上限保留，事件收到后直到最终 Drop 才释放
计数。新 native/attached 文档 owner 仍显式等待自身对象组清理。旧 task-session/Lane 归属及分配等待已删除。
SessionRegistry 仅维护连接内的有界身份与命令/事件路由，业务授权留在 Workspace/Run 和系统浏览器宿主。
隔离 page owner 的 attach 放行只接受 read loop 已登记、仍匹配的身份，不重新登记迟到事件或关闭目标。
裸响应登记不能复活近期关闭的会话，完整新 attach 才可建立新身份；关闭事件不会覆盖既有崩溃分类。

应用不再创建或恢复旧 Hub 使用的 `browser-v2/headless/profiles/`，也不扫描 `browser-data` / `platform-profiles`。
旧目录保留原样，不做搬迁或删除；新匿名检索和渲染各自保持明确的进程/Profile 清理权。
managed adapter 的 vault 写回与策略装饰钩子、应用到浏览器的持久登录密钥转发，以及无读取者的 Agent 工厂密钥字段
已物理移除。低层 BrowserTool 的保存协调器、启动导入、导航后写回和 bootstrap 密钥入口也已删除，浏览器引擎不再导出
共享 vault 模块或读写/路径 API。旧磁盘往返和独立引擎内存身份导入测试也已退役；系统浏览器直接附着既有会话，
不依赖这些导入功能。新原生交互与匿名后台链路不从旧共享身份仓库导入，运行时没有旧 Profile reader 或迁移路径。

Fresh-v4 的失效 Browser RoleRuntime、独立 Hub 构造、旧 Profile/Cookie 读取与续租代码已删除。该宿主在接入通过验收的 v2 owner 前明确保持 Browser host ports 未配置；这不影响 Nomi 会话的原生 Workspace 接入，也不代表后台 Headless 替代已完成。

无生产调用的知识库 Hub 渲染适配器 `BrowserFetcher` 及其专属租约/队列测试已物理删除，不再从 Agent crate 导出。
HTML→Markdown 转换由现有 Knowledge `rendered_content_to_page` 负责，转换、截断和缺少 Provider 不回退 HTTP 的行为
仍有测试。知识渲染的 `BrowserRenderContentPort` 现由 NomiCore 的 Kernel operation 组合，不再使用 Hub 适配器。
边界检查同时拒绝恢复旧文件和在其他 Agent 模块重新导出旧类型。

边界检查会拒绝恢复已删除的 UI 路径、路由和配置声明。后续开发与验收应面向 v2，不增加旧路径别名。
