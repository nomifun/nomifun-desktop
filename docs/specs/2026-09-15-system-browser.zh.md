# 系统浏览器：连接用户当前已登录会话

> **能力身份已被 2026-09-16 统一能力重构方案取代。** 已登录 Chrome 仍可作为受控 Provider，
> 但不再是与内嵌 Browser 并列的独立 Agent Capability；两者共享 `browser` Module 的 Action
> 授权，由 Resource Binding 选择 Provider。本文保留连接、授权、隔离和平台实现证据。
> 新目标见 [Agent 能力模型重构 §7](2026-09-16-agent-capability-product-redesign.zh.md)“Browser 产品模型纠正”。

状态：WINDOWS IMPLEMENTED AND TEMPORARY-CHROME CONFORMANCE VERIFIED / PERSONAL DATA NOT ACCESSED。

这是 Browser Workspace v2 的追加范围，不替换内嵌浏览器，也不恢复旧 Browser 管理页、登录保险库或 Profile 迁移。

## 1. 硬要求与实现边界

- 必须操作用户正在使用、已登录的真实浏览器实例及其真实标签页；网站请求自然使用该浏览器现有登录状态。
- 不创建另一个 Profile，不导入/复制 Cookie、密码、历史或用户数据目录，不要求为了连接而关闭、重启浏览器。
- 在 Agent 工作台独立选择“系统浏览器”。不得把它做成内嵌 Browser 的来源切换或运行时自动 fallback。
- 技术命名暂定 `nomi_system_browser`，与 `nomi_local_websearch`、内嵌 `browser.*` 分离。尤其不能把已有内部
  Role ID `system.browser_use` 的字面名称误认为已经实现了系统浏览器连接。
- “在系统浏览器打开”只是 URL 交接，不是这项能力；受管临时 Profile 也只能用于自有 conformance，
  不能成为生产 fallback。

## 2. 优先连接路线

Chrome 144+ 已提供浏览器内的授权连接机制：用户在正在运行的 Chrome 的 `chrome://inspect/#remote-debugging`
开启连接，并在 Chrome 的原生提示中批准请求。官方 Chrome DevTools MCP 的 `--autoConnect` 使用此机制，
不需要新建 Profile。它与旧命令行 `--remote-debugging-port` 路线不同；Chrome 136+ 对默认 Profile 的命令行
调试开关有限制，不能据此要求用户迁移到测试 Profile。

来源：[官方运行中会话连接说明](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/docs/advanced-usage.md#connecting-to-a-running-chrome-instance)、
[Chrome 调试开关变更](https://developer.chrome.com/blog/remote-debugging-port)。

实施顺序：

1. 验证 Chrome 官方授权连接路径后，复用现有 Rust WebSocket/CDP 传输实现独立 attach-only 连接器；
   不部署 Node/MCP 工具服务，不先建设自有浏览器扩展平台。选型证据见 §2.1。
2. 接口必须是 attach-only：连接失败即 unavailable，绝不转入 launch、isolated、新 user-data-dir 或自动重启路径。
3. 不把上游 MCP Tools 暴露给模型。连接、目标授权、真实输入与输出投影由 NomiFun 的独立能力宿主承担。
4. 不增加 MCP 的 usage statistics、performance CrUX、第三方页面工具、自动更新或任意脚本执行通道；
   不用 `@latest` 作为生产供应，不上传用户页面 URL 做性能查询。Rust 依赖仍由现有 Cargo.lock 固定。
5. Edge、多 Profile 精确选择等缺口先实际验证；不能因为同属 Chromium 就宣称支持。只有官方路径不满足明确目标时，
   才评估 `chrome.debugger` + Native Messaging 扩展方案，不同时维护两套未验证的默认连接器。

官方连接实现区分 connected 与 launched，前者退出时 disconnect 而不是 close。这是可复用的设计，最终仍须
在所选固定版本上验收，不能只依赖 main 分支代码。
[连接/退出实现](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/src/browser.ts)、
[隐私与功能参数](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/main/docs/configuration.md)。

### 2.1 固定版核验与简化后的实现选择（2026-09-15）

核验官方 `chrome-devtools-mcp-v1.9.0` 与其 Puppeteer 25.10.0 来源，而非仅依据 main 文档：

- stock MCP 的 attach 失败不转入 launch，connected 退出使用 disconnect；但后续 tool 调用会在浏览器断线后
  再次连接。只禁止 MCP 子进程 respawn 并不能阻止浏览器内部重连。
- 多个页面工具会附加全标签标题/URL，默认 context 会选择第一页；pageId 路由不是标签授权边界。
- `fill`、select/option 的部分路径使用 DOM 赋值或合成事件，不能原样满足真实输入要求。

来源：[固定版连接代码](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/chrome-devtools-mcp-v1.9.0/src/browser.ts)、
[按调用获取 Context](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/chrome-devtools-mcp-v1.9.0/src/index.ts)、
[响应中的标签列表](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/chrome-devtools-mcp-v1.9.0/src/McpResponse.ts)、
[输入实现](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/chrome-devtools-mcp-v1.9.0/src/tools/input.ts)。

Puppeteer 的 channel 连接实际读取默认 Chrome `DevToolsActivePort`，然后进行 WebSocket 连接。可直接复用
Rust 的相同连接基础，无须为接入再维护 MCP 响应过滤、Node 供应和第二套动作实现。
[固定版 discovery/连接实现](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.10.0/packages/puppeteer-core/src/common/BrowserConnector.ts)、
[Windows 默认路径](https://github.com/puppeteer/puppeteer/blob/puppeteer-v25.10.0/packages/browsers/src/browser-data/chrome.ts)。

当前 `nomi-browser-engine::attached_browser` 已落地：Windows 默认 Chrome discovery 只读且限长；严格构造 loopback
endpoint；仅连接并读取浏览器版本，不创建/附着页面，不启动进程，不读登录数据库，不接入旧 CdpBackend/Launched。
断线不重连；disconnect 取出并释放最终 Connection，即使外部端不回应 WebSocket Close 也释放本地 socket。
模块没有向调用者暴露 raw CDP handle。第九十九切片已接用户标签选择与应用路由；第一百切片继续接工作台 UI 和主文档 Agent 动作。

9 项本地协议端测试覆盖解析、缺失/错误 metadata、不支持版本、正常断开、Drop、断线、初始化取消、无关闭应答和
取消断开后的再次等待。断开由单个清理任务持有连接，调用方取消不会丢失其可等待结果。
这不是 Chrome 原生授权实测，也不是已登录页面验收。本轮 MCP 单连接试验代码已撤除，不遗留第二套实现。

### 2.2 会话宿主与标签授权（2026-09-15）

- `attached_browser/tabs.rs` 枚举仅供本地用户的标签选项，不附着页面。用户得到随机 choice ID，不得到 raw target ID。
  刷新会作废旧 choice；授权时重新核对目标和展示 URL，关闭或导航后的过期选择不会授权到别的页面。
- `GrantedTab` 不能从用户/模型 JSON 反序列化，绑定唯一连接代际；只读取授权目标的元数据，不附带其他标签。
  跨连接授权、原始 target/URL/序号、过期 choice 被拒绝；刷新失败也不会保留旧选择。
- `nomifun-app::system_browser` 是独立的进程内服务，以 user + conversation + incarnation 绑定连接和授权；
  Snapshot 不进行浏览器 I/O。断线投影为 connection_lost 并撤下缓存授权，不自动重新连接。
  授权按同一真实目标去重；宿主/底层各限制最多 32 个授权，连接记录有界，不新增数据库表或迁移。
- Windows DesktopHost 显式提供服务，其余宿主默认无服务。HTTP 路由在会话下独立命名 `system-browser`，
  同时要求 instance owner、local trust 与会话归属。connect/grant/disconnect 使用会话闲置重配置边界；
  不提供 begin-run、解锁、接管或任意协议入口，Agent 运行期间拒绝更改连接与授权。
- HTTP connect/grant 的 caller cancellation 传入重配置前置检查和工作体，不让后台重配置任务吞掉取消。
  连接中的 DELETE 可以先取消 connect 等待，再经闲置边界完成断开。disconnect 先同步阻断底层 I/O，
  再等待在途操作和同一个物理清理结果；失败保留准确对象供下次显式重试。
- 会话删除前和应用退出均结算连接；断开不关闭用户标签，不清理用户 Profile。删除前清理未获确认则保留会话。

已验证 17 项连接/标签协议端测试、9 项宿主生命周期测试，以及使用可控替身的完整 HTTP 权限/取消/删除链路。
这是第九十九切片的宿主证据，当时尚未接工作台和 Agent；后续进展见 §2.3。不能把单独的宿主 grant 当作完整 run 权限证明。

### 2.3 独立能力、会话 UI 与主文档 Agent 输入（2026-09-15）

- 注册 `nomifun.system-browser` / `nomi_system_browser@1.0.0` / `nomi_system_browser.invoke`，共用严格输入 schema。
  明确约束当前 Windows x64 Desktop target；macOS/Linux 不通过静态平台声明被假称支持。通用 Wave1 action host
  不能取得该会话 run；实际工具只在能力显式选中、冻结 annotation/贡献锁/运行时 digest 匹配时注册。
- Agent factory 注入独立 Host/Workspace/Turn，不创建浏览器。workspace 和 begin-run 均查询持久会话 owner、source、
  cron 和 delegated projection。每 turn 冻结连接 incarnation 与授权集合；跨连接对同一真实浏览器 Tab 使用稳定
  不透明 key 互斥。在途工作/释放均保留唯一 job，Stop 不丢弃原子输入；settle 在 terminal 前，finish 在 terminal 后释放 claim。
- 当前七个操作为 tabs、observe、navigate、click、type、press、scroll。只附着授权 Tab；主文档通过隔离世界观察，
  不开放模型脚本、协议参数、Profile 路径、原始 target 或浏览器窗口快捷键。frame/loader/observation/ref 共同拒绝过期输入，
  主文档 URL 在协议和语义世界内都检查，不能在授权后转到 chrome:/file: 页面继续读取。
- 点击/键盘/文本/滚动均通过 Chrome Input 协议。mouse move 后重新核对原元素与命中；Ctrl+A 后再核对原文档/焦点。
  未确认的 mouse/key release 保留到清理重试，当前文档仍存在时不能把失败的指针清理算作完成。
  ARIA 输入值和敏感文本沿用既有脱敏原语，密码 ref 额外遮蔽；工具 Tab URL 使用安全元数据投影。
- 会话标题栏只在 Snapshot 含该独立能力时显示入口。UI 为小型连接/授权弹层，不增加测试面板、管理中心或模式切换。
  运行期间不更改授权；只允许取消本组件自身发起的 pending connect，先 GET 精确 incarnation 再 DELETE 一次。
  已确认与真实 BrowserWorkspacePanel 的遮挡机制配合，不添加帧流或第二个浏览器页面。
- 真实输入测试启动的是**独立、可见、临时 Profile 的 Chrome 测试实例**，仅供验证，不是生产连接 fallback。
  Chrome 152.0.7977.76 中验证可信 click/中文 input/key/wheel、悬停替换拒绝、快捷键转移焦点拒绝、密码不输出、
  导航后旧观察失效，以及不同连接的同 Tab contention key 一致；disconnect 后原浏览器进程仍在，最终由 fixture owner 关闭。

以上是第一百切片的分层证据，不是个人登录页面的原生许可实测；第一百零一切片补主应用后端闭环，
第一百零三切片补已授权页的同源嵌套与 OOPIF 观察/基础输入。文件、拖放与更广浏览器自动化矩阵
不属于当前 Windows 首发合同；未经用户授权的个人 Chrome 仍不得读取。

### 2.4 主应用后端到真实浏览器的闭环（2026-09-15）

`nomifun-app/tests/system_browser_agent.rs` 使用真实 DesktopServer、产品 HTTP API、正式 Preset/Catalog/Factory、
Nomi 模型协议与 SystemBrowser Tool/Run/Driver。只通过显式 `browser-conformance` 构建特性替换连接发现路径，
指向由测试本身独立持有的可见临时 Chrome；默认 Desktop normal/build feature tree 不包含该特性，不新增产品 endpoint/Profile 参数。

测试通过产品 API 创建脚本模型、从 chat.minimal 创建 Agent、仅启用 nomi_system_browser、创建会话、连接、枚举和授权。
18 次模型调用验证两次真实观察/输入/提交，夹有一次 Stop 后的迟到 click，以及应用退出时等待中的 click。
测试不读取其他个人页面；另建一个未授权 sentinel Tab，先确认其存在于用户 chooser，再确认它不在授权/模型 Tool Tabs/模型请求中。

测试站点在 NomiFun attach 前设置 HttpOnly 会话 Cookie，ready/witness 端点要求该 Cookie；模型从未收到其值。
每轮检查新增 trusted input/click 事件和同一 document nonce，不借用前一轮的事件作为证明。退出后直接只读检查 fixture
DOM 的提交计数、nonce 和输入值，确认没有第三次提交；原浏览器进程和标签仍在。最后由 fixture owner 清理浏览器，
应用/服务器退出并显式删除临时根目录后才输出 PASS。

Windows 验证命令（显式提供测试浏览器可执行文件，不提供个人 Profile）：

```powershell
$env:NOMIFUN_CHROME_BINARY = 'C:/Program Files/Google/Chrome/Application/chrome.exe'
cargo test -p nomifun-app --features browser-conformance --test system_browser_agent -- --ignored --nocapture --test-threads=1
```

这是脚本模型驱动的主应用**后端**闭环，不是个人 Chrome 原生授权提示、真实用户登录网站或完整 Tauri GUI 验收。
上述第一百零一切片只在模型尚未交付 click 的阶段停止；下一节补充真实在途鼠标输入证据。

### 2.5 在途鼠标输入的停止与退出（2026-09-15）

- 修复生产驱动的取消窗口：最终 LOCATE、focus 查询或鼠标 move 等待期间收到 Stop 后，紧邻发送效果前重新检查
  cancellation，不再发出新的 mousePressed、insertText 或 wheel；已经发出的 mousePressed 仍无条件配对 mouseReleased。
  4 项本地协议测试覆盖这三类禁止新输入边界以及已按下后的精确释放；全部 attach 自动测试 24 项通过。
- `system_browser_main_application_atomic_input_settles` 继续使用正式主应用后端与生产 connection。测试专用透明
  WebSocket relay 将指令完整转发给可见 Chrome，只扣住它实际返回的 mousePressed / mouseReleased ACK，不伪造执行结果。
  另一个纯测试 wrapper 只观察真实 invoke 收到的 cancellation token，所有连接/输入/清理仍完整委托正式实现。
- 在 Chrome 已产生 trusted pointerdown 后，经产品 Stop API／app shutdown 发起取消；必须确认上述 token 确实取消，
  才放行 down ACK，排除“请求尚未到达”的假阳性。扣住 up ACK 时，turn 仍为 running、shutdown 仍未完成；放行后要求
  精确 down → down ACK → up → up ACK → detach 顺序，且页面有对应 trusted pointerup/click，没有重复输入。
- fixture 点击的是输入框，已开始的 click 在 Stop 后完成属于原子收尾，不被误判成应当撤销的动作。后续仍验证同 nonce
  页面新 turn、既有测试登录 Cookie、两次明确提交、浏览器和 Tab 存活，最终回收测试自有资源；每个场景 18 次模型调用。
- 两项主应用真实 Chrome 场景单独通过，最终同进程串行整组 2 项通过；引擎真实输入回归也通过。首次整组执行时第二项
  在 Preset revision 保存遇到 SQLite database locked，后续未重放该 mutation，而是销毁失败 fixture 后全新运行；
  保留该稳定性差异，不能声称其根因已经修复。

此证据仍不是个人 Chrome 的授权/登录页面或完整 Tauri GUI 验收，也不覆盖全部键盘原子阶段、文件、对话框、
拖放和强制断线矩阵。没有增加产品面板、运行中接管、数据库迁移或生产连接替代路径。

### 2.6 授权标签页内 iframe 的观察与真实输入（2026-09-15）

- 七个 Tool 操作保持不变，不新增 frame 选择器或产品模式。Observe 返回主页面与其子文档的元素，子文档 ref 在内部
  加命名空间，仍绑定同一次 observation；Click/Type/Press/Scroll 继续通过根页面的真实 Input 管线执行。
- `FrameRoutes` 只在已授权 page session 上开启 iframe-only auto-attach，再递归处理该父链的 iframe。既不在浏览器根
  session 全局 attach，也不把 worker、popup、其他已授权或未授权页面当作子 frame。不暂停用户页面，不保留后台 worker。
  session 来源与 frame parentId/tree 均核验，再由语义层通过 DOM.getFrameOwner / resolveNode 核对实际父元素。
- 观察具有 frame/loader/session 与父链边界；导航、替换、遮挡或焦点不符会拒绝旧动作。支持 open Shadow DOM 内
  iframe 的深层焦点检查。读取受限于当前页面及子文档，privileged/data/file 文档不观察，未覆盖子文档计入结果。
- 将既有原生内嵌浏览器 ContentQuad 与父元素命中检查移入唯一共享 `nomi-browser-engine::frame_geometry`，不复制
  CSS 变换算法；同进程嵌套与跨进程 frame 的局部坐标逐层映射到真实根视口，移动后重新定位再发出按下/滚轮。
- 修复资源边界：子文档 core 与父 iframe owner 使用可释放的独立 object groups；root Evaluate 的错误/缺 objectId
  仍保留准确 group，重试和 settle 都先释放它。语义专用 InjectedScript 子类禁用两个未使用的永久拦截/监听器 hook，
  不改通用引擎 injection；固定 world 名避免每次观察分配新 context。
- 真实 Chrome 验证发现关闭 auto-attach 时不允许 target filter，已改为仅开启时附带 iframe filter，并加入协议断言。
  关闭按最深子源到根源执行，失败保留清理义务，只有准确 detach/dead-session 证据才能收敛，不吞掉任意失败。
- 38 项 attach 测试（含两项显式启用的真实 Chrome 场景）全部通过。新增真实 fixture 确认跨站目标确实为 OOPIF，
  验证同源父/嵌套与跨站 iframe 的中文 Type、Press、Click、Scroll、遮挡拒绝、子页导航后旧 ref 失效，以及退出后
  原 Tab/其他未授权 Tab/浏览器仍存活。重复观察的 4 个 context 与 Window listener 数量不增长；独立 probe 的 WeakRef
  在 object group 释放与 GC 后消失，证明语义 core 不再被这些永久监听器闭包保留。
- 共享改动的回归包括真实 WebView2 `--frame-input-only`（输入锁、click/type/key/wheel、select、同源/OOPIF/嵌套、
  变换/透视/缩放与原生 Profile 清理）、78 项原生宿主单测、3 项共享几何单测，以及两项主应用真实 Chrome 场景。
  原生 smoke 的模块接线遗漏已补齐，测试依赖只在 dev-dependencies 中声明；未增加生产系统浏览器实现或兼容分支。

这些 iframe 是实际网站内部的文档，不是产品通过 iframe 模拟浏览器。证据来自自有可见临时 Chrome，未连接个人
浏览器或读取个人登录页面；系统浏览器 iframe 的完整主应用模型链路、动态/极端变换、文件/对话框/拖放与发行矩阵仍待验收。

### 2.7 原生脚本对话框与保留的浏览器操作（2026-09-15）

- 在独立 `nomi_system_browser` Tool 内增加 `dialog` 操作，使用实际返回的 `script_dialog.dialog_id`、accept 和可选
  prompt_text；不提供 raw session、脚本、猜测选择器或第二个产品入口。旧七个操作身份保持不变，不做旧接口 alias。
- 真实 Chrome 探针确认 confirm 会阻塞原 Input ACK。因此点击不被取消或重放，而是保留原操作与输入清理责任，先返回
  awaiting dialog；回复经独立控制路径解除该 modal 后继续等待原 ACK，不能把回复排到原输入互斥锁后制造死锁。
- Opening/Closed 使用同一有界可靠事件队列，保持协议顺序；只接受已授权根 Page 的事件，OOPIF 的真实根路由已验收。
  消息限长并标记不可信，不输出 URL、raw frame/session、defaultPrompt 或 Closed.userInput。
- 只在该操作句柄暂停 modal 期间的 CDP 响应预算，恢复继续消耗剩余预算，普通连接 clone 不受影响，写入预算仍有界。
  不靠延长所有操作超时或自动接受对话框处理阻塞。每个 dialog_id 的答复发送前记录唯一任务，取消等待/旧 ID 重试只
  等同一个回执，不再次发送 Page.handleJavaScriptDialog；旧回复完成也不能清除下一只 modal。
- Stop 对等待本次操作的未答复 modal 只拒绝，不接受 confirm/prompt/beforeunload；已提交的答案不能被 Stop 改写，
  必须结算原答复。这里的 owned 仅表示该操作期间观察到 modal，不声称具备浏览器提供的绝对因果证明。
- Chrome 不一定给新连接补发已有 modal 的 Opening，且已有 modal 可能阻塞 attach/enable/读取。取消准备阶段或无法
  正常收束的读取时只撤销 NomiFun 的连接，不盲发 dismiss 探测，不关闭用户页面或浏览器；已存在的 modal 保留给用户。
  用户处理后需要显式重新连接，不自动重连或启动另一个 Profile。正常短读取有短暂的协作取消窗口，不默认强制断开。
- 准备阶段 caller-drop 会阻断未知在途附着；正式操作移入 retained task。显式 disconnect 除关闭协议连接，还保留并
  等待原 Pending 与 dialog shutdown receipts，不把 operation gate 空闲误当成所有答复/观察任务都已完成。
- 已有真实 Chrome 证据覆盖 alert、confirm、中文 prompt、连续两只 modal、旧 ID 无重放、OOPIF prompt、Stop 拒绝
  confirm、beforeunload 拒绝后保留 nonce、连接前原 modal 不被改变，以及测试自有 Profile/进程最终清理。不是个人浏览器验收。
- 主应用两项真实 Chrome 链路已加入模型 Dialog(false) 与 Stop 时迟到 Dialog(true) 抑制，每项实际 25 次调用通过。
  attach 整组最近 51 项通过；但此前一次整组仍有 Dialog 点击超时（50/51），根因未关闭，不能宣称稳定性已全面验收。

协议限制仍须如实保留：Chrome 的 handleJavaScriptDialog 没有 dialog-ID 参数，不能对用户或另一调试客户端在最后
发送瞬间关闭旧 modal、再打开新 modal 做浏览器端原子 compare-and-handle。NomiFun 保证自身队列/代际/回执不重放，
不宣称能物理锁定外部 Chrome 或消除其他客户端并发。权限弹窗、OS 文件选择器不属于这个 script-dialog 操作。

### 2.8 后台页面绘制与滚动修复（2026-09-16）

真实回归复现了隐藏页面的 wheel ACK 超时：`visibilityState=hidden`，脚本仍可读，`Page.bringToFront` 已返回，
但没有 wheel 事件或滚动。修复仅在本连接已授权页的首个非观察动作开始保持绘制，并由既有 turn settle 释放；
不增加启动参数、全局鼠标控制、用户 Profile 导入或产品状态。单次输入 ACK 后立即释放太早，已通过失败用例确认，
因此 owner 保持到本轮收尾，而不是用延时或 DOM scroll 冒充默认动作。

`Rendering` 使用当前 page session 的 `Emulation.setFocusEmulationEnabled`；机制参考
[Chromium 的临时 capture 生命周期](https://raw.githubusercontent.com/chromium/chromium/main/content/browser/devtools/protocol/emulation_handler.cc)，
实际支持仍以本机真实 Chrome 验收为准。正常清理等待关闭自身 override 的 ACK；启用/清理不确定或 owner 被丢弃时
只退休自己的协议连接，不关闭用户页面。清理 Connection 不继承模态对话框的暂停时钟，避免 observer 停止后悬挂。
其他外部客户端并发修改同一调试状态仍无跨客户端 CAS 保证，不把此机制声称为锁定用户或整个浏览器。

相关完整引擎组 54/54、主应用两项各 25 次模型调用通过；后者继续覆盖 Stop/原子输入、同页续跑、登录 fixture、
未授权标签隔离和退出保留原浏览器。旧失败证据保留，不据本轮通过宣布个人 Chrome、GUI 或完整发行验收完成。

## 3. 简单产品交互

- Agent 工作台分别显示“会话浏览器”“已登录 Chrome”“Nomi 本地网页搜索”，可独立选择；互不授予权限。
- 首次使用系统浏览器，提供“连接正在运行的 Chrome”的引导；用户只处理 Chrome 原生授权，不填写 Profile 路径或搬数据。
- 连接后，由用户明确选择允许本会话操作的标签页。默认不把全部窗口/标签的标题、URL 和内容注入模型。
- 显示精简的连接/操作状态及断开入口。页面继续留在系统浏览器原窗口，不伪装为会话内的第二个渲染实例。
- Stop 沿用会话原有按钮；不添加“接管”“交还控制”、测试模式、问题面板、步骤面板或第二套浏览器管理中心。

必须诚实展示的权限边界：官方 auto-connect 在浏览器侧可访问选定 Profile 的全部窗口，多活跃 Profile 时由
Chrome 决定默认 Profile。NomiFun 的“指定标签页”是其自身额外执行边界，不是 Chrome 原生提示授予的逐 Tab 权限。
若实际连接的 Profile 不符合用户选择，应断开并提示，不能静默切到另一个已登录账号。

## 4. 生命周期与隔离

独立宿主只拥有连接、已授权目标与在途操作，不拥有用户浏览器进程、窗口、Profile 或登录数据。

- 不复用 managed-child 的 kill-on-drop、孤儿进程回收、Profile scrub/delete 或退出时关闭所有标签的逻辑。
- 一个授权标签同一时刻只受一个 Agent run 操作；权限绑定 host/user/conversation/连接 incarnation/真实 Tab 身份。
- 不接受模型传入浏览器路径、任意 CDP endpoint、原始 target/session ID 或任意协议方法。
- 观察与动作可以复用传输无关语义层，但必须使用真实浏览器输入路径；不把 DOM mutation 伪装为用户操作。
- 停止后拒绝新操作，取消排队动作，确认已提交动作结算并释放按键，再 detach/disconnect；不自动重放写操作。
- 浏览器退出、授权撤销、连接断开、页面被关闭或目标代际失效，都使旧授权失效。重连需要新的绑定，不能沿用旧 ref。
- 断开 NomiFun 后用户浏览器及原有登录仍保持；既有标签不因清理连接而关闭。
- 不读取 Cookie/密码数据库，不建立共享 vault，不向内嵌 Profile 或 `nomi_local_websearch` 复制认证状态。
- 不修改用户浏览器的全局代理、安全开关、站点隔离或其他扩展；不隐藏浏览器的调试/授权提示。

## 5. 输入控制的真实限制

内嵌 WebView 的原生输入锁仍是硬合同。系统浏览器不是 NomiFun 持有的原生控件：授权调试连接不能强制禁用
Chrome 的地址栏、关闭按钮、用户撤销授权或关闭浏览器的能力。因此不能照搬内嵌 HWND 输入门并宣称同样的物理锁定。

不提供运行中“用户接管”流程。对浏览器外部操作或强制断开，只能让 Agent 停止/中断、拒绝后续动作，并重新观察或授权；
不能假装外部操作从未发生，也不能用接管整台桌面、全局鼠标键盘拦截来补足。该差异须作为实现和验收边界明确确认，
不能隐瞒为“系统浏览器已具备与内嵌一致的输入锁”。

## 6. 交付证据与隐私选择

系统浏览器的 Windows 代码与真实临时 Chrome/主应用验收已完成。不为验收主动读取用户个人
Profile/标签/登录页；用户以后在产品内明确连接和授权时，属于正常使用而不是交付前隐私检查。
当前证据：

1. Rust attach-only 连接器只附着用户手动启动的真实 Chrome；未启动第二个浏览器进程/Profile，也未要求重启。
2. 自有临时 Profile 的真实 Chrome 先建立 HttpOnly 登录 fixture，再由 NomiFun attach、观察与真实输入；
   模型不获取 Cookie 值，不复制登录数据。
3. 未授权标签、另一个 Profile、另一个会话和已撤销 run 均不可操作；输出不包含未授权页的内容/元数据。
4. 浏览器原生授权取消/拒绝、多 Profile 选择不符、连接失败时诚实 unavailable，不自动启动替代浏览器。
5. Stop、在途动作、强制断开与后续重新授权验证；不得将不确定的写入结果自动重试。
6. 正常完成、NomiFun 退出或适配器退出不关闭用户浏览器、原有标签，不清理用户 Profile。
7. 三项浏览器相关能力的工作台选择、schema、provider 和资源边界互不泄漏权限；厂商 web_search 保持独立。
8. Windows 已验收；macOS 按主 ADR 的移交安排实施，Linux 可后置。个人浏览器和登录页只在用户主动使用
   产品连接/授权后访问。
