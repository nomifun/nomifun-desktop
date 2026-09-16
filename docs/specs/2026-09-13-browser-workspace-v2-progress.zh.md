# Browser Workspace v2 实施记录

目标：完整交付 `2026-09-13-browser-workspace-v2.zh.md`。本文件只记录代码与验证事实，不缩减 ADR 的交付条件。

## 当前状态

- 并发收口与开放网络裁决（2026-09-16）：按用户要求将可独立的网络、跨进程 drag、后端 owner 审计
  并发，文件边界互不交叉，子任务不同时运行 Cargo；主任务统一串行编译/原生验收。
  网络切片曾实现 per-runtime SOCKS、DNS/IP pinning、loopback 端口准入与 WebResourceRequested，
  但用户明确指出这会增加开发/测试复杂度并降低灵活性。该切片已全部撤回：无新模块、无 desktop
  `nomifun-net` 依赖、无 proxy args/请求防火墙/端口集合残留。Conversation WebView 保留系统原生
  DNS/代理/证书、localhost/LAN/WebSocket/HMR。只保留无内嵌凭据的 HTTP(S) 顶层 URL 校验、
  child 无 Tauri capability/local trust/backend credential/filesystem authority，以及 Conversation/run/Provider 权限。
  新增边界扫描禁止内嵌 Browser 恢复 application proxy/IP/端口白名单，同时禁止 DevTools 入口回归。
  URL 定向单测 2/2 通过：公网、localhost、127、LAN IPv4/IPv6 均允许；特权 scheme、内嵌凭据、
  缺 host/超长 URL 拒绝。后台 `nomi_local_websearch`/render 仍保留独立严格公网边界，其 engine 单测通过。
  后端删除 `BrowserRuntimeTarget` 伪中间态；只有显式选中能力且 Snapshot 含 exact v2 Provider 时才
  ensure Conversation Workspace，普通聊天不再隐式创建；后台/执行步骤/无 native host/无 Provider 均 fail closed。
  Fresh-v4 AgentSession 无 ConversationId，不伪造 BrowserWorkspaceKey；Platform 新增宿主 `owned_role_ids`，
  未声明真实 owner 的 Browser binding 不加载/派生/持久化，`browser.render_content` 在 Fresh-v4 明确 RoleProviderNotBound。
  统一验证：desktop bin no-default-features check 通过；`browser_workspace` 3/3；主应用 canonical mount 在
  browser-only 和 browser+computer 两种 feature 各 1/1；Agent Platform 24/24；Browser tool 13/13；
  Browser UI 63/63（344 assertions）；typecheck/i18n/desktop-ui/browser boundary/diff check 通过。Ctrl+L UI 用例首次
  并发运行时在初始地址 effect 尚未稳定前 select，act 随后 flush effect 将 caret 移到末尾，单项独立也复现。
  测试现在先等待已加载地址同步，再保留 focus+selectionStart/End 全断言；单项及全组通过，未改生产逻辑/弱化要求。
  真实 `--site-data-probe-only` 在内部协议 helper 改名、开放网络裁决和 owner 收口后再次 PASS：
  两 origin+持续写页清空、另一会话不变、maintenance controller 和 fixture Profile 均清理。
  新独立预览 `dist/browser-preview-20260916-open-network/NomiFun-Browser-Preview.exe` 已构建：
  315996672 bytes，NotSigned，SHA256 `774683F2F11FFE2FD909FECDB193C0A148D9D8B16D58F64CD2333B6D89D053AC`，
  frontend `de8c011a-8e62-4775-82f0-5122eab38794`、API 29；EXE 内嵌前端 id、独立 channel 和 identifier 均已核对。
  启动器使用新的旁路 data/work 目录，不覆盖旧预览或正式数据。该包尚未做本轮主 GUI 站点清理/
  F12 不支持/开放网络的实际点击验收，`gui_retested_after_this_build=false`，不当作最终发行。
  已另建无真实密钥的 deterministic GUI fixture：数据目录 `dist/browser-preview-20260916-site-data-gui-data`，
  page/control `http://127.0.0.1:51885/`，seed AgentSession `01a0a864-5c93-7572-92a5-bf2b5e162578`，
  status 显示 model_calls=0/native_actions=true/failure=null。预览目录新增 `Launch-Site-Data-GUI.cmd`，等待用户手动启动；
  Ready 只证明数据/本机服务就绪，不是 GUI 通过。连续三轮均未发现该预览 EXE 进程，最后一次只读核对仍为
  `preview_status=not_running`；为避免长期占用资源，已正常 POST `/shutdown`，确认 fixture 进程和 Cargo runner 均退出。
  数据集、预览和启动器保留；用户恢复时须重新创建新的服务端口并更新/重建启动入口，不能将已停止服务当作验收失败或通过。

- 跨进程 HTML drag 收口复核（2026-09-16）：独立并发审计未改代码。同页 `--html-drag-only`
  已通过 trusted dragstart/dragenter/dragover/drop/dragend、原始 DataTransfer、pointer capture 和取消恢复；
  `--frame-drag-cancel-only` 已通过跨进程取消的 trusted dragend(none)，目标不提交 drop。
  严格 `--frame-drag-only` 仍失败：目标 OOPIF 取得 trusted drop 和原始 text/custom MIME，但真实源 renderer
  没有成功 `dragend(move)`，只在失败清理后得到 none。Chromium 当前在最终落点 RenderWidgetHost 同时
  调用 DragTargetDrop 和 DragSourceEndedAt；CDP 没有“指定真源 widget + 已协商 effect”的完成接口。
  WebView2 Composition 的支持路径是系统 OLE DoDragDrop/IDropSource，需要全局系统指针/按键与命中，
  不能由 WebView2 虚拟 SendMouseInput 独立驱动；改用 SendInput/SetCursorPos 会影响用户系统输入，违反输入门。
  因此不切换整个生产宿为 Composition，不关闭站点隔离、不合成 DOM dragend、不将目标 drop 单独称为成功。
  当前交付选项只有等待上游修复/增加 source-completion API，或由产品明确裁决 Windows 首版在按下前
  拒绝跨进程 HTML drag（不得声称支持）。其余同页真实 drag 不因此降级。
  为避免现有生产路径先对目标产生 trusted drop、随后才因源端不能完成而报错，已加入输入前 fail-closed：
  根据 fresh observation 内部 frame route 比较 source/target 的 native protocol session；不同 session 在首个
  mouseDown 前返回 unsupported，不移动或按下鼠标，也不产生部分 drop。root/同进程 iframe（均为 None）及同一个
  精确 OOPIF session 保留原真实 drag 路径。session 判定单测 1/1 通过；真实 `--html-drag-only` 重跑 PASS，
  完整 trusted drop/dragend(move)、原始 DataTransfer、取消和替换拒绝均保留。不把该拒绝表述为跨进程 drag 支持；
  上游能力出现前仍是明确平台限制。

- DevTools 产品范围删除（2026-09-16）：用户明确裁决整体可见 DevTools 功能代码从本期删除，
  内嵌浏览器不支持 F12/Inspect。ADR 已删除菜单、上下文设置和用户操作要求；生产所有普通页、
  popup 和站点数据维护 view 均固定 `devtools(false)`。删除 `browser_devtools.rs`、`--devtools-probe-only`
  及 smoke 注册/分支，不保留未达产品要求的探针代码。历史记录中对该探针的数据仅作失败/
  决策证据，不再是 TODO。不删除 WebView2 的宿主内部协议 API：观察、原生输入、Frame、文件选择和
  生命周期依赖 `CallDevToolsProtocolMethod`，但已将自建 helper 从 `devtools*` 改名为 `protocol_call*`，
  明确它不是面向用户的 DevTools 功能。上游 COM 类型/方法名保持 SDK 原名，不 fork 系统 API。
  网络边界保留为最小安全层，不建通用代理平台或用户设置页。

- 站点数据清理正式接入（2026-09-16）：新增用户专用 `ClearSiteData { runtime_generation }`，复用 Workspace
  user_operation、现有关闭全部页的创建/弹窗/在途任务边界与 PendingWork，不新增 Agent capability、数据库或全局设置。
  未创建 Runtime、旧代际、运行中用户请求及 Agent 请求均拒绝；载荷不能指定 Profile、目录或其他用户/会话。
  Windows 使用 `site_data.rs` 等待 ALL_SITE | DISK_CACHE 的原生完成回调。清理 controller 不进入 Tab 注册表，
  只允许 about:blank，隐藏、禁用输入/DevTools、拒绝新窗口。创建前登记精确 label，失败保留归属；已确认创建的
  view 也只在 native close 完成后释放。清理已分发后不 race/drop 等待，不以超时或 renderer/worker 退出当作完成。
  独立 UI-thread Pending 注册保留 Core/Profile 和 browser-exit 监听；恢复关闭前通过 UI barrier 等待该 view 的
  原生清理收尾，覆盖原等待者意外结束的归属问题。未知创建/关闭/回调错误保持 WorkerFailed 围栏，不允许静默复用。
  会话菜单新增确认框，说明关闭网页、丢弃未保存内容及清理登录/站点数据；取消不执行，Agent 后启动则不能确认，
  会话切换/Runtime 换代使原确认无效。只有命令成功后显示清理完成；失败不自动重试，并提供需要确认的“重新打开浏览器”。
  删除探针中独立的 Profile 清理器，`--site-data-probe-only` 现直接调用生产用户命令。最新真实回归 PASS：
  取消在 native dispatch 前保留数据、已关闭 view 不报成功、持久性前置对照、六类数据清理、另一会话不变、
  maintenance controller 销毁、native_fixture_profile_cleanup=true。原 `--close-all-only` 也 PASS，持久数据仍保留。
  UI 63/63（343 assertions）、平台清理权限/载荷 2/2、Browser 工具 13/13 通过；工具 schema 不含清理命令。
  typecheck、desktop-ui boundary、browser boundary、i18n（7687 keys）及 diff 检查通过。最后补 settlement barrier
  时漏导入 Tauri Manager 造成一次编译失败，补齐后最新 native run 通过；未放宽原生断言或延长探针超时。
  尚未重打包当前 conversation 预览；正式主 GUI 的确认清理点击、清理已分发后的异常退出/关应用/大数据量及
  多页并发故障矩阵仍需进一步验收，不能将这里的正常路径、预分发取消和单页双 Profile 对照扩大为完整交付。
  后续补齐调度归属和多页对照：两项 start_paused Workspace 用例主动 abort 原 user_command
  等待者，证明已启动的 Runtime 清理仍独立持有 operation gate；新 Agent begin 与 Workspace close 均等待
  受控 native settlement，不提前 close、不重放清理，4/4 定向平台测试通过。真实 WebView2 用例中增加
  `127.0.0.1` 与 `localhost` 两个 origin，第二页在清理前持续写 localStorage；生产命令先关闭全部页
  再清理，两 origin 的 Cookie/Local+Session Storage/IndexedDB/Cache Storage/Service Worker 及持续写标记均为空，
  另一 Conversation 不变，native fixture cleanup 通过。这扩大了正常并发覆盖，仍不代表强制杀进程/
  app 在完成回调前崩溃的数据结果已被证明。

- 站点数据清理原生路径验证（2026-09-16）：新增 `support/browser_site_data.rs` 与 `--site-data-probe-only`，
  只使用本次 native smoke 的全新临时根目录，在其下通过生产 `for_conversation` 选择两个独立持久 Profile。
  不使用 incognito 的自动清空作为成功证据。先种入 Cookie、Local/Session Storage、IndexedDB、Cache Storage、
  Service Worker，再关闭/重建第一会话网页，证明 Cookie 和其他持久数据仍在，只有新 Tab 的 Session Storage
  自然消失；第二会话保持原页作对照。首次 Cookie 未设有效期，普通关闭导致其正常消失，对照失败；改为
  Max-Age=3600 的持久 Cookie 后保留相同持久性断言，未运行到清理就当作通过。
  随后发现从已关闭 WebView 捕获的 Profile 执行清理返回 0x8007139F。调整为同一 owned Profile 下的短生命周期
  空白原生 controller：不聚焦、隐藏、禁用用户输入/DevTools，只允许 about:blank、拒绝新窗口，不进入 Workspace
  Tab 注册表，不执行站点内容或 Agent Browser 操作。先关闭当前会话所有网页，等待
  `ICoreWebView2Profile2.ClearBrowsingData(ALL_SITE | DISK_CACHE)` 的真实完成回调，再关闭清理 controller，
  最后重建网页验证数据。COM Profile 保留与释放均在 UI 线程，未用 wry 那个仅发起请求、不等待完成的封装。
  最终 native smoke PASS：持久性前置对照、网页先关闭、原生完成回调、清理视图销毁、六类数据为空、另一会话
  数据不变，外层 native_fixture_profile_cleanup=true，exit 0。两次失败同样保留；没有清理个人浏览器、正式数据
  或此前 conversation 预览数据。编译检查通过，已有未使用符号/旧 opusic PDB 警告不影响这次执行。
  依据 [Microsoft Profile2 API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2profile2?view=webview2-1.0.4129.50)
  核对数据种类与完成回调；关闭后对象失效及替代顺序来自本机实际验证，不从 API 名称推断。
  这仍是原生路径探针，不是生产清理命令/菜单已经完成。后续必须将短生命周期 controller 接入现有 Runtime
  关闭/取消权威，未确认完成时禁止显示成功或放行新一轮复用；再接用户确认菜单与代际/Agent 拒绝/故障回归。
  探针的 15 秒回调超时只会报失败，不能照搬为生产中“超时后可安全继续”的语义。没有新增全局设置、数据库或迁移。

- 真实模型主 GUI 前端闭环（2026-09-16）：用户手动启动 conversation 预览，实际窗口 5185058，EXE/hash 与下述
  预览一致。使用 computer-use 技能从真实 Agent 工作台选择 Browser act/navigate/observe、Read、Edit，保存并
  从“使用 Agent”创建会话，通过原生目录选择器选择该独立数据集 work，没有新增测试产品面板。
  会话 `01a0a7e4-d600-74c0-9806-87800d437e42`；真实 StepFun 共 19 次中转请求，非固定工具序列模型。
  第一轮 Browser 打开原生页，真实点击得到 2，nonce `6cf337a1-2cee-45b4-bfe5-546c198f701d`。随后模型请求
  未提供的 Glob，被 provider-stream 工具白名单拒绝，界面显示 provider gateway error，页面保留并解锁。
  明确提示只用已提供工具后，第二轮 Read/Edit 将唯一文件 app.js 的 `value + 2` 改为 `value + 1`，重新导航，
  真实点击/重新观察依次得到 1、2、3；同一 Tab `browser-01a0a7e4-ddec-7390-ab55-996eb364020c`，runtime=1，
  新文档 generation=2，引用 observation_generation=3/4/5，三个回执均 browser_input/completed。
  固定 witness 页面记录 nonce `9306f6fe-348f-4c82-a70b-c81de3331c05` 的 trusted 1/2/3，served_versions=2，
  changed_source_served=true；模型正常结束后，用户路径点击到 4，nonce 不变。运行中一次用户路径点击未增加额外
  witness，但此轮夹具没有单独时间戳围栏，不把它替代已有确定性的输入锁测试。仅读取此测试数据库的 tool_call
  记录交叉核对，并检查 app.js 实际文件，没有读取个人浏览器数据。
  本轮另有 3 次 Browser 参数校验失败（observe 多传 target 两次，点击对象扁平化/字符串化一次），模型自行纠正；
  不声称一次无干预成功。失败保留。通用提示确有无条件推荐 Glob 的措辞，现已增加“只调用当前请求提供的工具”
  和“已知路径直接读取”限定；Browser schema 增补 observe 示例及嵌套 action/reference 提示，仍严格拒绝错误参数，
  不添加别名、兼容参数、自动启用能力或工具执行重放。提示改动尚未打入本次已启动预览。
  提示回归 `nomi-agent --test tool_guidance_prompt_test` 13/13 通过；生产 Browser 工具经桌面 example test target
  的 `browser_tool::tests` 13/13 通过（70 项未选中），新增回归核对 observe 示例有效且此次错误载荷仍被 schema
  与反序列化双重拒绝。browser boundary 和 diff 检查通过。后者因共享 Agent 依赖链重编译耗时较长，但实际运行
  0.02 秒；未重复启动构建，未将这次纯单测当作新的 GUI 或提示效果的真实模型复测。
  物理键盘验证：原生页焦点下 Ctrl+L 聚焦并选中原地址；浏览器栏 Ctrl+T 创建空草稿、Ctrl+W 关闭草稿后原页仍为 4。
  未据此声称其他快捷键、框架开发服务器启动/构建、多页场景、macOS 或完整发行均已验收。
  验收结束正常 /shutdown，runner session 37400 已确认 stopped/exit 0。窗口与测试数据保留，但此服务不再接收请求。

- Profile 决策落实（2026-09-16）：用户确认选择简单、交付快的方案，并同意手动启动独立 GUI 验收实例。
  内嵌 Profile 改为已认证 user id + conversation id 的长度前缀 SHA-256 身份，使用全新
  `browser-v2/conversations/<identity-hash>/`；临时或无 workspace 会话仍 ephemeral。HTTP 用户入口与 Agent
  resolver 同时替换为 `for_conversation`，删除项目路径解析方法及无用的 RuntimeRequest.workspace 字段。
  不建立 profile.json、不增表、不迁移、不读取或删除旧数据。ADR 和中英文架构说明同步更新。
  平台 3 项身份/隔离/临时目录单测通过；主应用 browser_workspace 3 项串行测试通过（4.97 秒），其中新增用例
  分别从 HTTP 与 Agent 入口创建并关闭 Runtime，证明两条路径一致、同一项目不同会话目录不同、临时目录不持久化。
  项目路径故意不存在，目录选择不再依赖 canonicalize。首次用例中的临时 workspace token 不合法导致 HTTP 500，
  修正测试 token 为规范 UUID 后通过，没有修改生产校验。browser boundary、desktop-ui boundary、runner self-test、
  diff 检查与前端 build 通过。Rust 链接仍提示旧 opusic PDB 缺失（调试信息警告），不影响测试执行。
  这些是 Profile 合同/接线验证，不代表 DevTools、网络出口或完整 GUI 交付；不再以 Profile 未决作为阻塞理由。
  GUI 仍由用户手动启动，不重试此前被策略拒绝的自动启动。旧预览和正式数据保留。
  新预览 `dist/browser-preview-20260916-conversation/NomiFun-Browser-Preview.exe` 已构建，315879424 bytes，
  NotSigned；SHA256 `17991A7BB95C6A609FB349F0C0A842DC0D7BC09537DA9121ABF059822E75CC37`，
  frontend `11f9611c-0d76-445d-9982-541505fbfc7e`、API 29，已核对 EXE 内嵌前端标识及独立频道。
  此包同时纳入此前已验证的键盘接线；附普通/真实模型两种手动启动器，使用不同新数据目录，未自动启动 GUI。
  已用既有专用测试凭据准备新 live fixture，Ready page/control 为 `http://127.0.0.1:65160/`，
  session `01a0a7dc-c6ff-70c2-9c19-87a2b0c06a97`，数据目录 `dist/browser-preview-20260916-conversation-live-data`。
  本机服务等待用户通过 `Launch-Live-GUI.cmd` 手动打开窗口；Ready 仅表示服务/种子配置就绪，尚无本轮真实模型
  推理或 GUI 通过证据。服务有 30 分钟上限，过期需重新准备，不将旧数据集复用为新验收。

- 交付阻塞收口（2026-09-16）：复核后没有等待中的 Cargo/rustc 构建；此前 GUI 启动拒绝未获新的允许条件，
  本轮不重复或换通道尝试。该限制在后续键盘接线、搜索共存验收期间一直存在，相关独立工作已完成。
  Profile 的“会话独立/项目共享”选择仍未得到确认，不能擅自改写已冻结的数据边界，也不继续堆叠建立在未定边界上
  的 DevTools/站点数据/窗口生命周期实现。当前需要用户确认 Profile 方向并配合真实 GUI 验收启动，才能继续关键交付。
  原目标不缩减：跨进程拖拽、受控 DevTools/站点数据、popout/归位、真实 GUI/自主模型、发行验收等仍保持未完成，
  已通过的局部测试和 unsigned preview 均不当作整体交付。保留现有代码、数据与预览供继续工作。

- 本地/原生搜索共存主应用验收（2026-09-16）：新增 `tests/local_websearch_coexistence.rs`，复用正式 DesktopServer
  HTTP 创建 Provider、保存 Preset、创建会话并发送 turn；选择 nomi_local_websearch、web.search、citation.render。
  普通模型调用只出现三个独立 function 工具，不直接附加隐藏的原生搜索，也不出现 Browser/nomi_system_browser。
  真实安装 Chrome 完成本地公开网页检索；web_search 则通过生产 SearchProvider 发送到本机 Responses 协议夹具，
  核对 native tool_choice、sources include 与 store=false，不使用真实 OpenAI 账户。模型主循环 4 次、已有意图分类
  1 次、原生搜索端点 1 次；citation_render 同时返回本地/原生 ID 及各自 URL，未互相覆盖。
  与既有无原生搜索 trait 的 Chat 模型用例一起串行执行，2/2 通过（34.34 秒），两个临时后端与搜索浏览器均清理。
  这证明当前主应用的选择/路由/引用合同，不替代真实 OpenAI 服务、GUI 来源卡片或用户热切换模型验收。
  OpenAI Docs 技能用于核对 [Responses 函数调用](https://developers.openai.com/api/docs/guides/function-calling) 与
  [原生搜索 sources](https://developers.openai.com/api/docs/guides/tools-web-search) 合同。夹具首次错误地选择不支持
  Responses 的 custom 平台，被 400 拒绝；改用支持该协议的 openai 平台和本机 endpoint，并修正 limit 参数。
  随后识别并单独处理已有的无工具意图分类请求，未修改生产路由或放宽工具注册断言。diff 检查通过，未改 GUI/数据库。

- 浏览器键盘接线（2026-09-16）：确认 browser.render_content 为已有后台 hidden operation，不改成额外用户工具。
  补齐常用键 Ctrl+L/T/W/R、F5、Alt+左右：普通页/popup 注册 Windows AcceleratorKeyPressed，先 SetHandled，
  再异步转给 main 的浏览器栏；遵循 [WebView2 同步输入回调约束](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2acceleratorkeypressedeventargs)。
  payload 带 conversation/完整 target，发布前检查原生运行锁、文档、关闭状态；renderer 再检查当前会话、活动文档、
  运行锁、busy/dialog，仍通过已有命令/服务端 user_operation 执行，不保留等待解锁的快捷键队列。重复按键、AltGr、
  Shift 与 Windows 组合不触发这些动作；controller 确认关闭后移除注册。没有增加网页 IPC、模型工具或用户设置。
  浏览器/SystemBrowser UI 相关 77 项通过（410 assertions），纯键映射补充校验 1 项通过；typecheck、desktop-ui
  boundary（1916 sources）、browser boundary、i18n（7682 keys）与 diff 检查通过。正式桌面 bin no-default-features
  cargo check 通过；原生映射单测首次在 chromiumoxide 编译时 rustc 0xc0000409 退出，未运行测试。
  一次 -j 1 重试完成，1 项映射单测通过（81 项未选中）。这是纯单测，不启动 GUI，不绕过此前被拒绝的 GUI 启动。
  真实用户按键→原生回调→浏览器栏焦点链路仍需 GUI 验收，当前 integrated EXE 未重打包此改动。

- 真实模型主 GUI 验收准备与执行阻断（2026-09-16）：已有 0915 的真实模型原生宿主基本闭环证据，不重复将其
  当作新的 GUI 交付。本轮给开发用 browser_gui_fixture 增加 --live-frontend：新建独立 work/app.js 计数器缺陷，
  固定 StepFun endpoint/model 的本机中转，真实 key 只经 stdin 保留在中转进程内存；桌面配置只存随机本机 bearer，
  不存真实 key。请求数最多 32、单次 90 秒、输出预算 4096，关闭时停止服务；没有新增用户测试面板。
  现有隔离 runner 与 Windows Credential Manager 入口增加 BrowserGui/DataDir 模式，Cargo/构建脚本/子进程 env
  继续不接收 key。只流出经过路径/loopback/session 白名单校验的 Ready 元数据，fixture 退出只标 stopped，不标 GUI pass。
  cargo check、runner 构建及两种模式 self-test 通过，已创建
  `dist/browser-preview-20260916-live-gui-data`，Ready page 曾为 http://127.0.0.1:49834/，seed session
  `01a0a6fc-935b-73c3-8a96-87d5835622a6`。这是配置/启动证据，不是本轮真实模型推理或 GUI 通过证据。
  随后“检查集成预览是否已运行→带独立 data/work 环境启动→本机未认证请求检查”的组合命令被执行策略在
  CreateProcess 前拒绝，未通过改工具、改命令路径或重复提交绕过。因而尚未启动此数据集的 GUI 或发送验收消息。
  只执行了正常 /shutdown 清理，runner 已确认 browser_gui_fixture_status=stopped 并 exit 0；测试数据保留，真实 key
  不写在这些文件中。后续 GUI 步骤需要用户手动启动配合或执行策略明确允许；整体目标仍未完成。

- 最新集成预览（2026-09-16）：`dist/browser-preview-20260916-integrated/NomiFun-Browser-Preview.exe`，
  独立频道 Windows x64、实际签名状态 NotSigned、315783168 bytes，SHA256
  `6CD92779592E3545FBC8A68FC1C092DE790ACDE8605668BFE2347CD6C8F9FB1C`，frontend
  `32b03cb6-bcc6-4eaf-aee6-8a9cef9a5f6b`，API 29。本包纳入系统下载目录、开发者脚本、此前 Focus Stop，以及
  禁止未受管理的原生 DevTools 入口；旧预览不覆盖。启动器使用新的独立数据目录，没有预置测试模型或搬迁正式数据。
  UI 与实际桌面 EXE build、浏览器边界、diff 检查通过；本包未再次进行完整 GUI/自主模型或安装发布验收。
  DevTools 管理仍未完成，不将禁止未受管理入口当作该功能交付。

- DevTools 原生路线核对（2026-09-16）：新增只针对临时 owned Profile 的 `--devtools-probe-only`，不操作个人
  浏览器。当前 Edg/153.0.4234.32 的真实结果：AreDevToolsEnabled=false 仍可由宿主 OpenDevToolsWindow 打开；
  该设置不会关闭已经打开的 DevTools（300ms 后仍可见）。窗口在 WebView2 browser PID，Chrome_WidgetWin_1，
  owner HWND=0；不能用父 HWND 将其直接绑定到某个会话。仓库 vendor/wry 的 Windows close_devtools 是空实现，
  is_devtools_open 恒 false，未将这两个 API 当作生命周期证明。
  同一临时 Profile 的 Target.getTargets 可见新增 devtools://devtools/bundled/devtools_app.html 目标；
  Target.closeTarget 对该唯一测试目标返回 success=true，实际 HWND 消失，原生页面/文档保留、Profile 清理通过。
  返回值没有 inspected-target/opener 关联，这一 fresh single-page probe 不证明共享 Profile 多会话的归属。
  尚未在生产菜单启用 DevTools，也没有靠窗口标题、类名或不受约束的目标差集关闭生产窗口。
  普通页和 popup 的 builder 固定 devtools(false)，关闭 F12/原生菜单这种没有会话归属的入口，避免独立窗口绕过
  Agent 输入锁；不是删除目标需求，后续受控菜单仍可通过宿主 API 打开（本机已验证 false 下 API 可打开）。
  已基于这项新证据询问是否采用会话独立 Profile，以简化网络与 DevTools 归属；未确认前保留原设计范围和现状。
  [Microsoft AreDevToolsEnabled 合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings#Get_aredevtoolsenabled)
  只承诺控制菜单/快捷键打开，不承诺关闭现有窗口。Windows-only evaluation example 的遗漏 cfg 门也已补齐；
  未在本机声称完成 macOS 执行验收。

- browser.evaluate 接通（2026-09-16）：原先已可选的 capability 未出现在正式工具中。现在 exact 选中后注册工具及
  evaluate operation，经 NativeBrowserTurn/Workspace run guard 到同一个原生 Tab；不选择时 schema/执行均拒绝。
  原生 root 页仅 HTTP(S) localhost/127.0.0.1/[::1]。独立开发者 world 与语义/页面全局分离，真实 DOM 可修改；
  Tab/Runtime/文档校验与单次 nonce 约束上下文，脚本不是 browser.act fallback，旧输入观察立即失效。
  5 秒浏览器 timeout、64 KiB 源码、128 KiB JSON 返回；不调用 Runtime.terminateExecution，避免其“当前或下一次
  执行”语义影响下一轮（[上游 Runtime 合同](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/json/js_protocol.json)）。
  WebView2 实际在死循环超时时返回 COM 命令失败而非 exceptionDetails，首次 smoke 因此失败；已明确转换成“未确认脚本
  完成”的 script_error，不猜测精确原因、不当作成功、不重放。脚本造成的 DOM/页面后台效果不回滚，Promise 不等待。
  对话框复用 PendingWork，原表达式保留，回复返回原脚本结果或错误；不因 dialog 丢失结果。
- 原生 `--evaluate-only` 已 PASS：同页 DOM 修改/重新观察、页面 globals 隔离、旧元素拒绝、异常/Promise/超大结果/
  死循环失败、超时后的 6*7、dialog 后原结果 42、过期文档与已结束 run 拒绝、执行中 Stop 收尾后下一轮 21*2 成功，
  临时 Profile 清理通过。Stop 以测试脚本同步 XHR 到本机 fixture 的实际接收为启动证据；先前尝试通过 console
  快照确认启动在 4 秒内未取得证据，保留为一次失败，没有将发送请求或固定延迟当作已启动。
  工作台增加开发者边界说明，11 项文案/配置测试、typecheck、desktop-ui boundary 与浏览器边界检查已通过。
  工具 opt-in/域名策略两项单测通过（真实桌面 example test target，79 项未选中），随后正式 Browser Tool 的 12 项
  schema/权限/回执单测全过（69 项未选中）；浏览器边界检查复核通过。
  未重新打包 native-stop 预览；仍不代表真实模型自主开发、全动作或完整发行完成。

- 下载目录入口（2026-09-16）：会话 Browser 菜单新增“打开系统下载目录”，普通页与 popup 的原生 Save 默认位置
  同步改为 Tauri `PathResolver::download_dir`（[上游合同](https://docs.rs/tauri/latest/tauri/path/struct.PathResolver.html#method.download_dir)）。
  不增加设置/迁移、不接受页面或 renderer 的路径；命令绑定已有 Runtime 代际，Agent 不可调用，运行时用户请求拒绝。
  只在原生 STA 调用 ShellExecuteExW 的 explore，不执行文件或命令行；失败提示保留当前网页，无自动重试。
  两项平台合同测试通过；实际 Windows `browser_workspace_smoke --downloads-folder-only` 返回 PASS：过期/运行中拒绝、
  OS folder handoff acknowledged、原生 Tab 保留、临时 Profile 清理。只证明 OS 接受打开目录，未查看 Explorer 目录内容。
  UI 浏览器面板 55/55（290 assertions）、typecheck、两项边界检查与 i18n 7681 keys 通过。最初从仓库根目录运行
  UI 测试漏用 `ui/bunfig.toml` 的 DOM preload 导致 popup 测试失败；显式 preload 的两项新测试、随后从 ui 目录运行
  完整组均通过，未为此改生产代码/断言。i18n 生成首次 Windows EUNKNOWN，重试后成功。
  尚未重新打包上一轮 native-stop EXE；该 EXE 不含这次菜单新增。站点数据清理范围仍需 Profile 边界选择，
  DevTools、剩余原生动作/自主开发闭环和完整发行验收不因本项通过而视为完成。

- Focus Stop 已完成真实 GUI 修复验收（2026-09-16）：新包 `dist/browser-preview-20260916-native-stop`，
  SHA256 `ED2C9790EA1DC74633DC972D7DC8AFD3AE0A4B7AEACA3841CEEBF525EEA10B92`，315602944 bytes，
  frontend `24b130e3-0a73-4f42-a437-f4dbfa996d9c`，独立频道 Windows x64 未签名开发预览。
  从实际 Agent 工作台保存三项能力并新建会话，模型 7 次请求、Browser 原生导航/观察/中文输入/可信点击均通过。
  Focus 模式聊天隐藏时，标题栏有“停止 Agent”；尝试用户点击后 witness 仍只有计数 1，原生 HWND 保持禁用。
  点击标题栏 Stop 出现“正在停止”，随后可手动操作，用户点击到计数 2，nonce 始终为
  `3933b8c3-615f-45d0-90f0-462d194ebf12`，中文保持。放行迟到的模型完成后，聊天只显示“你在 46秒 后停止了”，
  没有接受迟到回复或新增工具调用；本机服务随后正常 exit 0，已加载原生页面保留。
  该技能驱动的真实 UI 检查直接发现并促成 Stop 位置修复；13 项定点测试、typecheck、desktop-ui boundary
  （1914 sources）、UI build、桌面 EXE build 通过。首次 UI build 曾 exit 5 无具体诊断，后一次成功；
  首次 typecheck 的新测试未使用 React import 已修正。不是降低失败断言，不声称其他 UI/发布/自主模型验收完成。

- 主 GUI Agent 原生操作（2026-09-16）：真实桌面工作台选择并保存 browser.act/navigate/observe 三项能力，
  从“使用 Agent”发送普通聊天；固定本机模型沿正式 Browser Tool 完成 navigate→observe→type→observe→click→observe，
  共 7 次模型请求。浏览器自动打开且 HWND/导航控件锁定，页面显示“Agent 主界面真实输入”、计数 1，页面 witness
  为 trusted=true。正常结束后用户通过 Windows Computer Use 点击同页到 2，两次 nonce 均为
  `7fa6385a-f5a5-4d96-b9a0-718c3eed272a`，中文保留。验证的是实际聊天/原生页面链路，不是自主 LLM 开发闭环。
  独立预览 `dist/browser-preview-20260916-native-gui` 的 hash/build ID 见其 build-info；fixture 已正常退出。
  同时发现 1280 宽窗口、双侧栏占位后的 Browser Focus 模式隐藏了聊天 Stop。已在源码将原 SendBox 的 Stop 通过
  portal 移入当前会话标题栏，仅改变位置，保持原停止回调/去重/等待状态，不增加浏览器停止接口或接管状态。
  13 项布局/Stop/portal 检查通过，真实 GUI 修复验收与新包尚待完成；不把前一包当作已修复。

- 交付顺序纠偏（2026-09-16）：用户指出长时间没有完成整体交付。核对时无存活编译进程，三个子任务均已完成；
  不属于等待某个任务，而是工作过多集中于底层边界。当前优先真实主界面 Agent→原生页面操作、最新 Windows 包，
  不增加用户测试面板，不将脚本模型链路证明当作真实模型自主开发闭环，也不缩减剩余 ADR 要求。
- 隔离进程配额收尾：搜索、网页渲染和版本探测在 `headless_page::open_owner` 共用 2 个实际进程/16 个请求名额，
  排队消耗原请求 deadline，过载返回 Busy；名额随已有 HostCleanupLease 留存至进程/Profile 清理完成。
  本地检索 Busy 已映射至聊天来源组件；不增加设置，不影响内嵌或借用的系统浏览器。三项 admission/清理凭证单测
  与原检查通过。真实组在正确设置 `NOMIFUN_SEARCH_CHROME` 后为 11/12：公网 example.com 渲染返回 Blocked，
  同时系统 DNS 解析为 198.18.1.38，未修改地址拒绝策略。首次误用另一个测试组的环境变量导致六项缺少二进制路径，
  不计作浏览器行为证据。最新 UI build、typecheck、browser-platform 与 desktop-ui boundary 通过。

- 当前 WebView2 拖放复核（2026-09-16）：实测 Runtime 已为 `Edg/153.0.4234.32`，revision
  `@9aab8632678bdbd60c393455e1394ee523ba682d`，不同于先前 152.0.4191.66。生产 `--frame-drag-only`
  在该版本仍 exit 1：目标收到原始 text/custom MIME 的 trusted drop，源只在取消清理后收到 dropEffect=none，
  不符合成功 move 的 native dragend。未删断言、未合成事件或将部分 drop 当作完整成功。
  Composition probe 的页面本来就是 visible；绘制保持、提前确认 Handled、原生 MoveFocus、停止子 frame debugger
  等定点实验均未解决问题。153 当前复现有 DOM trusted dragstart/pointercancel，但没有收到原生 DragStarting
  回调；临时 callback-entry 诊断没有命中。不能将此直接归因于版本变化，因为尚未做同环境固定版本 A/B 验证。
  所有实验性输入/生命周期修改已撤回，保留的只有运行版本/可见性/事件证据和回调参数错误回传，避免把参数读取
  失败与等待超时混为一谈；example cargo check 与 diff --check 通过，未修改生产宿主。
  已向用户询问是否允许首版明确拒绝此类拖拽；没有得到确认前，原完整验收要求及失败状态保持不变。

- 工作台配置保存的 SQLite 竞争修复（2026-09-16）：定位到当前 Nomi-core ControlPlane store 的读后写事务升级。
  在真实文件 WAL 数据库中固定另一写者，旧 deferred BEGIN 的 append_revision 稳定返回与主应用一致的
  `database is locked`；另一个无时间竞态的测试证明其在首次读之前尚未持有写入资格（修改前两项均失败）。
  写事务统一通过 `BEGIN IMMEDIATE` 开始，在读取 owner/current revision 之前取得写入资格；所有原版本/digest
  CAS 检查仍在同一事务中，正常只读查询不改、不增加 SQLite 超时、不重放 HTTP mutation、不增加表或迁移。
  依据 [SQLite 事务说明](https://www.sqlite.org/lang_transaction.html)，此修复针对 deferred read→write 升级竞争，
  不声称超过既有 busy timeout 的任意长写锁也一定成功。
- 验证：本模块 6 项测试通过（7.25 秒），包括实际 WAL 写入资格、其他读者可读、等待既有写者、旧版本拒绝、
  两个并发 append 只有一条 Revision/一个成功/一个明确 409，以及原有 owner/退役/remote CAS 回归。
  主应用真实 local_websearch_agent 1/1 通过（22.28 秒），系统浏览器两项各 25 次模型调用也通过（11.34 秒）；
  先前数据库失败记录保留，不以重新运行代替锁冲突的受控复现。diff --check 通过。
  测试临时目录在 pool 关闭后仅对 Windows sharing violation 做最多 2 秒物理清理等待，生产保存不重试。
  一次早期失败遗留的 `C:/Users/rika0/AppData/Local/Temp/.tmpvY1fRy` 清理命令被执行策略拒绝，已保留该测试目录，
  未改用其他通道删除；它不是用户数据库，也不影响生产事务修复。整体浏览器重构仍未完成。

- 旧会话归属路由退役（2026-09-16）：移除 TaskSessionAuthority、LegacyUnscoped/Task/PendingAuthority 等模式、
  Lane/family 计数器、分配等待和开启开关。SessionRegistry 只保留连接内的有界身份/回调/事件路由，原 4096 live
  session 总上限不变；超限使该连接失效，不由通用 transport 猜测目标所有权并关闭目标。隔离 page owner 继续持有
  自身目标/进程关闭权。删除旧全局 attach loop、旧 quota 目标关闭 helper、任意 NOMI_CDP_WS_URL 手工 smoke
  及专属测试；真实隔离检索使用的 enable/handle attach 收口为 crate-private。
  attach helper 只放行已经由 read loop 登记且身份仍匹配的会话，迟到事件不能重新注册或关闭目标。裸响应登记保持
  幂等、不变更已知类型、不复活近期关闭会话；完整新 attach 可补齐响应先到时的身份，目标别名/身份变更拒绝。
  首次测试捕获并修复 closed 事件覆盖 crashed 分类的细节，原分类断言保留；队列/字节/回调上限及协议取消测试不变。
- 本轮验证：引擎串行 lib 295 通过、9 个明确 opt-in 未运行（18.08 秒）；真实系统浏览器 54/54 通过（14.18 秒）；
  正式 Windows 运行时供应的真实隔离公网检索 1/1 通过（13.79 秒）。scanner 自测、残留扫描、diff --check 通过。
  并行 lib 首次运行另遇 profile 日志捕获为空断言失败，未修改/宣称解决该测试的并发问题；串行覆盖已通过。
  主应用 local_websearch_agent 在保存测试 Preset revision 时返回 SQLite database is locked，尚未到浏览器调用，
  未重放该 mutation，也不计作本轮主应用检索成功。直接检索证明受改动的真实 transport/page 链路仍可运行，
  不替代失败的主应用配置保存验收。整体交付仍未完成。

- 旧协议后台回收与订阅层收尾（2026-09-16）：删除已无消费者的 `ObjectGroupReleaseDispatcher`、排队方法、
  两种 transport 构造中的 worker 启动和对应 shutdown/Drop 分支；每条连接不再保留这个空闲任务。连接读循环、
  pending command 取消与新版文档对象组的显式清理不变。继续删除仅旧 Host/Lane 动作使用的跨 Host task 事件
  Budget/receiver/全局 weak registry 和订阅入口。当前每 subscriber 与每 connection 的事件数/字节上限不变，
  保留并迁移普通订阅的收到后计费、Drop 退还、队列丢弃和溢出测试；旧跨 Host/Lane 数学/共享 task 测试随模型退役。
  scanner 新增旧 dispatcher 和 task 订阅恢复拒绝，零生产引用扫描、自测与 diff --check 通过。
  引擎 lib 297 项通过、10 项明确 opt-in 未运行；真实 attached_browser 组随后 54/54 通过（13.97 秒），
  主应用真实 local_websearch_agent 1/1 通过（21.28 秒），包含结果/引用与临时后端目录清理。
  本轮未修改原生 UI、浏览器输入、Profile 或用户数据；剩余旧 task-session 归属代码尚未清理，不宣称债务全部清零。

- 系统浏览器后台绘制修复（2026-09-16）：针对上一轮 Scroll 失败做单项定位，确认 `Input.dispatchMouseEvent`
  wheel 超时，fixture 为 visibility=hidden、focused=true、scroll=0，且无 wheel 事件（87.68 秒失败）。仅将
  `Page.bringToFront` 返回视为可输入不足。新增私有 Rendering owner，从已授权页的本轮首次非观察动作开始启用
  `Emulation.setFocusEmulationEnabled`，直到现有 run settle 的 `release_granted` 才关闭；不改变浏览器启动参数、
  OS 窗口、用户 Profile 或设置，不输出截图流。曾试验每次操作 ACK 后立即关闭，虽消除了超时但滚动还未生效，
  已改为 turn 生命周期，不放宽原断言或加观看延迟。正确生命周期下原单项 1.78 秒通过。
  机制参考 [Chromium EmulationHandler](https://raw.githubusercontent.com/chromium/chromium/main/content/browser/devtools/protocol/emulation_handler.cc)，
  本机真实运行结果而非 main 分支源码作为验收证据。启用/复原结果不明或 owner 被丢弃时仅关闭本连接；复原使用
  不暂停响应时钟的 Connection，避免已停掉的 dialog observer 使清理无限等待。3 项协议测试验证正常复原、复原失败、
  启用 ACK 等待被丢弃；现有取消 mock 增补精确的当前 session emulation 命令，原禁止迟到输入断言不变。
- 修复后完整 attached_browser 组 **54/54 通过**（14.15 秒），包括此前 root/iframe 滚动及 alert/confirm/prompt/
  sequential/OOPIF/Stop 对话框。真实主应用 system_browser_agent 两项也通过（各 25 次模型调用，11.60 秒），
  保留真实原子 down/up ACK、Stop、同页续跑、fixture 登录、未授权 Tab 隔离和退出不关原浏览器的断言。
  之前的 48/51、对话框间歇失败和失败复现记录继续保留；本轮通过不等于个人 Chrome 或完整 GUI/发行验收完成。

- 旧独立执行器退役（2026-09-16）：全仓生产引用检查确认 `CdpBackend`/`CdpHostRuntime`、旧 `BrowserEngine` 和
  Host/Lane task authority 没有新链路消费者。物理删除 18 个旧源码文件与 41 个专属测试/fixture/snapshot 文件；
  删除旧动作/evaluate、注入管理、身份快照执行器和公开 facade 类型。原 `input.rs` 中真实 CDP 输入、键盘/几何
  原语及其测试原样抽取保留；`download.rs` 保留新原生下载实际使用的文件名/魔数检测与测试；vendor 语义 bundle
  和 NOTICE 保留，新 native/attached 驱动继续自己持有文档/对象生命周期。删除无消费者的 psl/insta workspace
  依赖及引擎 async-trait/regex 直接依赖，Cargo.lock 随实际图更新。未删除或迁移任何用户数据库/Profile。
  边界 scanner 新增精确删除清单与旧执行器类型搬迁拒绝；自测、生产扫描和 diff --check 通过。
- 删除后验证：引擎全部 targets cargo check 通过，剩余 lib 测试 301 通过、10 个明确 opt-in 浏览器测试未运行；
  桌面全部 targets（含示例）cargo check 通过。真实 WebView2 `--frame-input-only` 通过，包含主页输入/锁定、
  同进程/OOPIF/嵌套点击与中文输入、select、透视/缩放、页面保持与临时 Profile 清理。正式主应用
  `local_websearch_agent` 再次通过（1/1，18.60 秒），无原生搜索 trait 的模型仍能调用真实隔离检索并取得来源/引用。
  新独立系统浏览器真实 Chrome 全组为 **48/51**（175.73 秒），不是通过：原对话框首个 alert 等待超时再次出现
  （connected=true、pending=true、dialog=None），另两项 root/iframe Scroll 返回 ExecutionFailed；根因尚未确定，
  不据静态删除检查把运行失败认定为与本轮无关，也不通过复跑覆盖失败证据。
  协议层尚有旧 task quota/延迟对象回收 helper 无生产消费者，仍需收尾；不声明历史债务已全部清零。

- Windows 主界面运行锁验收（2026-09-16）：新增开发用 `browser_gui_fixture` 示例，在不存在的独立目录中通过
  正式 API 准备本机模型/最简会话；准备后先关闭后端及数据库，再由真实桌面 EXE 独占该目录。fixture 只监听
  loopback，不读取个人模型凭据或浏览器，页面/模型不访问外网，不向生产应用添加测试接口或面板。
  通过 Windows Computer Use 在真实聊天与原生 child WebView 中验证：用户先点击到计数 1 并输入中文；发送消息
  后页面 HWND 禁用、导航/标签控制锁定，尝试点击计数仍为 1；本机模型正常回复后解锁，点击到 2；第二轮重新
  锁定，点击仍为 2；用户点击聊天 Stop 后解锁，点击到 3。全程中文“同一页面应保留中文”保留；Stop 后放行
  迟到模型回复，没有产生第二条回复或恢复运行。模型请求计数为 2，验收结束后本机 fixture 已正常 exit 0。
  此证据覆盖无 Browser Tool 的普通 Agent turn 也锁定原生网页，以及等待模型时的正常结束/Stop GUI；不替代
  在途浏览器原子动作取消、Agent 自动点击的主 GUI、失败 GUI 或真实模型前端开发闭环。
- 新独立 GUI 预览：`dist/browser-preview-20260916-gui/NomiFun-Browser-Preview.exe`，Windows x64、未签名，
  315826176 字节，SHA256=`3FC73FC76AE3326B52745BC3562FA0828A98CE6B98FCAD8561F43F2D4A22109E`，
  frontend build ID=`dfbff54d-c8a0-4dec-b425-faa7455a07ee`，频道 `browser-preview-20260916-gui`。含本地搜索来源 UI，
  未对此来源组件新增原生视觉验收；旧预览不覆盖。原生窗口保持打开以便检查，测试模型服务已经停止。
- GUI 验收发现聊天 Stop 图标在辅助功能树中没有名称：源码为 Stop/停止中补充本地化 aria-label/title，
  同时给共享发送/插入图标补充名称，不改变 Stop 状态机。6 项 stop 结构/生命周期检查、desktop UI boundary、
  i18n parity 通过；此补充发生在 GUI 预览打包之后，不声称上述 EXE 已包含该可访问性修复。

- 跨进程拖放排查（2026-09-16）：在现有 Composition probe 中试验 DragStarting 后停止普通 SendMouseInput，
  只沿 OLE DragEnter/DragOver/Drop 转发原始 IDataObject。真实 `--composition-drag-only` 仍 exit 1：原生
  DragStarting、源 OOPIF 和目标可信点击均成立，allowed=3，全部协商 effect=0，目标 drops 为空，源 dragend=none。
  因而“同时发送普通鼠标移动和 OLE 导致失败”的假设未成立；该试验代码已精确撤回，不增加替代生产路线。
  [微软 DragStarting 合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller5)
  与 [OLE 转发合同](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller3)
  已复核；文档/API 可调用不等于跨进程拖放通过。既有失败和完整成功 dragend 的验收要求继续保留。

- 本地检索消息呈现（2026-09-16）：正式聊天过程行、完成收据和直接工具消息接入同一个紧凑来源组件，展示查询、
  来源标题/域名/两行摘要与检索状态。用户点击才经已有系统浏览器打开入口导航；不预取网页、不自动打开标签页、
  不增加测试面板或权限入口。超时、challenge、取消和无效/截断输出不伪装成空结果；网页文字不作为 Markdown/HTML
  执行，仅接受精确本地 Provider 合同的 HTTP(S) 来源，拒绝带凭据/控制字符/非网页协议的链接。
  已有重试历史仍保留，重试详情同样可阅读来源。50 项相关 UI/消息汇总测试通过，最终组件与过程结构复测 17 项通过；
  TypeScript、i18n parity、desktop UI boundary、browser boundary 与 UI production build（25.07 秒）通过；保留既有
  bundle chunk/dynamic import 警告。尚未在原生主程序中取得此组件的视觉证据，
  也未更新此前的 Windows 预览 EXE；不把组件测试视为完整 GUI 验收。

- 本地检索运行时供应核对（2026-09-16）：正式 Windows 启动已经调用 `headless_browser_runtime::prepare`，
  从固定安装目录读取 Chrome PE 版本信息，向 DesktopHostServices 注入精确版本/二进制绑定；启动不运行浏览器。
  `discovered_release_searches_public_sources` 真实测试通过（1/1，12.56 秒）：使用同一启动供应函数，隔离检索
  `Tauri WebView2 documentation` 并取得非空公网来源及 `nomi-local-search-` 引用。此前“检索运行时供应未完成”
  不再准确；provider 直调本身不视作会话链路完成。
- 本地检索主应用链路（2026-09-16）：新增显式集成目标 `local_websearch_agent`，用正式 DesktopServer 和工作台
  HTTP API 创建仅声明 `function_calling` / `streaming` 的普通 Chat 模型，保存独立本地检索能力并创建会话。
  两次确定性模型协议调用之间运行真实隔离 Chrome 公网检索；检查精确工具名仅注册一次、未注册 `web_search` /
  `Browser` / `nomi_system_browser`、取得 1–3 个有效来源与引用标识、会话结束及临时后端目录关闭。
  `cargo test -p nomifun-app --features browser-use --test local_websearch_agent --quiet -- --ignored --nocapture`
  在明确指定测试 Chrome 后通过（1/1，17.40 秒）。测试未供应原生 Workspace 或系统浏览器连接，不读取个人登录。
  这证明无原生搜索 trait 的模型可通过正式会话调用本地检索，不替代工作台 GUI、真实模型自主检索、引用渲染、
  两种搜索同时启用以及完整隔离/资源/发行验收。

- Windows 主界面关卡（2026-09-16）：实际 GUI 发现两个阻碍，现已修复。首页原本必须先发消息才能进入有浏览器的会话；
  新“浏览器”入口复用当前 Agent/模型创建空会话，不发送草稿、不应用暂存自动化，也不展示“正在启动自动工作”的过渡。
  原生浏览器误继承文件 Preview 的 translateX(20px) 进场动画，首次挂载和收起再展开时会触发窗口越界校验；
  浏览器容器现在不使用该动画，并等待有效视口布局后才挂载，后端边界校验保持原样。
- 通过 Windows Computer Use 在修正版真实应用中完成：首次打开无需重试、本机 URL 导航、网页可信点击、中文输入、
  收起/重新展开保留计数与输入、并排 Chat/Browser、第二个标签页独立状态和切回原页保留状态。会话内无聊天消息，未启动 Agent。
  没有修改安全/能力授权或接入个人浏览器。此证据是 UserReady 主流程，不替代 AgentRunning/Stop 的完整主 GUI 验收。
- 最新可执行预览：`dist/browser-preview-20260916-fixed/NomiFun-Browser-Preview.exe`，独立频道与数据目录。
  SHA256=`33C8CBBDEBC766C778FD27FF65B5E609705D961EC97A2F0E8B9A1B9B573B480D`，
  frontend build ID=`d561e1bf-9b26-454d-b074-6848eb6b8bbf`。旧预览不覆盖。
  63 项 BrowserPanel/会话创建测试、10 项入口/布局结构测试、类型检查、desktop UI boundary、browser boundary 与打包构建通过。
  仍未完成整体交付，原有 Agent GUI、拖放、Profile/egress、检索会话与发行验收、旧代码退役与发行项继续保留。

- 交付收敛（2026-09-15）：用户指出长时间未形成整体交付；停止继续扩展验收场景，改按 Windows 可执行预览、
  核心 GUI 流程、清理与发行三个关卡推进。完整 ADR 与独立系统浏览器范围保持不变，不把预览版算作完整交付。
- 已生成并实际启动 `dist/browser-preview-20260915/NomiFun-Browser-Preview.exe`（315822080 字节，未签名）。
  channel=`browser-preview-20260915`，identifier=`com.nomifun.desktop.browser-preview-20260915`，
  frontend build ID=`72bb23f3-a564-4ef0-b42e-1c0bdc569654`。启动脚本固定独立数据目录，不使用正式版数据。
  通过 Windows Computer Use 实际检查首页、Agent 工作台与网页分类，确认两个独立能力条目可见；未启用权限、未连接个人 Chrome。
- 最新主应用真实 Chrome 两项链路通过，每项 25 次脚本模型调用，含 Dialog 回复、Stop 拒绝与迟到接受抑制；
  原子输入、同页续跑、既有 fixture 登录和退出保留原浏览器等断言继续保留。
  对话框事件顺序/响应预算支持的 10 项协议测试通过，相关 attach 全组最近 51 项通过，单项真实 Dialog 又连续复跑 3 次通过。
  必须保留此前 50/51 的真实超时失败：根因尚未定位，新增诊断只用于下次定位，不把复跑通过说成已修复。
- UI production build、TypeScript、desktop UI boundary、browser platform boundary 自测/扫描与 custom-protocol
  Windows 主程序构建通过。界面资源已经内嵌，不依赖 Vite 服务；仍有既有 PDB/helper/可见性及 bundle chunk 警告。
  会话浏览器完整 GUI 主流程、原生跨进程 HTML drag、检索运行时供应、剩余退役与签名安装包验收仍未完成。

- 范围纠正（2026-09-14）：用户明确不要浏览器内的终端、控制台、问题、测试步骤或专门测试产品流程。
  第三十三至三十五个切片的这些 UI 扩展已撤销，不再是交付条件。目标是 Agent 自动使用真实内嵌浏览器底层能力，
  用户无需进入测试模式；正常工具记录沿用会话，既有独立终端不改动。
- 平台顺序（2026-09-14 用户确认）：当前先完成 Windows；macOS 仅记录待实施项，Windows 完成后整理 Mac
  移交清单，由用户转交 Mac 环境执行。macOS 未完成不阻塞 Windows 阶段，但不能据 Windows 验收声明跨平台可用。
- 基线：`439bb385a`，当前工作区保留此前设计文档改动。
- 第三十七个切片的隐藏连续输入失败已由第三十八个切片修复：真实隐藏页面的点击、中文输入、End/Backspace 与
  移动元素拒绝已通过。完整后台跨 frame/崩溃/取消矩阵仍须验收，不由这一个用例代替。
- 已实现基础模块：原生 WebView 权限隔离、Conversation run 生命周期、WebView2 宿主专用 CDP 与输入门。
- Windows 正式 Runtime 已接入同会话 popup 消费（第二十六个切片）；此前“正式 popup 尚未开放”的切片记录
  表示当时状态。新页保留原生 opener/Profile 并继承当前运行锁，完整主应用视觉和剩余竞态验收仍未完成。
- 当前完整原生 smoke **未通过**：第十八个切片新增跨进程 iframe 拖放验收，发现目标有 trusted drop，但源页面
  缺少成功 dragend。第三十九个切片修复了取消收尾，失败后可得到 dropEffect=none 的取消结束，但不等于成功的 move。
  该失败项保留在默认完整矩阵；主页面 HTML 拖放的单独回归通过，不代表完整 conformance。
- Windows 最小原生验证程序已实际运行通过。生产 Agent 生命周期、会话 Browser UI 与 Nomi 原生 Browser Tool
  已接入；Role Contract 已升至 v2，Snapshot exact Provider 已接到 Native Workspace。完整 Role action/resource
  宿主替换与工作台原生视觉验收尚未完成。
- 第二个实现切片增加了 `BrowserWorkspaceService`、typed Runtime/Tab command、延迟创建与跨 turn 保留，并将
  Native Host 注入 `DesktopServer` / `AppServices` 的启动关闭流程。
- 第三个切片将依赖提前注入 AgentFactory，接 Nomi 的运行开始、Stop、正常/失败 terminal 和异常退出；新增
  本地客户端鉴权的会话 API，以及删除会话前必须成功的原生资源清理。
- 第四个切片增加 `BrowserAutomationPort`、typed element references 与主 frame 原生观察/动作实现；复用已有
  Playwright injected 语义内核、键盘映射与脱敏。尚未切换正式 Agent Browser Role/Tool。
- 第五个切片增加会话 Browser 面板、标签/地址/导航、桌面 Focus 布局、原生 Surface attach/update/detach 与
  Channel 状态推送。运行/页面 revision 使用 watch 合并通知，界面不轮询 HTTP；迟到的 HTTP 结果不会覆盖新的锁态。
- 第六个切片在 Nomi 中注册原生 `Browser` Tool，按 Snapshot 能力集合限制每个操作，使用当前 Conversation 的
  run guard。原生会话不再为这个工具创建 Hub Lane；渠道、定时任务、执行子任务依据宿主记录排除 Native Workspace。
  后台旧 Hub 执行的完整 headless 化仍须随 v1 退役完成。
- 第七个切片删除了后端契约/宿主投影中的 `browser.takeover`，Browser Role 升至 `2.0.0`，从实际 registration
  生成 target inventory 并重建 digest envelopes。Native Workspace 接收 Kernel 验证 Snapshot 的完整 exact
  Provider；用户先建页可以未绑定，首次 Agent 解析后绑定，后续用户 reopen 不清除绑定。没有数据库迁移。
- 第八个切片把发布的 Browser Role 收敛为 ADR 的 7 个成员，删除 Catalog 中的 `browser.identity` /
  `browser.site_memory`、旧 Wave 2 Role resource port/factory 与宿主注册。7 个成员不再要求持久化 `browser`
  绑定；真实 Kernel 编译/调用测试改为无 Browser binding，仍验证 exact Provider 的唯一分发。
  Native Workspace 路径不再读取旧浏览器偏好，不注入旧登录加密 key；其 Browser Tool 仍由 Snapshot 授权。
- 第九个切片扩展原生 Click：左/右/中键、1/2 次点击，按键种类与 click count 纳入 pressed-state cleanup。
  双击的第二次输入前重新确认同一元素仍可达且位置未变；目标被第一次点击替换时返回
  `BROWSER_ACTION_INTERRUPTED`，不会点击替代控件。Agent 工作期间关闭 WebView2 默认原生右键菜单，
  terminal 后恢复；网页 `contextmenu` 事件保留。真实 WebView2 smoke 已通过这些新增检查。
- 第十个切片增加标准 HTML select：按观察到的选项名称执行单选、多选和清空多选。通过浏览器协议聚焦控件，
  再发送真实键盘输入，不打开 OS picker，不写 DOM value/selected/selectedIndex，也不合成 input/change。
  每一步核对选项节点、名称、可用性、书写方向、焦点及选择结果；页面取消方向键时停止，防止切换错误的行。
- 第十一个切片增加 `search_runtime.rs` 搜索专用 Headless owner 和 `local_web_search` 工具原型。每次使用独立
  临时 profile、anonymous context 和单页；浏览器原始网络走只拒绝的本地代理，HTTP GET 经 exact-origin 校验和
  `SafeHttpClient::get_once` 的公共地址 DNS pinning 后转发。重定向回交浏览器，每一跳重新校验。
  代理端口保留权随 exact process/profile cleanup lease 移交，取消调用方不会提前释放该网络边界。
  该阶段尚未注册到 Catalog / 工作台，Snapshot runtime build/exact adapter 绑定也尚未接入。
- 第十二个切片发布独立 `nomi_local_websearch@1.0.0` 能力/Action/input/output 契约，增加独立 typed owner port，
  不调用厂商 ResearchSearch owner。工作台 `网页` 分类显示 `Nomi 本地网页搜索` 和数据外发/登录态说明。
  当前未绑定 verified runtime，真实 Catalog 返回 unavailable，UI 可查看详情但不能启用；这一点已由 HTTP
  集成测试验证。模型没有原生搜索能力不构成本地搜索的路由限制，但完整 exact runtime/adapter 绑定仍待完成。
- 第十三个切片接入精确运行时绑定：实现/egress 摘要、adapter 摘要、浏览器可执行文件摘要与实际 product 版本
  放入本地搜索能力的只读 schema annotation，由 Kernel 的能力/来源摘要冻结进 Snapshot。Session 读取时核对
  frozen/live 摘要，factory 重查文件和本地版本，搜索启动后在发送页面请求前再次核对 product。
  独立 `nomifun.local-websearch` 包避免本地运行时变化牵连厂商搜索的 package provenance。
  `DesktopHostServices` → services/factory → Snapshot Session → Nomi registry 接线已完成；宿主仍须显式注入
  已通过发行验收的运行时。默认 main 入口保持未注入，不能据此声称已向普通用户开放完整搜索。
- 第十四个切片物理删除 28 个旧文件：全局 Browser 页面、Settings 内容及测试、v1 common/browser 数据层、
  侧栏入口、两份 browser 翻译模块、URLViewer/WebviewHost 和后端 browser_login 模块。移除旧全局路由与
  设置重定向、库存轮询、配置类型和每种语言各 74 个旧设置 key；保留新会话 Browser 入口。
  PreviewContentType 的 url 变体及 UI 分支已删除，标准文档/HTML 预览保留；旧登录 API 返回 404。
  不迁移数据库、不删除用户偏好数据，不增加旧 URL 别名。当前架构/使用指南已改写为 v2 的真实状态。
- 第十五个切片删除 `browser_management.rs`（含 v1 管理 DTO、协调器和测试）及所有 `/api/browser/*` 管理路由，
  同时移除 services 中的显示策略迁移、旧启动偏好读取、资源策略恢复，以及 Nomi factory 的 v1 Browser 偏好读取。
  剩余 Hub 消费者固定无头；通用设置 API 拒绝写入退役 Browser key。旧数据库值保留且启动不改写。
  这不是 Hub 核心、身份 vault 或旧 Lane facade 的全部退役；这些仍须由 v2 运行时替换后删除。
- 第十六个切片增加 WebView2 的 iframe/OOPIF 会话 transport：只从当前 child WebView 的 Target 事件接收
  iframe 会话，按已证明的父会话递归配置 auto-attach，不枚举或连接浏览器全局 target，不暂停用户页面。
  每个 WebView 只有一个 frame owner；session 带 owner/generation，父 frame detach 连同后代一起失效。
  事件在 UI 线程有界复制，通过持续运行的有界消费者处理；队列溢出、格式错误和关闭均使路由不可用。
  原生 Tab 初始化和 observe 已使用该发现通道，未观察 frame 计数包含跨进程后代且去重；Agent 的 element
  ref/语义观察和输入仍只覆盖主 frame，不能将此切片称为完整跨 frame 自动化。
- 第十七个切片将同进程 iframe 和 OOPIF 接入统一 `SemanticFrames`：每个 document 保留独立 isolated world、
  父 iframe 的真实 node handle 与有命名空间的元素 ref。ARIA 文本与结构化 ref 保持一致，观察内容继续脱敏；
  frame 层级、总元素和文本均有上限，未连接/超限深度的 frame 不猜测归属。
  输入点逐层映射到主视图的 CSS viewport，核对父 iframe 的稳定性、命中和遮挡；支持实测的二维旋转、缩放、
  边框与 padding，3D/perspective 和尚未处理的 individual rotate/scale 返回 typed unsupported。
  键盘核对整个父 iframe 焦点链；点击后以及 Ctrl/Meta+A 后重新确认输入控件，焦点被页面改写时不继续插入文字。
  本切片通过 Windows 同进程、跨站和跨站嵌套的真实 click/type/press/select；其他跨 frame 动作仍待逐项验收。
- 第十八个切片修正 Agent 拖拽移动缺少当前 `button` 的问题（此前 raw CDP smoke 有该字段，Agent 路径没有）。
  现在同页 HTML 拖放产生真实 DataTransfer 与完整 trusted 生命周期，Pointer Capture 的八步移动也通过。
  拖拽逐步计时并检查取消，释放前重新验证原目标；失败/Stop 先调用浏览器取消拖放，再释放按住的鼠标键，
  避免在有效 drop 区域把取消清理变成提交。被 dragover 替换的目标不会收到 drop。
  驱动以 passive listener 观察源页面的真实 dragstart/dragend，不构造 DOM 事件、不读取或替换 DataTransfer；
  缺少结束事件返回 ActionInterrupted，不能仅凭 native 命令回调就报告成功。
- 第十九个切片增加 Windows 原生 popup 桥接原型 `browser_surface/popup.rs`。新窗口请求使用真实 WebView2
  deferral 与 SetNewWindow，创建同 Environment/Profile 的 child WebView，保留原 WindowProxy/opener，
  不通过重新导航 URL 冒充同一个新窗口。COM 对象和事件 token 保留在 UI 线程，异步侧只持有私有请求票据。
  绑定前验证精确 child 身份与原生输入锁；放弃请求会关闭尚未交付的 child 并结束 deferral，opener 导航/关闭
  会撤销待处理请求。原型仅在专用验证场景安装 listener，**正式 Runtime 仍拒绝 popup，尚未对用户开放**。
- 第二十个切片拆分 Native Runtime 的标签表与单页操作锁。Tab 使用 Arc 保留精确实例，observe/act 不再持有
  整张标签表等待 native callback；取得单页锁后重新核对关闭状态、运行输入门、实例与 target generation。
  现有页的导航/关闭命令与该页操作串行，释放按键与 Runtime 关闭也不在等待单页锁时占用标签表。
  Runtime 层隐藏可立即执行且不改变正在使用的 viewport；可见 resize 等待当前页操作，再重查活动 Tab。
  每次布局请求有内部 epoch，较新的 hide/resize 会作废旧请求，防止排队 resize 在隐藏后重新显示页面。
- 第二十一个切片接通布局取消到 IPC 与原生 UI 线程：每次 update 替换前次 layout cancellation，等待可见
  resize 时不再占用 attachment 锁；detach、非法 bounds 和更新序号检查会取消旧布局。该 token 与 Agent
  输入取消独立，原生 UI 真正执行前再次核对，调用也等待 UI task 完成而非只确认已投递。
  attach 初始保持隐藏，前端首个有效布局决定是否显示；前端去掉等待旧 update 才发送新布局的串行循环，
  modal/menu/隐藏可以抢先发送，旧测量与旧请求错误被忽略。切换会话后，旧浏览器 command 回包不再覆盖新状态。
- 第二十二个切片移出已有页命令的整表 I/O 等待：navigate/reload/stop/back/forward、原生关闭以及 history
  查询在保留精确 Tab 和单页操作锁的同时释放 registry；完成后重新取得状态并拒绝已关闭的 Runtime。
  因此等待页面响应不再阻塞该 Runtime 的 snapshot 或隐藏请求。普通新 Tab 创建阶段的整表等待仍需后续处理。
- 第二十三个切片给已有页导航命令接入原生取消：navigate/reload/history navigation 等待期间收到操作取消，
  会在同一个 WebView 上调用 stopLoading，并继续等待原导航回调结算，再返回 Cancelled。导航提交先于停止
  命令，不通过丢弃 future 假装操作已经停止；已经取消的请求不提交导航。
- 第二十四个切片将相同的原生导航取消接入新 Tab 创建，并将创建视为未交付候选：初始化或取消失败时，
  恢复原活动 Tab，取得原生关闭确认后才移除候选记录。关闭失败则保留 Failed 记录与资源权限以供重试，
  不把清理失败伪装成成功取消。正常创建仍在全部初始化完成后才激活新 Tab。
- 第二十五个切片将创建串行锁与 registry 分离。原生创建、初始化和导航不再长时间占用整张标签表；候选
  注册后持有自己的页操作锁，输入门的应用仍与 Runtime 状态变化串行。Runtime 关闭先标记关闭并发出取消，
  等待创建锁释放后才清理所有标签与 profile，防止控制器尚在创建时提前删除目录。
- 第二十六个切片启用正式 Runtime 的 scoped popup 消费。原生事件捕获来源 target 与当时的操作取消信号，
  创建前复核 generation、所属 Runtime、URL 和 8-tab 配额；Agent 运行但没有当前输入操作时拒绝请求。
  原生绑定前新页保持锁定，绑定后加入同一 Runtime 的标签表，激活并通过既有 revision 流展示。
  用户空闲时允许有原生手势的请求，运行期间继承锁，terminal 后恢复用户操作；输入释放会等待来源 popup 工作。
  原型测试保留显式 transport-only 构造器，正式 DesktopBrowserHost::new 默认启用消费者，不增加用户设置或旧架构别名。
- 第二十七个切片将旧路由旁仅用于测试的 URL 安全投影迁入 `nomifun-browser-platform::url_projection`，
  删除旧模块入口，不保留兼容别名；原九项测试一起迁移。Browser 工具的 tabs、navigate/tab 命令结果经统一
  投影，只在模型侧 URL 标识中移除 userinfo、全部 query/fragment，并拒绝调试入口及非网页协议。
  用户地址栏、真实 Runtime 快照与导航目标不改写；工具描述明确投影 URL 不是完整导航地址。
  同时补充 IPv6/IPv4-mapped 私网识别和投影前后长度限制。
- 第二十八个切片接入原生 WindowCloseRequested，网页自行关闭后清理对应 Tab、切换剩余活动页并推送 revision。
  Wry 销毁承载 HWND 时可能先结束事件通道，稍后才发到关闭通知；消费者在有界等待内处理该通知，避免提前退出
  留下已失效标签。只有取得原生关闭确认才移除记录，失败时保留 Failed 状态；并发清理会复查精确实例是否已移除。
- 第二十九个切片用原生 HistoryChanged/SourceChanged 更新导航元数据。网页内部历史变化可以直接刷新
  can_go_back/can_go_forward；同文档 pushState/replaceState/hash URL 更新不增加 document generation。
  删除每次命令后遍历全部标签查询 history 的逻辑；相关 native token 随 view 关闭注销。
- 尚未完成：Native Runtime/Conversation UI 的完整能力、全部 Role action/resource 宿主替换、本地搜索、旧路径
  物理删除、Windows/macOS 原生与签名包完整验收。Linux 按 ADR 后置。
- 当前机器为 Windows；macOS 输入 spike 与签名包的真实运行证据尚未取得。

## 代码与证据

| 项目 | 实现 | 验证状态 |
| --- | --- | --- |
| child WebView 无插件权限 | default capability 只使用 WebView label，删除 window 范围 | 核对 Tauri 2.11.2 authority：匹配为 OR |
| child 无应用自定义命令 | `browser_surface/security.rs` 包装主 invoke handler | desktop 编译通过，2 个权限单测与真实 child IPC 拒绝通过 |
| run admission / settle | `nomifun-browser-platform/src/run_guard.rs` | 8 项异步单测通过，含明确 terminal 证明、RAII、panic 与关闭/开始并发 |
| 取消时原子动作不提前退出 | owned worker 保持操作门直到 native future 返回 | cancellation、caller abort、queued work 单测 |
| Windows native transport | `apps/desktop/src/browser_surface/windows.rs` | CDP 调用等待真实 COM callback；专用 child HWND 输入门 |
| 原生输入 smoke | `apps/desktop/examples/browser_workspace_smoke.rs` | 实际点击、中文输入、键盘、nested wheel、pointer-capture drag，记录事件全部 trusted |
| 页面连续性 | 同一 smoke 的随机页面 identity | hide/resize/show、popout/dock 后 identity 不变 |
| 临时 Profile | 同一 smoke 的独立 TempDir + incognito | app 关闭后 profile.close() 成功 |
| Workspace 服务 | `workspace.rs`、`runtime.rs` | 5 个隔离/生命周期/坐标测试通过，含关闭失败后保留清理权限 |
| Workspace 与 native Runtime | `browser_surface/host.rs` | 真实 smoke 验证首次建页、run 输入锁、跨 run 同一 tab、controller close 与 profile cleanup |
| 桌面依赖注入 | `DesktopHostServices` → `AppServices.browser_workspaces` | `cargo check -p nomifun-desktop` 通过 |
| 原生文档代际与 History | Workspace typed navigation commands | 真实 smoke：过期 target 被拒绝、Back/Forward 成功 |
| Agent 生命周期 | `manager/nomi/browser_lifecycle.rs` 与 `agent.rs` | 5 个新增测试通过：正常、Stop、失败、异常退出、锁定途中取消 |
| 原有 Agent 行为 | Nomi manager 定向回归 | 原有 85 个生命周期/完成/取消测试通过 |
| 会话 API 与删除 | `router/browser_workspace.rs`、`BeforeConversationDelete` | HTTP E2E 通过；拒绝非本地请求/运行中用户操作，清理失败保留会话，成功重试删除 |
| 原生语义观察/输入 | `native_semantic.rs`、`browser_surface/automation.rs` | 真实 WebView2 observe → ref type → fresh observe → ref click 通过；事件 trusted、编辑值脱敏、已消费引用拒绝 |
| 原生按键与引用边界 | 同一真实 smoke | Enter 只触发一次按钮默认动作；disabled 控件拒绝；上一轮未消费的引用也被拒绝 |
| 会话 Browser UI | `conversation/Browser` 与 `ChatLayout` | 4 个交互测试、10 个现有布局测试与 TypeScript 检查通过；尚未完成真实主应用视觉验收 |
| 原生显示区域 | `browser_surface/commands.rs` | Desktop 编译通过；只接 main WebView，校验窗口 bounds、挂载 id 和单调更新序号 |
| Nomi 原生 Browser Tool | `manager/nomi/browser_tool.rs` | 3 个能力/运行权限测试；模型 ToolUse → 当前 Workspace Runtime 测试通过 |
| 跨 turn 调用隔离 | `NativeBrowserTurn` 固定一次 invocation 的 guard | 迟到 invocation 在新 turn 中读写都被拒绝；Native 生命周期测试增至 7 项 |
| 会话选择 | `browser_workspace_provider.rs` | HTTP E2E 证明渠道不取得 Native Workspace，Agent 与 UI 获得同一个 Workspace |
| Browser Role v2 / exact Provider | `plugin_tools.rs` → factory typed request → native resolver → Workspace | Wave 2 15 项单测通过；HTTP E2E 增加 v1 拒绝、exact 摘要变化冲突、UI reopen 保留绑定 |
| 用户先开页后绑定 | `workspace.rs` 的 `ensure_user` / `ensure` | Workspace 6 项单测通过，验证绑定不重建页面、不能通过 reopen 清除锁 |
| Browser 不要求用户选择资源 | 7 个成员的 resource contract 与 `nomi_core_resource_bindings.rs` | Kernel 编译/调用不带 Browser binding 通过；资源解析器测试通过，拒绝额外传入旧 Browser binding |
| 原生鼠标手势 | `runtime.rs` / `automation.rs` / Browser Tool schema | 真实 smoke：`dblclick`、右键 `contextmenu`、中键 `auxclick` 均 trusted；各按钮 down/up 数量和 buttons mask 正确 |
| 双击中途换目标 | 同一 smoke 的首次 click 替换按钮 fixture | 第二次输入停止，返回明确部分操作错误；新按钮未收到 click |
| 原生默认右键菜单 | `windows.rs` 输入门同步切换 WebView2 settings | smoke 读取实际 COM settings：Agent 期间禁用，用户可操作时恢复；没有用 fixture preventDefault 隐藏问题 |
| 原生 select | `native_semantic.rs` 的只读准备/核对与 `automation.rs` 键盘驱动 | 真实 WebView2 单选、多选、清空多选、重复选择无额外按键、竖排多选通过；input/change 全部 trusted |
| select 边界 | 同一真实 fixture 的 disabled/hidden/optgroup/重复名称/动态选项/拦截键盘 | 不可选目标不发送输入；已有不可访问选中项可通过原生 Home 重置；重建选项或取消方向键后停止，不继续切换新行 |
| 搜索 Headless 与 egress | `search_runtime.rs`、`launch_search_chrome`、`SafeHttpClient::get_once` | Chrome 真进程运行 JS、加载同源资源；HTTP/WebSocket/popup/TURN TCP/STUN UDP 私网探针无连接/数据；返回前临时 profile 已删除 |
| 搜索取消清理 | 同一真实 Chrome fixture | 正常 cancel 与直接 abort 调用方均通过，底层 owned worker 继续清理，临时 profile 最终删除 |
| 本地搜索工具原型 | `nomifun-ai-agent::local_web_search` | 4 项纯测试通过：输入、URL builder、tracking unwrap/dedup、citation 命名空间共存；未接工作台 |
| 本地搜索 Catalog / 工作台 | Wave 1/support inventory + Agent workbench 网页分类/专用文案 | 真实 HTTP Catalog 含独立 ID、无资源/冲突依赖并正确 unavailable；17 项相关 UI 测试通过 |
| 本地搜索普通 Tool 契约 | `nomifun-agent-domain-wave1` / Nomi projection | 9 项 Wave 1 单测及非原生搜索模型路由测试通过；不投影为 web_search / Browser |
| 本地搜索 exact binding | `LocalSearchBinding`、Kernel schema annotation、Session/factory 校验 | Kernel 编译/Session 测试通过：旧 Snapshot 拒绝新 adapter；文件改变启动前拒绝；本地绑定不改变 vendor package 来源锁 |
| Host 注入与真实 Catalog | `DesktopHostServices.local_web_search` | Chrome 本地 probe + 真实 HTTP Catalog 的 materialized 状态通过；未注入的 HTTP unavailable 回归仍通过 |
| 实际 browser product 校验 | `search_runtime::probe_search_runtime` 与每次 query 的 pre-navigation check | Chrome 真进程测试通过：product 不匹配时无页面 HTTP 请求 |
| v1 UI/登录/URL viewer 退役 | Sider/Router、config keys、ipcBridge、PreviewContentType、browser_login | 类型检查、导航与会话面板测试通过；33 个 office DTO 测试通过；真实 HTTP 确认旧登录接口 404 |
| 防止恢复旧入口 | `check-browser-platform-boundary.mjs` 退役路径/路由/配置/预览类型规则 | 自测及真实仓库扫描通过；旧 Hub 规则仍保留至后端硬切换完成 |
| v1 后端管理/迁移退役 | routes/services/factory/client_pref | HTTP 验证 14 个旧登录/管理方法返回 404；预置 external/version2/resourcePolicy 旧值后启动仍无头、数据原样保留 |
| 剩余后台行为 | 固定 AlwaysHeadless + 机器资源策略 | 启动策略 2 项测试、factory 43 项测试通过；没有用被删除的设置恢复外部窗口 |

本轮验证命令：

- `cargo check -p nomifun-desktop`：通过。
- `cargo test -p nomifun-browser-platform run_guard --lib`：8 passed。
- `cargo test -p nomifun-browser-platform workspace::tests --lib`：6 passed。
- `cargo check -p nomifun-app --no-default-features`：通过，非浏览器后端不依赖 Tauri。
- `cargo check -p nomifun-ai-agent --features browser-use`：通过。
- `cargo test -p nomifun-desktop --example browser_workspace_smoke security::tests`：2 passed。
- `cargo test -p nomifun-ai-agent --features browser-use --lib manager::nomi::agent::tests::`：原有 85 passed。
- `cargo test -p nomifun-ai-agent --features browser-use --lib native_browser`：7 passed。
- `cargo test -p nomifun-ai-agent --features browser-use --lib browser_tool::tests`：5 passed，包含鼠标/select 参数严格解析及权限/运行边界验证。
- `cargo test -p nomifun-ai-agent --features browser-use --lib manager::nomi`：134 passed。
- `cargo test -p nomifun-agent-domain-wave2 --lib`：15 passed，包含实际 Kernel materialization、Role v2 与 exact Provider dispatch。
- `cargo test -p nomifun-agent-domain-support --lib`：8 passed。
- `cargo test -p nomifun-agent-domain-wave1 --lib`：9 passed，包含本地搜索独立操作与输入/输出 schema。
- `cargo test -p nomifun-app --features browser-use --lib local_search_does_not_require`：通过。
- `cargo test -p nomifun-app --features browser-use --lib nomi_core_resource_bindings::tests`：7 passed。
- `cargo test -p nomifun-ai-agent --features browser-use --lib factory::nomi::tests`：43 passed。
- `cargo test -p nomifun-system --lib client_pref::tests`：14 passed，覆盖退役 key 写入拒绝。
- `cargo test -p nomifun-app --features browser-use --lib browser_startup_policy`：2 passed。
- `cargo test -p nomifun-app --features browser-use --lib browser_url_projection`：9 passed。安全 URL 投影独立保留为测试覆盖的原语，不保留管理 API。
- `cargo test -p nomifun-net egress::tests --lib`：11 passed，含 one-hop redirect 不跟随、私网拒绝。
- `cargo test -p nomi-browser-engine --lib search_`：2 passed；2 项真实浏览器测试需显式 `--ignored`。
- 指定 Chrome 后运行 `cargo test -p nomi-browser-engine --lib search_runtime::tests -- --ignored`：3 passed。
- `cargo test -p nomifun-ai-agent --features browser-use --lib local_web_search`：7 passed；真实公网搜索默认 ignored。
- 指定 Chrome 后运行 `cargo test -p nomifun-app --features browser-use --lib admitted_runtime_reaches -- --ignored`：1 passed，本地 probe + 实际 host 注入/Catalog 接线，不执行公网查询。
- `cargo test -p nomifun-ai-agent --features browser-use --lib web_search::tests`：8 passed（名称过滤同时包含 2 项 local 搜索测试），厂商原生实现保留。
- `cargo test -p nomifun-app --features browser-use --test browser_workspace`：1 HTTP/删除/Provider 绑定 E2E passed。
- `cargo run -p nomifun-agent-domain-wave2 --example target_inventory -- check`：通过，实际 registration 与 target inventory 一致。
- `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`：通过，派生清单及 digest 一致。
- `cargo run -p nomifun-desktop --example browser_workspace_smoke`：`BROWSER_WORKSPACE_SMOKE_PASS`，exit 0。
- `bun run check:desktop-ui-boundary`：通过。
- `bun run check:browser-platform-boundary`：通过；这仍是现有 boundary gate，完整 v2 退役 gate 随硬切换更新。
- `bun test ui/src/renderer/pages/companion/useCompanionClickThrough.test.ts`：3 passed。
- `bun test --cwd ui src/renderer/pages/conversation/Browser/BrowserWorkspacePanel.test.tsx`：4 passed。
- `bun test --cwd ui src/renderer/pages/conversation/components/ChatLayout`：10 passed。
- `bun test --cwd ui src/renderer/components/layout/Sider/capabilityHubNav.test.ts src/renderer/pages/settings/components/settingsNavigation.test.ts src/renderer/pages/conversation/Browser/BrowserWorkspacePanel.test.tsx`：10 passed。
- `bun test --cwd ui src/renderer/pages/conversation/Preview/context/PreviewContext.persistence.test.ts src/renderer/pages/conversation/components/ChatLayout src/renderer/pages/conversation/Browser`：16 passed。
- `cargo test -p nomifun-api-types --lib office::tests`：33 passed，明确拒绝 url 预览类型。
- `bun scripts/check-browser-platform-boundary.mjs --self-test`、`check:browser-platform-boundary`、`check:desktop-ui-boundary`、`check:i18n`、`check:dead-css`、`check:theme`：通过。
- `bun test --cwd ui src/renderer/pages/agentSettings/AgentCapabilityWorkspace.interaction.test.tsx src/renderer/pages/agentSettings/model.test.ts src/renderer/pages/agentSettings/capabilityGroups.test.ts`：17 passed。
- `bun run typecheck`、`check:i18n`、`check:theme`、`check:desktop-ui-boundary`：通过。

运行环境：Windows 10.0.26200，Tauri 2.11.2，Wry 0.55.1，webview2-com 0.38.2。早期 smoke 的范围不包含人工
硬件输入绕过测试、完整 IME composition、HTML drag/drop DataTransfer、文件选择器、跨 origin frame 或 macOS；
后续新增的跨 frame、HTML 拖放证据及仍失败的场景，以第十六至十八个切片的记录为准。

构建问题已经解决：旧构建缓存引用其他 checkout 的 Tauri permissions 路径，清理相关 Tauri package 缓存后恢复；
Cargo example 未包含 Common Controls v6 manifest，启动报 `STATUS_ENTRYPOINT_NOT_FOUND`，已给 examples 嵌入独立
Windows manifest，真实进程启动通过。

第七个切片的校验仅覆盖后端契约、Provider 接线与回归；未在此切片重新运行真实 WebView2 smoke，也未取得新的
macOS 或主应用视觉证据。契约修改后的维护顺序为 `target_inventory write` → `agent-v2-contract write`，再分别
运行两个 `check`；生成器从注册内容计算 Role/member digest，不手填摘要。

第八个切片同样没有新增完整原生 UI 验收证据。一次清单写入遭遇 Windows 1224 文件占用，待构建结束后重试
成功，随后两个生成器 `check` 均通过。资源解析器的既有全资源覆盖测试发现旧 fixture 使用
`plugin.product.read`；已改为生产代码实际识别的 `plugin.read`，没有为旧 ID 添加兼容分支。

第九个切片在真实 Windows child WebView 中完成上述鼠标动作与菜单策略验证，程序返回
`BROWSER_WORKSPACE_SMOKE_PASS` / exit 0。非法 click count 0/3/255 被原生入口拒绝，不产生点击。
这不是完整主应用视觉验收；中键打开新标签的 popup 路径仍未实现，文件选择器、JS dialog、运行开始前已打开
的原生菜单等输入隔离场景仍须单独验收，不能由默认右键菜单 settings 的验证推定全部原生弹窗已受控。

第十个切片真实 smoke 已通过（`BROWSER_WORKSPACE_SMOKE_PASS` / exit 0）。键盘导航会像用户按键一样产生中间
change，Tool 文案明确说明这一点；对自绘 dropdown / `appearance:base-select` 不使用 DOM 修改来伪装成功，
而是保留普通 click/press 路径。选项数量和标签长度有输入上限。macOS 尚未验证此驱动，不能由 Windows 成功
推断跨平台已经完成。键盘方向与非连续选择逻辑对照了 [Chromium 的 select 实现](https://chromium.googlesource.com/chromium/src/+/9dd7d49061ff6271c74f4dba9d90e11ae2a3dafc/third_party/blink/renderer/core/html/forms/select_type.cc)，并以当前 WebView2 的真实运行结果为验收依据。

第十一个切片的边界与失败证据：

- 真实 Bing 搜索已显式运行，但返回 `Blocked`。系统 DNS 将 www.bing.com / cn.bing.com 解析为
  198.18.0.12 / 198.18.0.109，触发保留地址保护。没有增加 Fake-IP/私网放行，也没有改用会话浏览器或厂商搜索。
  已向用户非阻塞询问真实 DNS 的验证环境；其他开发继续。
- WebRTC 私网 UDP 探针曾实际失败，证明仅 HTTP/WS 用例不足。按
  [Chrome 开关定义](https://chromium.googlesource.com/chromium/src/+/0096967fc01f86f3fbefa4d90b2fb403ca33b97b/chrome/common/chrome_switches.cc)
  改为新 Headless 所用的 `--webrtc-ip-handling-policy=disable_non_proxied_udp`，同一探针重跑通过；旧 force 参数未保留。
- Windows 系统 Edge 的同一测试未通过，原因是启动期间 `browser ownership commit failed`，没有放宽进程所有权证明。
  当前正向 Headless 证据只覆盖 Chrome，不据此宣称 Edge 或 macOS 已通过。
- 仍需真实搜索页面 E2E、更多协议/权限/下载/崩溃隔离矩阵、全局资源 governor 和启动孤儿 profile 回收。
  当前常规完成/取消后的 profile 删除证明不能替代进程崩溃恢复验收。

第十六个切片的验证（2026-09-14）：

- `cargo test --profile dev -p nomifun-desktop --example browser_workspace_smoke windows:: -- --nocapture`：
  8 passed。覆盖相同 session ID 的不同 owner 隔离、父 detach/后代失效、session 重用、别名/深度/数量限制、
  frame tree 去重、有界 UTF-16 复制、队列满/关闭拒绝、坏事件永久失效，以及没有 Agent observe 时连续消费 200 个事件。
- `cargo run -p nomifun-desktop --example browser_workspace_smoke`：最终代码返回
  `BROWSER_WORKSPACE_SMOKE_PASS` / exit 0。测试专用 child 加 `--site-per-process`，使用
  `127.0.0.1 → localhost → 127.0.0.1` 嵌套页面，实际发现两个独立 OOPIF session，并在各自 isolated world
  读取到对应 URL/title；加上同进程 srcdoc 共计三个后代。主页面不能读取跨站 contentDocument。
  移除父 iframe 后两个旧 session 均被拒绝；重复 frame owner 被拒绝；显式 controller Close 后已有订阅不可再使用。
  原有主 frame 的 trusted 输入、select、input gate、运行生命周期和 profile 清理检查同时通过。
- `cargo check -p nomifun-desktop`、`bun run check:browser-platform-boundary`：通过。
- 未修改 renderer/UI 规则，因此未重复运行 desktop UI boundary。未新增 macOS、主应用 UI 或跨 frame 输入验收证据。
  自动发现默认不等待 debugger，也没有添加任何远程调试端口。实现依据为
  [WebView2 CDP 文档](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/chromium-devtools-protocol)
  与当前 `webview2-com 0.38.2` 的 session/event COM 接口，验证以真实 WebView2 运行结果为准。

第十七个切片的验证与证据修正（2026-09-14）：

- `cargo test --profile dev -p nomifun-desktop --example browser_workspace_smoke -- --nocapture`：12 passed，
  包含两个新增 frame plan 用例和此前权限/会话/队列测试；`cargo check -p nomifun-desktop` 通过。
- 最终 `cargo run -p nomifun-desktop --example browser_workspace_smoke`：
  `BROWSER_WORKSPACE_SMOKE_PASS` / exit 0。三个子页面（同进程、OOPIF、嵌套 OOPIF）均产生真实
  click、中文 input、keydown、select change，父页面收集到 27 条 trusted 事件。
  两层带 border/padding、旋转/缩放的 iframe 使用主视图 native Input，未使用 DOM click/fill/select 兜底。
  主页面遮挡阻止点击，父焦点移走阻止 Press，Ctrl+A handler 转移焦点阻止后续文字插入；父 iframe 导航令旧
  ref 失效，重新 observe 后可以继续操作。3D/perspective 明确拒绝，不推断已支持。
- 正向 smoke 仍执行已有主 frame 鼠标/键盘/select、运行锁、原生挂载、History、IPC 隔离等检查。
  `bun run check:browser-platform-boundary` 通过；本切片未修改 renderer/UI 规则。
- 修正测试 HTTP fixture 的串行处理：浏览器 speculative connection 不再阻塞其他 iframe 请求。
  postMessage 证据使用有界事件等待，不把 child 命令回调完成等同于 parent 已收到消息。
- **修正此前 smoke 的证据边界**：Tauri `App::run` 在此平台没有返回到其后的 fixture profile 清理/断言。
  因而此前 PASS 不能证明那一段退出后清理曾运行。现已改用 `run_return`，实际触发过文件占用失败；保留
  精确临时目录所有权，有限等待并实际删除后才输出 PASS，最终包含 `native_fixture_profile_cleanup:true`。
  这不追溯证明旧运行留下的 fixture 目录已经删除，也不替代生产 Runtime 的全部崩溃/孤儿 profile 回收验收。
- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --verify-failure-exit`：故意失败路径
  实际返回 exit 1，且没有 PASS 输出。调用 shell 显式传播 `$LASTEXITCODE`。此负向验证不是产品测试失败。
- 尚未验证跨 frame 的完整鼠标手势/拖放/IME/全部 transform、macOS 或真实主应用 UI；这些仍属于原交付目标。

第十八个切片的验证与未解决项（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --html-drag-only`：
  `BROWSER_WORKSPACE_SMOKE_PASS` / exit 0，输出明确标注 `scope:html-drag-only`。
  同页 drop 正确传递 `text/plain` 和自定义 MIME 数据；完整 dragstart/dragenter/dragover/drop/dragend 为 trusted；
  Pointer Capture 收到八次按住左键的移动；进入有效 drop 区域后取消没有 drop，dragend 的 dropEffect 为 none；
  取消后同一页面可以再次拖拽；目标替换后没有向替代控件 drop。验证程序的临时 profile 实际删除。
- `cargo test --profile dev -p nomifun-desktop --example browser_workspace_smoke -- --nocapture`：12 passed；
  `cargo check -p nomifun-desktop` 和 `bun run check:browser-platform-boundary` 通过。
  本切片未修改 renderer/UI 规则，也没有新增 macOS 或主应用 UI 验收。
- **完整矩阵失败**：默认 `cargo run -p nomifun-desktop --example browser_workspace_smoke` 返回 exit 1。
  两个带二维变换、分属不同渲染进程的 iframe 之间，目标确实收到 trusted drop 和两种正确数据，但源页面没有
  dragend。驱动现在返回 ActionInterrupted（动作可能已经部分生效，需要重新观察，不能盲目重试），测试仍要求
  完整生命周期成功，因此继续保持失败；没有删除断言、默认跳过该用例或构造假的 dragend。
- 失败环境已由真实 browser protocol 返回值固定：`Edg/152.0.4191.66`、protocol `1.3`、
  revision `@cc2931e6363af1d70882ad63ee33b0e8cd524de0`。检索到的
  [Chromium InputHandler 实现](https://raw.githubusercontent.com/chromium/chromium/main/content/browser/devtools/protocol/input_handler.cc)
  在 drop 位置的 RenderWidgetHost 上调用 DragSourceEndedAt，这与跨渲染部件场景的缺失事件一致；这是源码与
  实测对应的原因线索，不是已经修复底层浏览器的证据。需继续验证公开原生输入替代路径或上游修复，保留完整需求。
  未取得该 Edge revision 对应的公开源码（按 revision 查询 Chromium 仓库返回 404），因此不将 public main
  的代码直接宣称为此 Edge 二进制的确定根因。

第十九个切片的验证与接线要求（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --popup-only`：
  `BROWSER_WORKSPACE_SMOKE_PASS` / exit 0，输出标注 `scope:popup-only`，临时 profile 实际删除。
  真实 child 保留 opener、原 WindowProxy 的 postMessage 来源身份和同 profile Cookie；`window.open` 后同步
  `document.write` 的内容也出现在最终绑定的真实 child 中。将 child 显示到原生区域后，实际点击产生 trusted
  事件并回传 opener。child 的应用 IPC 被拒绝。
- 非用户手势请求被拒绝；未应用输入锁或绑定到错误 child 被拒绝；未认领的已创建 child 实际关闭；请求 Drop、
  opener 导航及关闭后 pending 清零。代码设置每个 opener 最多 8 个 pending、10 秒过期；当前 burst 实测
  只有一个请求通过原生用户手势判定，因此不把该结果当成并发达到 8 个或全部过期竞态的验收证据。
- `cargo test --profile dev -p nomifun-desktop --example browser_workspace_smoke -- --nocapture`：13 passed；
  `cargo check -p nomifun-desktop` 和 `bun run check:browser-platform-boundary` 通过。
  主程序编译仍有新桥接未接线的 dead_code 警告，没有通过 suppress 或虚假注册把它标成已交付。
- 原先“等点击完成再处理 popup”的验证顺序实际得到过期请求；并行处理触发输入与 deferral 后通过。这说明
  正式宿主不能在浏览器动作等待 native callback 时，把 popup 消费者阻塞在同一 RuntimeState 锁上。
  接线须提供同会话 NewTabRequested 消费、opener generation/Provider 校验、Tab 配额与创建中资源保留，并让
  Stop/close 可以拒绝 pending 请求后再等待输入结算。不能复用普通 create_tab 的先导航/先初始化 DOM 顺序。
- Windows API 要求同环境、同 profile，且绑定前不操作目标 DOM。依据是
  [NewWindowRequestedEventArgs](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2newwindowrequestedeventargs)
  与 [WebView2 线程模型](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/threading-model)。
  Tauri 2.11.2 的高级 NewWindowResponse::Create 只接 WebviewWindow，因此这里走原生 deferral 来承载 child，
  并使用 Tauri 的 with_environment 保留环境。未创建额外顶层产品窗口。
- 本切片没有完成正式会话 UI 的 popup 展示、窗口名称复用、自行关闭、完整 Stop 并发矩阵或 macOS popup。
  默认完整 smoke 的跨进程拖放失败仍保留；popup-only 的通过不改变整体未交付状态。

第二十个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --runtime-locks-only`：
  `BROWSER_WORKSPACE_SMOKE_PASS` / exit 0，明确标注 `scope:runtime-locks-only`。
  使用实际 DesktopBrowserRuntime 而非 mock：原页面点击等待真实 popup deferral 时，snapshot 和 hide 均可完成，
  另一 Tab 可以创建；可见 resize 确实等待输入，活动 Tab 改变后实际调整新页面而不误改原页面。
  原生 Controller bounds 与输入结束后的 DOM viewport 共同确认隐藏没有改变原页面尺寸。
  另一个用例在 resize 排队后发出较新的 hide，输入结束后原生 Controller 的 IsVisible 仍为 false。
- 原生回调等待期间不能用页面 JS 查询替代宿主状态检查：初版测试的 DOM 查询一直等到 popup 过期，误把
  已经结束的输入当作仍在等待。现改为该阶段读 native Controller bounds，输入结算后再核对 DOM viewport。
- `cargo test --profile dev -p nomifun-desktop --example browser_workspace_smoke -- --nocapture`：13 passed；
  `cargo check -p nomifun-desktop`、`bun run check:browser-platform-boundary`、`bun run check:desktop-ui-boundary` 通过。
  正式 popup 消费尚未注册，相关 dead_code 警告仍如实保留。
- 测试中的 popup 绑定仍由显式并发消费者完成，没有通过放开正式 Host 的拒绝策略来制造“已接线”结果。
  后续仍需会话授权/Provider/opener 校验、Tab 配额、pending 请求与当前操作取消信号的绑定、用户空闲时的消费，
  以及 renderer IPC 的挂载请求串行锁整理。Runtime 级 hide 通过不等于整个 UI/IPC 隐藏路径已完成验收。
- 默认完整 smoke 仍保留跨进程拖放的 dragend 缺失失败，不因本切片的并发基础通过而缩减交付范围。

第二十一个切片的验证（2026-09-14）：

- `cargo test --profile dev -p nomifun-desktop --bin nomifun-desktop browser_surface::commands::tests -- --nocapture`：
  3 passed，覆盖 hide 抢先、detach 取消与旧序号不能显示、非法 bounds 使用最后有效矩形隐藏。
  这些是 IPC 调度逻辑测试，不冒充原生画面验收。
- `bun test --cwd ui src/renderer/pages/conversation/Browser/BrowserWorkspacePanel.test.tsx`：8 passed。
  新增覆盖 modal 抢先隐藏、迟到测量不得显示、卸载后 attach 直接释放、旧会话 command 结果不得覆盖新会话。
- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --runtime-locks-only`：exit 0。
  真实 WebView2 证明排队布局可以在输入仍等待 popup 时取消，且没有取消 Agent 输入；另一个有界 UI 线程
  阻塞用例证明“已经投递、尚未执行”的布局被取消后不会显示。既有尺寸/活动 Tab/隐藏/原生清理检查也通过。
- `cargo test -p nomifun-browser-platform --lib workspace::tests`：6 passed；桌面编译、前端 typecheck、
  desktop UI boundary 与 browser platform boundary 通过。链接仍出现既有 opusic-sys PDB 警告，测试实际执行通过。
- 取消传播只涉及 Surface，不增加 run 开始、解锁或接管接口。后续仍须完成真正主应用的会话切换/Modal视觉
  验收，并覆盖导航和 Tab 创建等仍可能持有 Runtime registry 的 I/O 场景；本切片不据此宣称全部隐藏路径均已验收。
  正式 popup 消费尚未开放，跨进程拖放的完整生命周期失败也仍待解决。

第二十二个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --runtime-locks-only`：exit 0。
  新增真实 HTTP 延迟响应：测试服务器确认收到请求后等待两秒；在导航仍未完成时，snapshot 和原生 hide
  均在各自 250ms 验证窗口内完成，Controller 的 IsVisible 为 false。
- 初版验证依赖 page-load Started，但该通知没有覆盖服务器响应前的阶段；现以服务器已收到请求的独立
  信号证明导航正在等待，不用未发生的 load 事件或固定睡眠猜测浏览器状态。
- `cargo test --profile dev -p nomifun-desktop --example browser_workspace_smoke -- --nocapture`：13 passed；
  `cargo check -p nomifun-desktop` 通过。该切片没有宣称 StopLoading 可打断同页尚未完成的导航，也没有修改
  创建阶段的资源保留策略；这些与正式 popup 消费、主应用视觉验收仍属于后续完整交付工作。

第二十三个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --runtime-locks-only`：exit 0。
  真实延迟 HTTP 用例验证：预先取消不发请求；服务器已收到导航请求但仍延迟响应时，取消使原生导航在一秒
  验证窗口内停止并结算，早于服务器两秒响应；之后同一页面可以继续导航。
- 原生验证输出包含 `navigation_cancel_stops_native_load` 和 `navigation_recovers_after_cancel`。
  13 项 example 单测与桌面编译通过。未改 renderer/UI 规则。
- 范围仍有限：尚未覆盖新 Tab 创建中的取消、全部停载失败/崩溃路径，也不声称 UI 的 StopLoading 命令
  可以越过同页操作串行队列。正式 popup、主应用完整验收与跨进程拖放问题仍保留。

第二十四个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --runtime-locks-only`：exit 0。
  新建 Tab 已向延迟服务器发出请求后取消，在一秒验证窗口内结束；Runtime 的 Tab 数量、活动 Tab 与 Tauri
  实际 native WebView label 集合均恢复原状，随后可再次正常创建。临时 profile 清理仍通过。
- 输出包含 `cancelled_creation_closes_native_candidate` 与 `creation_recovers_after_cancel`；13 项 example
  单测和桌面编译通过。关闭失败分支的保留策略已实现，但本切片没有模拟 COM 关闭失败，不将其当作已实测。
- 创建阶段仍持有 registry 的部分等待、正式 popup 消费与完整原生 conformance 继续保留在原目标中。

第二十五个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --runtime-locks-only`：exit 0。
  新 Tab 正等待延迟响应时，snapshot 能看到被保留的候选，原有 native surface 在 250ms 验证窗口内隐藏；
  取消仍清理候选并可继续创建。另一个用例在创建导航期间关闭 Runtime，创建返回 WorkspaceClosed，之后
  Runtime/profile 清理成功。输出包含 `creation_does_not_block_hide`、`close_settles_creation_before_cleanup`。
- 13 项 example 单测与桌面编译通过。该验证覆盖正常回调与关闭取消，不替代全部崩溃、进程退出和强制中止
  创建 worker 的恢复矩阵。正式 popup 消费和主应用 UI 验收仍未完成。

第二十六个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --managed-popup-only`：exit 0。
  通过真实 BrowserWorkspaceService/run guard 驱动点击，无测试代绑；popup 进入同一会话并成为活动 Tab，
  opener、原始 WindowProxy 与 Cookie 保持；Agent 能继续观察/点击新页，新页保持运行锁，finish_run 后恢复用户操作。
  空闲用户的原生点击可打开新 Tab；8-tab 配额达到后，popup 不增加 Runtime 记录或 native WebView。
  运行中没有对应 Agent 输入操作的原生 popup 手势被拒绝。
- `--popup-only`：exit 0；新增旧操作取消后换入新操作的情形，旧请求仍不能绑定，未认领 child 被关闭。
  原生 create/bind 入口和消费器都复查取消；事件的取消/关闭信号会结束 native deferral，不只是取消 Rust 等待。
- 13 项 example 单测、桌面编译和 browser boundary 检查通过。编译剩余提示仅涉及专用 transport 测试入口，
  不再是正式 popup 创建链未使用。默认完整 smoke 仍到跨进程拖放处失败，未删掉该要求。
- 这是正式后端接线及原生业务验证，不等于已完成真实主应用视觉验收。窗口名称复用、self.close、全部 Stop/创建
  交错和关闭失败矩阵，以及 macOS 对等实现仍须继续验证；不要把这些缺口隐含为已通过。

第二十七个切片的验证（2026-09-14）：

- `cargo test -p nomifun-browser-platform --lib url_projection`：11 passed，含迁移的九项测试及 IPv6/映射地址、
  长度限制新用例。旧 app router 模块物理移除，新模块被实际工具输出调用，不再只保留孤立测试。
- `cargo test --profile dev -p nomifun-ai-agent --features browser-use --lib manager::nomi::browser_tool::tests -- --nocapture`：
  6 passed，验证 query/fragment 不进入模型 Tab URL，原始 Runtime URL 与 target 不变，缺少 observe 能力时
  仍只返回许可的 target 元数据。
- 新增平台 url 依赖后，按 `target_inventory write` → `agent-v2-contract write` 重建派生数据，两项 check 均通过；
  browser platform boundary 通过。本切片未修改浏览器导航或 renderer 地址栏实现。
- 本改动只保护 URL 元数据字段，不声称过滤了页面任意文本、标题或路径中所有可能的敏感信息；页面内容的
  不可信标记及其他脱敏规则仍独立存在。主应用视觉、macOS 和当前完整原生拖放失败没有被此项检查替代。

第二十八个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --managed-popup-only`：exit 0。
  用户空闲点击和 Agent 点击触发的 window.close 都删除了对应 Runtime/native WebView 记录，剩余标签恢复活动，
  主应用窗口仍存在，opener 的原内存 nonce 不变，之后仍可创建标签并完成配额测试和 profile 清理。
- 初版只等关闭通知而在事件通道结束时直接退出，真实测试复现了“页面已不可调用但 Tab 仍保留”的竞态；
  修正后通过。没有仅凭列表删除来声称原生控制器已经关闭。
- 13 项 example 单测与桌面编译通过。该验证不替代浏览器崩溃/强制进程退出及全部关闭失败矩阵。

第二十九个切片的验证（2026-09-14）：

- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --managed-popup-only`：exit 0。
  Agent 点击真实 SPA 控件后，完整 URL（含 query/fragment）和历史按钮状态通过 native events 更新；后退、
  前进再后退均正确，原 document generation 和内存 nonce 保持不变。该 URL 是 UI 状态，模型元数据仍使用
  第二十七个切片的安全投影。
- 13 项 example 单测与桌面编译通过。未新增 renderer 布局规则或 macOS 证据；完整 conformance 的其余
  未解决项不因历史元数据正确而视为完成。

第三十个切片的实现与验证（2026-09-14）：

- iframe 二维映射现可组合独立 rotate/scale/translate 与 transform，包含百分比、非均匀及负数缩放。
  按 CSS 变换顺序组合线性矩阵，平移仍由真实 bounding rect 校正；保留各层命中、稳定性及退化矩阵检查。
  非平面旋转、非单位 Z scale、非零 Z translate、perspective 与尚未处理的 motion path 明确返回 unsupported。
- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --frame-input-only`：exit 0。
  真实嵌套 iframe 在两组独立属性和原 transform 叠加后均收到 trusted click，其中一组为镜像缩放。
  原输入、父遮挡/焦点、导航失效与 perspective 拒绝检查同时通过，profile 清理完成。
- 13 项 example 单测、桌面编译通过。变换顺序对照
  [CSS Transforms Level 2 的工作草案](https://drafts.csswg.org/css-transforms-2/#ctm)，实际支持范围以 WebView2
  运行证据为准，不据此声明支持全部三维布局。默认完整 smoke 仍在既有跨进程拖放 dragend 检查处失败。

第三十一个切片的实现与验证（2026-09-14）：

- Windows 普通 Tab 与 popup 在加载页面前安装原生 PermissionRequested 默认拒绝策略，未知权限种类也拒绝。
  运行时支持相应接口时禁止把策略拒绝存入 Profile；关闭 Tab 时注销事件。去重后的权限名称进入 Tab 状态，
  触发 revision，跨 document 导航清空记录。此记录尚无用户授权 UI，不代表权限产品流程完成。
- `cargo run -p nomifun-desktop --example browser_workspace_smoke -- --managed-popup-only`：exit 0。
  Agent 真实点击定位按钮后，页面收到 denied-1，会话状态包含 geolocation；输出
  `native_permission_denied:true`，最后完成原生 fixture Profile 清理。
- 14 项 example 单测、`cargo check -p nomifun-desktop`、UI typecheck 与 desktop UI boundary 检查通过。
  桌面编译仍有专用 conformance 入口未使用提示。摄像头、麦克风、MIDI、USB/serial、剪贴板等未逐项完成真实验证；
  PermissionRequested 不等于覆盖键盘复制/粘贴。UserReady 授权交互、完整权限矩阵仍待完成。

第三十二个切片的实现与验证（2026-09-14）：

- ChatLayout 为支持原生浏览器的会话提供局部 BrowserLinkContext。用户点击聊天中的 HTTP(S) loopback 链接时，
  打开所属会话 Browser 面板，通过原有用户 command 创建 Tab；同 URL 已存在则激活原 Tab，不重载其页面状态。
  不通过全局 URL 事件或当前路由猜测会话归属。WebUI、伙伴独立表面和没有会话 owner 的内容保留外部打开。
- `0.0.0.0` / `[::]` 显式链接映射为相应 loopback，面板说明原监听地址；URL query/fragment 保持用于导航。
  非 Web URL、credentials、相似域名等不会被识别为本地预览。这只是用户明确点击，不是可信 endpoint 自动发现，
  不授予 Agent 额外网络权限，也不扫描端口或读取任意 stdout 推断服务。
- AgentRunning、input gate failure 或浏览器忙时不执行用户链接命令，不在停止后重放；需用户重新点击。
  服务端运行锁仍复查请求。链接命令失败显示面板内提示，保留现有页面；卸载也使迟到的命令 UI 结果失效。
- Xterm WebLinksAddon 使用同一局部上下文和最新回调，不因回调更新重启 PTY。当前独立 TerminalSessionPage 没有
  Conversation owner，仍外部打开；会话 Bottom Dock 尚未接入，不能把公共组件接线当作整个 Terminal 闭环完成。
- 六个测试文件共 34 passed：面板 command/运行锁/失效回调，URL 分类，真实 Markdown Shadow DOM 链接点击与
  两个会话上下文隔离，实际 xterm buffer + WebLinksAddon 解析/点击，以及原 PTY 与 ChatLayout 回归。
  测试期间仅有 KaTeX quirks-mode 环境提示。UI typecheck、desktop UI boundary、browser platform boundary 通过；
  i18n 类型按新增中英文提示重新生成后，i18n parity/type gate 通过。
- 本切片没有运行完整主应用视觉/原生点击 E2E；默认 native smoke 的跨进程拖放失败仍待解决。

第三十三个切片的实现与验证（2026-09-14）：

- Browser 导航栏新增终端展开入口，在真实页面下方挂载可收起的 Terminal Dock，两者同时显示。
  Dock 复用 XtermView 与原 TerminalService PTY，不新建终端运行时，不创建伪终端或独立终端记录；
  展开/收起只改变浏览器 Surface 的测量边界，不重新 attach 浏览器或执行页面命令。
- `useTerminalSessions` 扩展为显式 owner scope：全局侧栏仍读取 standalone 列表，Dock 使用当前 Conversation
  的列表 API。snapshot 和生命周期 event 都按服务端 owner_conversation_id 过滤；切换 scope 后旧事件回调及
  迟到 GET 不污染新会话。订阅/重连/在途事件合并复用同一 hook，不为 Dock 另建全局状态或轮询器。
- Dock 可选择已有会话终端、刷新列表和重试连接；新终端事件不改变当前选中的 PTY。XtermView 增加显式
  autoFocus 控制，Dock 挂载不抢走 Chat/native page 焦点，点击后仍可交互；退出的终端只回放，不调用输入或 resize。
  Dock 内 localhost 链接继承第三十二个切片的 Conversation context，仍受 Browser 运行锁约束。
- 六个文件共 52 tests passed：包含原 standalone TerminalSessionPage/useTerminalSessions/XtermView 回归，
  新 owner scope、选择保持、退出/错误重试、挂载不抢焦点及 browser surface 边界更新检查。
  Dock 组件测试替换了 emulator 渲染边界；独立 Xterm 测试覆盖实际组件的 PTY bridge，不能视为主应用原生 E2E。
  UI typecheck、desktop UI boundary、browser platform boundary 和 i18n gate 通过，新增九个中英文 key 后重建类型。
- 本轮检查确认 Process/Terminal 尚无 LocalEndpointDiscovered。未把用户输出中的任意 URL 提升为可信服务，
  没有前端端口扫描或日志 URL 猜测。可信服务发现、Process 工具会话与 TerminalService 的对接、用户新建会话
  终端入口、其余三个 Dock tab 以及真实主应用视觉/原生交互仍待完成。

第三十四个切片的实现与验证（2026-09-14）：

- Windows 普通 Tab 与 popup 在加载前订阅自身 root WebView2 session 的 Console、exception 与 Network 摘要事件。
  事件输入使用 32 项队列、单项 32K UTF-16 上限；worker 批量消费并合并 revision 通知。每页仅保留 32 条记录，
  message/source URL 各最多 512 字符，进行中的请求关联最多 128 项；超限/无法关联事件计入 dropped，关闭视图注销
  原生事件并取消消费器。没有开放调试端口，也不枚举全局 target。
- Network 只投影错误和 HTTP >= 400 的摘要，不保存 headers/body；source_url 使用 v2 URL 安全投影去掉 credentials、
  query 与 fragment。Console 只读取协议自带的有界预览，不跟随 objectId 或求值 getter。任意页面 message 本身仍是
  不可信页面内容，不能据 URL 元数据规则声称过滤了所有可能的文字秘密。
- Browser Tool 新增受 browser.observe 约束的 diagnostics 读取，明确选择当前/指定 Tab，并标记 untrusted_page_content
  与 root-only coverage；普通 tabs/navigation 输出不自动附带全体日志。无 observe 能力不能读取，不接受 expression
  等求值参数。运行归属继续来自既有 turn guard，没有新权限旁路。
- 底部工具区增加 Console/Problems；前者显示 Console，后者显示页面异常、console error 和网络错误。使用 React 文本
  渲染，不解释日志 HTML；切换活动页更换记录，显示 dropped 与跨进程 frame 尚未覆盖提示。不新增日志数据库或第二份
  Agent 执行日志。
- `--managed-popup-only` 真实 WebView2 smoke 两次通过；最终输出包含 native_diagnostics、diagnostics_no_getter_evaluation
  和 native_fixture_profile_cleanup。Agent 真实点击控件触发 console.error、异步页面异常、禁止端口请求失败；原生
  状态收到三类记录，getter 计数保持 0，结构化 source_url 没有 query token，原 popup/运行锁/权限等检查仍通过。
- 18 项原生 example 单测、7 项 Browser Tool 单测和 18 项 Browser/Dock UI 回归通过；UI typecheck、desktop UI boundary、
  browser platform boundary 与 i18n gate 通过，桌面主程序 cargo check 通过（仍有既有 conformance 专用入口未使用提示）。
  projection 测试覆盖队列处理时的旧 document generation、文本/ring/request
  上限和 URL 投影；不把这些单测视为完整跨导航、execution context、loader、崩溃或 OOPIF 诊断验收。
- OOPIF/worker 日志、完整 navigation attribution、日志限流的压力验收与主应用原生视觉仍待补齐；当前 UI/Tool 明示 root
  session 的支持范围。Test Steps 尚未实现，默认完整 native conformance 的跨进程拖放失败仍保留。

第三十五个切片的实现与验证（2026-09-14）：

- 底部工具区增加 Test Steps，直接投影已加载的 canonical Browser tool_call 消息，不另订阅工具事件、不复制消息
  store、不建执行日志数据库。NomiChat 的既有 MessageListProvider 通过 Conversation-scoped Portal 渲染到 Dock；
  Context 只持有 DOM mount 与 source 可用状态，不持有工具数据。聊天来源暂未挂载时提示切回聊天，而非显示假进度。
- 仅接受同会话、非 hidden、精确名为 Browser 的记录；操作来自 canonical args，不用 display input 覆盖执行参数。
  记录键含 turn + call identity，跨 turn 复用 call id 不混合。显示操作、元素 ref/脱敏 URL、工具返回时页面状态、
  错误代码及跳回原消息入口，不复制键入文本、整页输出或任意输入字段；结果解析上限 64K，显示最近 100 条已加载记录。
- 状态复用原工具记录规则，明确的 BROWSER_OPERATION_CANCELLED 错误显示取消；没有根据 Browser UserReady/Agent
  停止推断工具成功或取消。重试复用现有 ExplicitToolRetryReceiptIndex，缺根、缺步、跨 turn 的链不生成重试标记。
  界面说明“操作完成不代表测试断言通过”，页面 URL/load state 明确是工具返回时的快照，不宣称导航最终完成。
- 六个文件联合回归共 67 passed。Portal 测试经真实 useAddOrUpdateMessage 入口验证 running → completed → error
  修正、迟到成功不可覆盖错误、另一会话隔离、收起重开及原消息跳转；并回归已有消息 hydrate/merge 与 Browser/Dock。
  UI typecheck、desktop UI boundary、browser platform boundary、i18n gate 与 diff whitespace 检查通过。
  实现时发现 Windows 模块解析中 model/component 同基名仅大小写不同会冲突，已将 model 命名为 browserTestStepsModel，
  没有保留旧导入别名。
- 本轮未做主应用原生视觉验收。明确断言、PNG evidence receipt、导航最终完成证明及具体 console/error 引用仍须随
  生产 Browser Tool 的相应能力补齐，再投影进 Test Steps；不能将目前的工具调用过程视图当作整套前端测试闭环已交付。

第三十六个切片：真实桌面检查与范围纠正（2026-09-14）：

- 在新建的隔离数据目录启动真实 nomifun-desktop + Vite，通过正常本地 API 创建未选择模型的空会话，未调用外部模型。
  在主应用内实际打开本机 fixture，验证了中文输入、按钮响应及弹层关闭后页面内容仍在。启动检查曾发现 Dock 图标别名
  导入不受 Vite 插件支持；该 Dock 随本次用户范围纠正已整体删除。Popover 的 role=tooltip 遮挡遗漏已修复并保留回归测试。
- 用户明确纠正：前端测试只是 Agent 内部打开、观察和模拟操作真实浏览器的能力，不是要建设终端/控制台/问题/测试步骤
  工作台。ADR 已同步删除底部区域及专用测试、元素评论、服务发现产品等要求，不保留隐藏开关或旧入口。
- 物理删除 10 个专用组件、样式和测试文件，移除 ChatLayout/NomiChat 的 Portal 接线，恢复既有独立终端的四个源文件/测试，
  删除底部区域专用文案并重建 i18n 类型。正常会话记录、已有独立终端、普通浏览器导航和 Agent 原生输入能力不受影响。
- 内部诊断只保留给 Agent 按需观察：BrowserTabSnapshot 序列化不再带 diagnostics，原生诊断事件不再触发 renderer revision。
  非空诊断不进入 UI 快照的测试已通过；保留的 Agent diagnostics 不构成用户产品面板。
- 删除后 36 项前端回归、18 项原生 example 单测、UI typecheck、desktop UI boundary、icon imports、i18n gate 与空白检查通过。
  这些证据不代表完整 Windows 交付；最小窗口、完整 Agent 主应用联调及其他未完成底层能力继续按修订后的 ADR 验收。

第三十七个切片：Agent 自动打开及隐藏输入边界（2026-09-14）：

- 原生 Runtime 原来仅在 Tab command 后发送一次自动打开通知。现改为每轮首次成功 observe/act/command 按需请求
  展示；已显示时不再请求，同轮收起后不反复展开，下一轮可再次打开同一实例。库存读取、失败或取消的观察不触发展示。
  原生事件仍仅发送到 main，用户无需测试模式或额外产品入口。
- Agent 操作前对自身 WebView 使用 Emulation.setFocusEmulationEnabled；run settle/finish 撤销，失败保留清理责任，
  不改变系统窗口焦点或制造 DOM 输入。此为暂存的运行时实现，不是全部后台输入 conformance 已达标。
- 前一轮未中断前的 presentation fixture 验证了自动打开次数、同轮隐藏保持、实际 IsVisible=false 时 trusted click、
  同一 popupNonce 和 run 后恢复 document.hidden；managed-popup-only 及 18 项 example 单测通过。本轮重新运行
  runtime-locks-only 也通过，覆盖 hide/resize/creation/navigation 与取消清理。
- 本轮增加连续中文输入、End、Backspace 的真实要求后，`--presentation-only` 确定失败于 type。页面 rect 在 viewport
  内，elementFromPoint 命中原 input，document.hidden=false 且 hasFocus=true。额外语义稳定性探针曾超时；最终仅保留
  双 requestAnimationFrame 只读探针时又返回 frames-running，故调度时序相关但根因尚未完全证明，不把“重复 hide”
  当作已确诊原因。仅焦点激活不足以通过连续输入；后续按键断言尚未执行，不能写为通过。
- 排查尝试了跳过重复 native hide/show 以及再次发送 enabled=true，均未解决，已删除这些实验性改动；没有留下额外
  visibility helper、替代语义内核、DOM 点击或跳过稳定性校验。保留真实失败 fixture 及只读几何/调度诊断，默认完整矩阵
  也包含此用例。既有跨进程拖放 source dragend 问题仍未解决，且可能被这个较早失败的用例阻止执行。
- 18 项 example 单测和主程序 cargo check 通过；不以编译/单测或此前单次点击通过覆盖本次原生失败。
  下一步需解决 WebView2 隐藏状态下的持续调度与真实输入，并回归页面焦点、权限、弹窗和取消边界。

第三十八个切片：修复隐藏页面的连续输入（2026-09-14）：

- 在生产 LOCATE 的临时诊断中确认 missingState=stable 且 timedOut=true；输入框几何可见、命中正确，失败来自
  两秒内未获得足够稳定性动画帧。临时错误打印与诊断字段已删除，不遗留页面数据调试输出。
- 为 Native Interactive 的 isolated semantic core 增加有界几何观察调度，不修改 vendored Playwright bundle。
  动画帧和 20ms 计时器竞争唤醒，每次采样至少间隔 15ms，三次相同几何才通过；移动、节点断开/替换、非有限几何
  或两秒截止立即拒绝。沿用 upstream 的 visible/enabled/editable 检查且在采样前后都执行；element 与父 iframe
  坐标映射使用同一 helper，保留最终命中检查。没有 DOM click/fill/event 兜底，没有显示隐藏的原生控件。
- `--presentation-only` 两次通过，最终版本增加移动 input 的负例。实际 IsVisible=false 下连续完成 trusted click、
  中文 type、End、Backspace；value 与可信事件检查通过，运动中的 input 拒绝且未写入，run 结束后撤销临时页面激活，
  原实例 nonce 保留并清理 profile。
- 独立调度测试 8 passed：动画帧正常/不回调、移动、断开、替换、不可编辑、截止及采样后 enabled 变化，检查所有
  pending callbacks 清空；已接入 `test:browser` 的现有 runner。`--frame-input-only` 和 `--managed-popup-only`、
  `--html-drag-only` 的真实原生回归通过（含同进程/OOPIF/nested 输入与父遮挡、popup/权限、同页拖放及取消）。
- 这是 Agent 底层稳定性与输入修复，无 UI 新增。Windows 完整验收、跨进程 HTML 拖放和其他交付清单仍独立追踪。
- 重新运行默认完整 smoke：已越过隐藏输入检查，最终仍因既有跨进程拖放 source dragend 缺失而 exit 1，未删掉该失败。
  18 项 example 单测通过；该失败不能因定向回归为绿而视为整体 conformance 已完成。

第三十九个切片：跨进程拖放取消清理（2026-09-14）：

- 核对 Chromium InputHandler 的公开实现后，发现 DragController 的取消/结束调用使用当前拖入的 widget，未独立
  记录源 widget。此为源码线索，不将 main 分支等同于本机 Edge 的确切二进制。现有完整拖放失败仍需保留。
- NativeTab 的单次拖放现在保留源端原生会话和该会话内坐标。取消时先执行原 root Input.cancelDragging，再向
  原源端会话执行 native Input.dispatchDragEvent(dragCancel)。源会话仅来自已验证 iframe 路由，不接受任意 session；
  同进程祖先坐标逐层变换到源 widget，跨 OOPIF 边界停止。取消数据为空，不读取或合成 DataTransfer，不发 DOM 事件。
- 两项取消命令成功前保留 dragging/source 清理责任；实际按下前再次核对源位置和取消信号。成功 drop 路径不调用
  源端取消。发生部分 drop 后的清理仍返回 ActionInterrupted，不因为取消产生 dragend 就报告成功。
- 新增 `--frame-drag-cancel-only`（同时纳入默认矩阵）：真实跨 iframe 到达目标 dragover 后 Stop，源页面得到 trusted
  dragend/dropEffect=none，目标 drops 为空，profile 清理完成。原同页拖放、Pointer Capture、取消/替换负例回归通过。
- 默认完整 smoke 仍 exit 1：正常跨进程 drop 已发生，但缺少协商 move 的成功源端结束；新增清理能让源端随后收到
  none 的取消结束。测试额外要求成功 dragend 的 dropEffect=move，确保不会把这种部分成功/取消清理误当完整成功。
- 18 项 example 单测、桌面 cargo check、browser platform boundary 与 diff whitespace 检查通过。本切片只修复取消
  收尾，不声称正常跨进程 HTML 拖放已经完成，也不新增 UI。

第四十个切片：生产 Browser Tool 与真实原生页面联调（2026-09-14）：

- 新增 `--tool-only` 原生 example，用测试模块直接编译生产 browser_tool/browser_lifecycle 源文件，连接实际
  DesktopBrowserHost 与 BrowserWorkspaceService，不复制工具实现或把私有构造入口暴露成生产 API。
  为 example 增加明确的 dev-dependencies，不改变桌面生产依赖边界。
- 通过生产 ToolRegistry 注册并校验 JSON schema，再执行 navigate → observe → type → fresh observe → click → diagnostics。
  真实页面收到 trusted 中文输入，原始 nonce 与 Tab 保持；诊断包含原生 console 事件及不可信标记。
- 验证了未运行/已结束调用拒绝、未选择能力拒绝、伪造与重复旧引用拒绝、运行中用户导航拒绝；settle 后仍为
  AgentRunning，finish 后为 UserReady，旧 turn 句柄不能加入下一轮。下一轮 observe 仍返回同一个原生页面。
- `--tool-only` 两次通过，最终版本包含注册表 schema 校验与原生 Profile 清理。example 单测现为 25 passed，
  含编入的生产 Browser Tool 原有 7 项测试。没有新增 UI 或专用用户测试流程。
- dev-dependencies 改动使 Cargo.lock 摘要变化；target_inventory check 通过，按 agent-v2-contract write 更新
  runtime release fixture 与派生 envelopes 后 check 通过，没有手工伪造摘要。
- 本用例使用受控的 capability snapshot/provider 标识，未执行实际模型、Kernel resolver、工作台能力选择与 NomiManager
  全链。它填补“工具协议 ↔ 原生驱动”的证据缺口，不等同于整套 Agent/主应用闭环交付；该部分仍需继续验证。
- 默认完整 smoke 已包含该联调并运行到既有跨进程拖放失败，exit 1；browser platform boundary 与空白检查通过。

第四十一个切片：Agent 按需观察真实页面像素（2026-09-14）：

- Browser Tool 增加 screenshot 操作，沿用 browser.observe 能力、当前 turn guard 和每页操作锁；返回单张 viewport PNG
  到既有 ToolImage 观察上下文，不写入 renderer DTO、不增加 UI、不创建用户截图产物。工具说明明确这是页面数据，
  不是图片流，也不授权坐标操作；下一动作仍使用 fresh observe 的精确元素引用。
- 截图绑定当前 Tab/runtime/document target，前后读取 viewport metrics 并核对 target，几何/文档变化拒绝交付；
  PNG/base64 与像素尺寸有界，当前返回图像最大边 1600、base64 最大 3MiB，超限拒绝而非扩大已有协议上限。
  只缩放捕获输出，不改变页面 layout；高 DPI/缩放、导航/崩溃等完整矩阵仍待验收。
- 初版隐藏 WebView 的 Page.captureScreenshot 等待绘制导致 smoke 超时，未将其记为通过。最终使用 WebView2
  Controller.IsVisible 临时开启绘制，保持原 child HWND 隐藏、不调用窗口 show/focus；截图完成后恢复绘制并重新应用
  最新 Surface 状态。截图期间的 hide 只隐藏 HWND，不切断待完成的绘制；visible resize 仍受页面锁保护。
- 实际 `--tool-only` 校验 ToolRegistry 接受 screenshot，解码 PNG 后同时找到 Canvas 红/蓝色块，尺寸与 metadata 一致，
  原生 HWND 始终隐藏；页面繁忙时截图与 hide 并发，hide 在 200ms 验证窗口内完成，之后保持隐藏；取消时不返回图片，
  compositor 状态恢复。生产工具/原生页面完整定向 smoke 与 profile cleanup 通过。
- 27 项 example 单测通过；runtime-locks-only 回归通过。新增 image 仅是 example 的 PNG 解码测试依赖，Cargo.lock
  更新后重建 runtime release fixture/envelopes，agent-v2-contract check 通过。没有图片库加入生产截图实现。
- 本切片不声称像素中的任意敏感文字已被自动识别/遮盖；这是 Agent 明确请求的可见页面观察，并标记为不可信内容。
  主应用/实际模型图像上下文链、完整截图并发/异常矩阵和既有跨进程拖放失败继续保留在后续交付范围。

第四十二个切片：截图像素密度、缩放与滚动边界（2026-09-14）：

- 新增受控的 1/2/3 倍 deviceScaleFactor 原生渲染测试后复现：原缩放仅按 CSS 大小计算，2 倍密度会超出 1600 像素
  上限。截图现在在专用 isolated world 读取实际 devicePixelRatio，按物理像素预算缩小输出；不接受页面或模型提供的值。
  页面故意覆写同名 getter 为 0 的负例验证：getter 未被截图执行，PNG 仍正确。
- 原生 Controller.ZoomFactor 的 80%/125%/200% 测试又复现裁剪错误：CDP clip 使用缩放后的 viewport 单位，直接用
  cssVisualViewport 的宽高会漏掉右边 Canvas。现按 metrics 中的 zoom 换算 clip 的位置和大小，而像素预算使用已含
  zoom 的 DPR，避免重复计入。截图前后同时复查 geometry/zoom/DPR；变化时拒绝交付旧画面。
- `--tool-only` 最终通过：1/2/3 倍模拟密度、真实浏览器 ZoomFactor 三档、纵向滚动后的固定 Canvas 色块均可解码验证，
  图像不超过上限；页面尺寸、DPR、zoom、scrollY 和 nonce 保持。测试等待 native zoom 实际到达页面后再记录基线，
  不把异步设置尚未完成造成的变化归咎于截图。该证据不等同于实际多显示器 DPI 切换已验收。
- 28 项 example 单测、桌面 cargo check 通过，原有 hide/cancel/tool 断言仍通过。没有改变 Browser UI、没有全页图片流，
  未改页面布局来适配图片尺寸；全部原生/模型联调和现有跨进程拖放问题仍独立待交付。

第四十三个切片：清理 Agent 应用层旧 Browser 配置传递（2026-09-14）：

- 删除 NomiResolvedConfig 的 browser_source/full_power/persistent_login/site_memory/visual_fallback 五个旧字段，
  同步移除工厂常量、默认值变量、健康检查和测试构造处的传递。不加字段别名或迁移，不读取旧用户偏好。
- 应用 manager 现在完整替换 legacy BrowserConfig，不能由项目/全局 TOML 的 enabled=true 重新启用应用已禁用的
  旧 Browser adapter。原生 Workspace 固定禁用旧 adapter；尚未退役的后台 Hub adapter 只使用固定 headless/default-deny
  策略。原生路径不再把旧 persistent-login key 交给 bootstrap。
- NomiHostWiring 同时带 Native Workspace 和 legacy Lane 时，在 bootstrap 前明确拒绝，不允许两个 owner 接入
  同一个 Agent。原生 Browser 的 exact capability 注册逻辑未放宽。
- `native_browser` 定向生命周期测试 9 passed，覆盖配置/双 owner 新用例、模型工具路由、成功/失败/取消终态、旧 turn
  拒绝及 Stop 与原生锁交错。`cargo check -p nomifun-ai-agent --no-default-features` 与桌面 cargo check 通过；
  agent_types_integration 使用 browser-use,test-support 完成 no-run 编译，未将其报告为运行通过。
- browser platform boundary 新增旧配置字段防回归规则及自测，检查通过；五个旧字段在 backend Rust 中无剩余引用，
  diff whitespace 检查通过。没有 UI 改动。
- 此次只清理应用层转发和装配歧义，不是后台 Hub、Lane、vault 及独立 nomi-config/nomi-browser 核心的整体替换；
  这些剩余组件继续列入后续清理，不因上层字段删除而声称“没有任何历史债务”。

第四十四个切片：Windows 原生渲染进程退出与显式恢复（2026-09-14）：

- 普通标签与 popup 在导航前安装 WebView2 ProcessFailed 监听，关闭时移除。主文档进程退出使旧引用失效，
  保留标签身份；不自动刷新、不重放动作，也不因崩溃自动解锁。
- 原生协议等待按 view 管理，在确认整个页面进程退出或 controller 关闭时结束等待；迟到回调不能重复完成。
  已销毁主文档的输入状态在 Agent 收尾时清除，不向退出的渲染进程发送无法完成的清理命令。
- 隔离 fixture 的 `--crash-only` 真实故障注入通过：旧引用拒绝、运行锁保留、显式刷新恢复同一标签、
  新文档 trusted input，以及临时 profile 清理。没有向用户应用或浏览器进程注入故障。
- 此证据仅覆盖主渲染进程退出，不代表 browser process 重建、OOPIF 故障、GPU 或无响应恢复已验收。
  默认完整 smoke 的跨进程 iframe 拖放问题仍未解决。
- 示例定向单元测试 31 passed，包括退出类型区分、按标签结束等待及迟到回调保护；
  子 frame 退出不作为整个主页面输入状态已销毁的证明。没有 renderer UI 修改。
- 再次确认用户范围：自动打开浏览器、观察和模拟鼠标键盘属于 Agent 内部能力，不新增测试产品、
  测试配置或底部工具面板。上述 fixture 是开发验证代码，不是用户产品功能。

第四十五个切片：Agent 指针提示的绘制与生命周期（2026-09-14）：

- 修复原有 HIGHLIGHT 每次 fresh observe 释放语义对象却未移除 DOM 圆环的问题。释放旧 world 前清除其提示，
  Agent 正常结束、失败收尾与取消共用的 settle 路径也清除根页面提示；不把提示对象变成第二套输入锁。
- 使用简洁的 SVG 鼠标箭头标示真实输入的根 viewport 坐标，closed shadow 隔离内部样式；
  pointer-events:none、aria-hidden，不抢焦点、不创建合成输入、不增加任何测试 UI。
- `--presentation-only` 真实 WebView2 验证通过：提示存在但命中仍是原按钮，主 world 无内部指针引用，
  新观察/结束/取消后没有遗留节点。通过 Native Screenshot 的像素检查确认实际 compositor 在相应坐标绘制箭头，
  不是仅凭 DOM 样式推断。截图仅作内部验收，没有改为截图式 Browser Surface。
- `--frame-input-only` 通过，覆盖同进程、跨进程和嵌套 frame 的 trusted click/中文输入/按键/select、
  二维变换与遮挡/焦点防护。desktop UI boundary 与 browser platform boundary 均通过。
- 尚未据此宣称完整视觉/平台验收通过；真实主应用完整 Agent 闭环、其余权限/文件交互和旧后台 owner 退役
  仍在后续清单，跨进程 HTML 拖放失败也不由指针提示修复。
- 本轮重跑默认完整 native smoke：前置输入、frame、同页拖放、Workspace/锁、popup、presentation、Tool、
  崩溃与跨 frame 取消检查均执行至通过，最终在跨进程拖放失败并 exit 1。Edge 152.0.4191.66 中目标收到
  trusted drop 与原 DataTransfer，源端仅在失败清理后得到 dropEffect:none 的 dragend，不符合成功 move 要求。
  完整 smoke 保持红灯；未改弱验收或增加合成事件兜底。

第四十六个切片：跨进程拖放接口核实与命名窗口验证（2026-09-14）：

- 新增 `--frame-drag-only` 快速复现现有真实失败，不跳过或放宽默认矩阵。失败证据附带对同一 native controller
  的只读 CompositionController QueryInterface 结果：本机返回 80004002，不能通过 cast 得到 SendMouseInput。
- 核实 Chromium 上游两条 drag 路径的 source-end 路由；记录在 ADR Windows 输入约束中。Edge 对应 revision
  未取得，未冒充精确源码归因。原生 Composition hosting 的不同创建/输入/拖放契约仅列为待验证候选，不贸然切换。
- managed popup fixture 新增 WindowProxy/文档 nonce 断言：重复同名 window.open 保持同一个原生 Tab 与 Proxy，
  导航替换文档并增加 generation；关闭该窗口后同名打开得到新 Tab/Proxy；达到 8 个 Tab 时已有命名窗口仍能复用，
  新窗口仍被拒绝。所有触发通过原生按钮输入，不增加用户产品功能。
- `--managed-popup-only` 新增矩阵通过，连同已有 popup 锁、opener 通信、自关闭、权限拒绝、诊断和临时 profile
  清理验证；`--frame-drag-only` 按预期 exit 1，明确保持未交付项。diff whitespace 检查通过，无 renderer 修改。

第四十七个切片：删除失效 Fresh-v4 Browser Role/Hub 接入（2026-09-14）：

- 复核发现旧 BrowserRoleRuntime.acquire 已无调用入口；动作/context/operation 只查找从未填充的弱引用资源表。
  启动却仍会恢复旧 profile、读取 Cookie vault，并构造一套独立 Hub。没有将这条死路径包装成 v2 owner。
- 删除 BrowserRoleRuntime、BoundBrowserRoleInvoker、旧操作翻译、资源租约续租/清理及旧 identity Chrome ignored
  测试；保留 Computer 角色适配和共享资源/Provider 校验。Fresh-v4 compose 不再创建第二套 Hub，也不读取旧
  browser-data/Profile/身份快照，统一使用 build_from_open_pool。没有数据删除或数据库迁移。
- 现有 Nomi 原生 Workspace/Tool/turn 接入不变；Fresh-v4 Browser host ports 明确未配置，后台 v2 Headless
  owner 与对应 Role 功能仍未交付。删除死代码不等同于替代能力已实现。
- browser boundary 从“两处互斥 Hub 必须各创建一次”改为仅允许剩余 services.rs 后台 Hub，明确禁止 Fresh-v4
  重建；增加旧宿主/启动符号防回归及扫描器自测。扫描器自测、实际扫描和 browser-use 单独编译检查通过。
- 同时启用 browser-use/computer-use 的角色定向单元测试 5 passed；该已构建测试二进制中的 Fresh-v4 compose
  与重启用例分别 1 passed（compose 断言不创建旧 browser-data）。`--tool-only` 真实 WebView2 回归通过，覆盖
  生产 Tool JSON、同页 trusted 输入、诊断、截图/DPI/zoom、取消清理及 turn 锁。没有主应用全量或完整平台通过声明。
  链接仅见既有 opusic-sys PDB 缺失警告，未阻止测试执行；diff whitespace 检查通过。

第四十八个切片：网站权限的用户入口与原生请求所有权（2026-09-14，仍有未通过项）：

- Windows PermissionRequested 使用原生 deferral，最多保留每 Tab 4 个请求，30 秒超时默认拒绝；原生句柄只在
  UI 线程持有。仅可见、活动且 UserReady 的标签可由用户响应，决策绑定 exact Tab target 与一次性 request id。
  请求 URI 只投影为 origin；未知权限默认拒绝，不缓存到旧 Profile 或数据库。
- 会话浏览器增加普通网站权限提示（来源、权限、拒绝/仅本次允许），不新增全局设置、测试流程或工具面板。
  AgentRunning 隐藏决策入口；Workspace agent_command 明确拒绝 Permission，不能由模型代用户授权。
- 隐藏/切换 surface、导航、关闭、进程退出、Agent 输入锁与请求超时接入请求清理。隐藏时即使权限清理报错，
  仍先隐藏真实 native HWND，不能让网页覆盖 modal。过期用户响应只显示普通提示，不把现有页面替换成错误页。
- 前端 18 项测试通过（使用 setup-dom preload）；UI typecheck、国际化检查、desktop UI boundary 和 browser
  boundary 通过。原生权限 helper 2 项单元测试通过。首次 cargo check 在 nomifun-app 遇 rustc 栈溢出；
  随后的实际 dev 构建及测试构建成功，未通过修改编译器栈配置掩盖问题。
- 复用该已构建示例测试二进制执行全部单元用例，32 passed；这不包含上面仍失败的真实隐藏/显示权限场景。
- `--permissions-only` 的真实证据已覆盖允许/拒绝、授权后再次请求、错误/复用 id 拒绝、Agent 开始时拒绝未决请求、
  Agent 无法授权、Agent 请求直接拒绝、导航使旧请求失效、带未决权限关闭 native Tab。允许位置访问的唯一调用使用
  模拟位置，后续调用在权限处拒绝，不读取真实位置或设备。`--permission-timeout-only` 完整通过并清理临时 profile。
- **仍失败**：隐藏前原生拒绝与 Complete 均成功、请求从 snapshot 移除且不能再授权，但隐藏再显示后原页面
  geolocation 回调仍无结果。排除了仅仅等待页面可见，以及持续位置模拟造成干扰；没有放宽失败断言或伪造页面回调。
  该权限用例已加入默认 native smoke，因此当前权限能力不能宣布完成。真实主应用视觉/交互和其余权限种类也未验收。

第四十九个切片：权限自动重发的取消语义（2026-09-14）：

- 扩充原生错误证据后确认，第四十八个切片的“回调卡住”实际上是 WebView2 恢复显示后再次发起同源同类型请求，
  产生了新的 request id。该重发仍带 IsUserInitiated=true，不能凭这个标志认为用户已主动重试。
- 试验的渲染器往返/延后 IsVisible 方案只在快速 hide/show 时有效；等待真正隐藏后仍失败，已完全删除该方案及
  探针日志，没有留下额外绘制租约、延迟任务或控制状态。
- 自动取消按当前文档、origin 与权限种类记忆拒绝，最多 64 项；达到上限时该文档的新请求保守拒绝。新文档清空
  记录，不写入 Profile。明确用户的直接允许/拒绝仍仅回答该次原生请求；自动取消后浏览器栏提示刷新重新申请。
- `--permissions-only` 通过：既等待 HWND 隐藏，也等待 controller.IsVisible=false，再显示后旧位置请求收到
  denied-1；当前文档不重复弹出，刷新后有新的用户请求。保持旧 id 拒绝、Agent 不能授权、运行/导航/关闭清理证据。
- 前端测试 19 passed，包含刷新仅发送 exact target 的 reload 而不隐式 grant；UI typecheck 与 desktop UI boundary
  通过。原生权限单元用例 3 passed，新增当前文档范围和有界拒绝记录测试。
- 默认完整 native smoke 已越过新增权限矩阵，最终仍在原有跨进程拖放的成功 dragend 检查处 exit 1。
  没有放宽那条失败；其他设备权限、主应用完整视觉/交互、后台 Headless 替换等仍未交付。
- 同一示例的全部单元用例 33 passed，国际化检查通过；`--permission-timeout-only` 再次通过。导航开始时
  主动释放旧文档的拒绝记录，再跑 `--permissions-only` 通过，临时 profile 清理成功。

第五十个切片：本地网页检索的真实公网链路（2026-09-14）：

- 重验本机系统 DNS（含显式查询 1.1.1.1）仍把 www.bing.com 返回为 198.18.0.21，而普通 HTTPS 可取得搜索页。
  没有允许这个地址，也没有改系统网络设置。隔离检索显式选择固定 Google Public DNS HTTPS 路径，其他 SafeHttp
  消费者仍用原策略；这不是系统 DNS 被拒绝后的自动 fallback。
- 公开 DNS 仅接受调用方声明的引擎域名白名单，最多 8 个；非白名单域名在创建 HTTP 客户端前拒绝。DoH 使用
  固定公开 bootstrap 地址、HTTPS 证书校验、无 proxy/redirect/cookie，并设置 ECS=0。校验 Question、状态、
  截断标记、有界 CNAME 链及全部 A/AAAA；混入任何私网/保留地址即拒绝。实际搜索 HTTP 连接仍固定到验证后的地址。
- DNS 客户端与最多 32 个正向缓存条目仅由一次检索拥有，按最短 TTL（扣除 HTTP Age、上限 60 秒）过期，
  不共享到其他会话。实现指纹包括解析器源码和 Cargo.lock；工作台说明明确披露 Bing 与 Google Public DNS。
- 公网页面先从 Blocked 推进到 HTTP 200，但串行资源转发导致 30 秒 Timeout。改为最多 4 个由父任务直接持有的
  HTTP future；取消时整体丢弃，不生成脱离所有权的子任务。16 MiB 总预算使用原子预留，单响应 2 MiB 上限不变。
- 真实公网检索通过并返回 3 条自然结果与 nomi-local-search 引用；有界并发首次约 26 秒，DNS 缓存后约 13–14 秒。
  中间一次 DoH 超时明确失败，没有伪装空结果或放宽 DNS 校验。Edge probe 返回 Unavailable，未声称 Edge 已支持。
- Chrome 真实隔离/网络陷阱/取消/调用方 abort/产品绑定和并发预算合计 5 项用例通过；本地检索与冻结绑定测试
  合计 8 项（含真实公网）通过。工作台说明定向测试、desktop UI boundary、browser boundary 与契约重生成/检查通过。
- Egress 定向用例 18 passed，包含公开 DNS 白名单、私网混入、CNAME 环、TTL、私网测试开关不能覆盖公开模式、
  以及未知域名不会创建外部 DNS 客户端。没有把系统 DNS 的旧行为改成全局 DoH。
- **未交付边界**：默认 desktop main 仍未注入搜索 Provider；需完成运行时供应、启动/取消/清理装配后才能让该入口
  实际可选可用。没有把通过 provider E2E 等同于产品已交付，也没有回接旧 Browser Hub 或会话登录态。

第五十一个切片：Windows 默认桌面的本地检索运行时装配（2026-09-14）：

- Desktop main 在后端装配前，通过新的本地宿主模块发现 Program Files / Program Files (x86) / LocalAppData
  中的 Google Chrome 120+；不搜索 PATH/CWD，不读取旧 Browser 配置，不把 Edge 未通过的探测当作 fallback。
- 使用 Windows PE 版本 API 读取有界版本资源，检查返回区域/签名，再交给宿主创建文件指纹绑定。这里的签名是
  VS_FIXEDFILEINFO 结构标记，不是对发行商做 Authenticode 认证。发现与 Session 绑定不启动 Chrome；真正查询时
  仍要求 live Browser product 在网页请求前匹配。没有新增启动浏览器进程或启动失败时遗留进程的问题。
- 安装描述缺失/无效时该可选能力保持不可用；有合格描述时注入 DesktopHostServices，现有目录/工作台仍按显式
  capability selection 注册 Tool。没有给所有 Agent 默认开启本地搜索，也不依赖模型的原生 web_search 特性。
- Windows desktop 编译通过。安装发现、非 PE 拒绝、元数据与真实 product 一致、错误 product 在查询前拒绝、
  默认发现路径真实公网检索 5 项测试通过。离线构造/文件变更测试 1 passed；使用同一安装描述入口的 HTTP 目录
  测试 1 passed，断言 materialized 和 exact binding。工作台依赖说明测试与 desktop UI boundary/browser boundary 通过。
- 系统 Chrome 更新后旧绑定会拒绝，当前需要重新启动应用以重建安装描述；不在旧 Snapshot 中偷偷更换运行时。
  主应用真实 UI/Agent 完整交互验收、安装包矩阵及整体浏览器其余交付项仍待完成；macOS 保持待移交状态。
- 工作台交互 8 项通过，覆盖模型无原生搜索时本地检索独立可选、不可用项不可启用。检索组合测试中 8 项通过、
  1 项公网请求因 10 秒网络超时失败；随后单独重跑该公网用例通过（约 14 秒），不把首次失败抹成组合全绿。
  检查中只有既有 native conformance helper 未用与 opusic-sys PDB 警告；diff whitespace 检查通过。

第五十二个切片：空闲关闭的权威边界与关闭失败保护（2026-09-14）：

- 核对确认原 Nomi 工厂无论是否选择 Browser Tool 都会解析本地 Workspace，Provider 缺失时走 ensure_user；
  不新增另一套运行锁。已有纯文本 turn 用例明确断言没有注册 Browser Tool，仍在 terminal 后才解除输入锁。
- 增加仅供宿主使用的 close_idle：与 begin/finish 共用 transition/operation 锁，活动或尚未 finish 的 run 均拒绝，
  不取消该 run。接受后禁用残留原生输入，调用方 future 丢弃不丢失关闭工作；失败保留 Workspace/Runtime，允许重试，
  成功才移除注册项，后续新 Provider 使用新的 runtime generation。三项定向测试通过。
- 纠正一次调查假设：原有 detach 已能隐藏关闭失败的残余 Runtime，且对已关闭/未打开 Runtime 不会重新创建；
  删除试验性的额外关闭证明分支。新增回归测试区分“关闭失败但隐藏成功可 detach”和“隐藏也失败必须保留 attachment”，
  连同原有取消/旧 sequence 测试共 5 passed。
- 实际修正 WebView2 关闭顺序：Core controller.Close 成功前不移除 permission/popup/navigation/diagnostic 守卫，
  也不把所有待完成协议调用当作已经结束。关闭失败不等于页面已销毁。
- Nomi native-browser 定向回归 9 passed，真实 `--managed-popup-only` 通过（包含 self.close、输入锁、命名复用、
  quota 和 profile 清理）；browser boundary、desktop UI boundary 与 diff whitespace 检查通过。
- **未开放恢复 API/UI**：仅关闭 Workspace 会让缓存的 Nomi Agent 保留旧 Arc。后续需要在 Conversation 的空闲
  preparation/生命周期边界内先退役缓存实例，再关闭旧 Workspace；保留消息和项目数据，禁止复用“重置会话”来清空历史。
  不把这份基础实现当作 Provider 恢复产品入口已交付。

第五十三个切片：会话浏览器的显式恢复入口（2026-09-14）：

- 会话浏览器菜单增加“重新打开浏览器”，使用随组件生命周期卸载的确认对话框；取消不发送关闭请求，切换会话
  关闭未确认对话框。明确告知所有网页及未保存内容会关闭，不调用会话 reset，不清空消息或项目文件。
- 新增 owned Conversation DELETE browser 入口，要求 expected runtime generation。先在 preparation 边界检查
  旧请求，再进入 idle reconfiguration 生命周期栅栏；正在执行的 Runtime build 不被取消，活动/收尾 turn 也被拒绝。
  结果承载的缓存 Agent 退出成功后才调用 native close_idle，成功后新 Workspace 可以绑定新 Provider。
- 整个操作由服务拥有的任务完成，HTTP 调用方断开不释放中途栅栏。关闭失败保留 native 重试权；已关闭请求可重试，
  但旧代际请求不能关闭新实例。接口集成验证了认证、运行锁、失败重试及会话行/已存在消息内容保持不变。
- 恢复协调定向测试 3 passed，包含校验→退出缓存→资源操作顺序、失败不继续、构建中拒绝和调用方 abort。
  前端 23 项通过；类型检查、国际化及 desktop UI boundary 通过。未为 React 19 的静态 Modal.confirm 加临时补丁，
  改为受控 Modal；仍有 Arco 的 element.ref 提示，不把它当作新的业务状态。
- 原生 workspace smoke 增加真实 close_idle→新 Provider/新 runtime generation/新 Tab 验证，`--workspace-only`
  通过并清理临时 profile。恢复 API、UI 与 native 各层已有证据，真正主应用的整体视觉/Agent 交互验收仍需继续。
- 最终重跑接口集成与协调测试通过，含已关闭请求的幂等重试、旧请求不推进取消 epoch；浏览器平台 close_idle 三项
  回归通过。完整 native smoke 越过新重建流程后仍因原有跨进程 dragend 不正确而 exit 1，未宣称整体通过。

第五十四个切片：主应用真实交互与最小桌面入口修复（2026-09-14）：

- 重新构建 Windows 主程序并运行在既有隔离 QA 数据目录，确认启动进程与该目录的 port.json 对应，
  未使用用户正常会话或登录资料。通过原生系统输入打开会话 Browser、导航 localhost fixture、输入中文并点击按钮，
  页面实际返回输入内容；WebView 的文本框、按钮进入系统辅助功能树。此证据是用户原生输入，不冒充 Agent Tool 验收。
- 实际打开浏览器菜单和恢复确认对话框：原生网页避让弹层，取消后重新显示同一页面，表单值与点击结果保持。
  本次 UI 只验证取消路径，没有点击确认关闭；实际关闭/重建仍以第五十三切片的接口和 native 分层用例为证。
- 将真实桌面窗口缩至最小尺寸（捕获外框 882×602，对应 880×600 内容区），发现收起浏览器后恢复的侧栏挤占顶栏，
  浏览器按钮被裁剪。修复为按会话容器宽度折行既有控件，浏览器入口独立保留在标题行；不新增菜单、测试模式或移动布局。
  同时移除该样式文件里已有的 768px viewport 分支，保留 reduced-motion 行为。
- 真实主应用复验最小窗口下聊天/Browser Focus 切换，入口不再被裁剪；hide/show 保留同一网页输入和点击结果。
  27 项 UI/顶栏定向测试通过、UI 类型检查及 desktop UI boundary 通过。静态顶栏测试用于防止结构回退，
  尺寸与可点击性依据本次真实窗口操作，不依赖 DOM mock 的布局结果。
- 未在本次验证实际模型驱动的完整 Agent turn、会话之间切换或 popout/reparent；默认 native smoke 的跨进程
  dragend 失败也没有在本切片解决，继续保持未交付状态。

第五十五个切片：正式配置与真实原生 Agent turn 串联（2026-09-14）：

- 新增内部 `--agent-only` 集成用例：启动隔离的正式 DesktopServer，通过正式 Provider/Agent 工作台/Session API
  创建配置，选择 browser.navigate/observe/act，走实际能力编译、factory 和 Nomi turn；本地确定性 OpenAI 协议
  响应提供工具调用，不直接构造 ConversationBrowserTool，不访问外部模型或使用用户凭据。
- 首次正式保存配置返回 422：目录把 Browser 标为 materialized，Nomi 编译环境却把 installation_role_bindings
  置空。修复为仅在宿主拥有 native Workspace 时，从已物化的 bundled Browser Provider 提取 exact Role v2 绑定。
  contract digest 来自 registry，不硬编码 digest，不要求用户添加内部配置，不写数据库迁移或历史绑定。
- 真实链路完成 7 次模型协议请求：导航→观察→中文输入→重新观察→点击→读取真实 console 诊断→正常回复。
  auto-open 事件限定该 Conversation 且只发一次；测试宿主响应事件挂载真实 Surface，并在可见确认后才继续输入。
  读取实际 child HWND 验证运行中 visible/disabled，原生事件 isTrusted、beforeinput/input 与点击结果通过；
  用户导航在运行时被拒绝，terminal 后同一 Tab 保持内容、HWND 恢复输入，普通消息接口能读到最终回复。
- 链路最初通过但整个用例因资源清理失败而 exit 1，未把它记作通过。定位到宿主漏关 Companion 独立 memory.db
  连接池（不是 WebView 残留）。在所有 ingress/background owner 停止且前序清理成功后，显式关闭该池，再关闭主库；
  超时保留失败结果。新增保留 store clone 的文件释放测试通过；短暂 SQLite worker 文件关闭尾部按既有 5 秒边界等待，
  不仅检查 pool.is_closed。未修改任何数据库结构或数据迁移。
- 修复后完整 `--agent-only` 通过，含隔离 DesktopServer 数据目录及原生 fixture profile 的实际清理。
  加强版再次通过，增加可见后输入与普通会话回复断言。依赖锁变化引起的生成合同漂移已重生成，contract check 通过。
- 最终定向回归：native exact-binding 单元测试 1 passed，browser_workspace 接口集成 2 passed（含同一绑定测试），
  Companion 文件释放 1 passed；browser platform boundary 与 diff whitespace 检查通过。故意失败模式验证 exit 1，
  没有把异常清理后的失败包装成成功。失败运行的已退出、可再生临时目录已清理，无用户正常数据变更。
- 这不等同于真实模型自主开发→发现问题→修复→重验，也不等同于主应用 renderer 的所有视觉交互已验收。
  取消中的真实 Agent turn、完整跨 frame/文件能力、旧 Hub 退役与 macOS 移交仍需继续；不新增测试面板。

第五十六个切片：原生 Agent 停止、续跑与应用退出（2026-09-14）：

- 扩展正式 DesktopServer 的 `--agent-only` 集成用例。停止等待模型响应的 turn 后，原生输入恢复且页面内容不变；
  本地模型服务独立持有一份已准备好的点击响应，Stop 后再返回，确认客户端已断开且点击不执行。第一版 HTTP handler
  会随客户端取消而消失，没有形成迟到响应证据；已改为测试服务拥有并追踪响应任务，最终清理等待其全部退出。
- 下一轮使用新观察代际操作同一个 Tab/document，重新输入中文并点击，文档 nonce 不变。另一个 turn 在实际 WebView
  的 Page.navigate 请求已抵达服务器、响应头尚未返回时被 Stop；等待 finished/native enabled 后，用户正常导航成功，
  再释放旧请求也不能覆盖用户的新页面。没有 DOM 合成输入、重新创建替代浏览器或测试产品面板。
- 追加“模型正在等待时关闭应用”发现真实缺陷：原关闭流程会返回成功并销毁浏览器，但模型请求仍在等待。
  新增 Registry 的结果承载 shutdown 边界：同步封闭新入场，先请求已建 runtime 停止；独立拥有的退出任务等待已入场
  构建完成并清理其结果，再逐个证明原 runtime 退出。它不丢弃可能已经启动进程的 factory future，不发布关闭期间完成的
  新 runtime。关闭失败保留 quarantine、工作区权限与重试能力，禁止重新开放入场。
- AppServices 在停止生产者后等待 Registry shutdown，成功才关闭 Browser/Gateway；若 Agent 退出失败，浏览器和数据库
  都保留，不把部分成功缓存成完整退出。新增接口层顺序测试验证失败保留、重试后关闭数据库。
- 修复后的真实原生集成用例完成 17 次模型协议请求并通过，包含活动 Agent 的模型连接断开、原生 child 移除、
  隔离后端目录与 fixture profile 清理。模型仍是本地确定性协议端点，不把这份证据写成真实模型自主开发验收。
- 定向回归通过：Registry 全部 45 项，Nomi native-browser 生命周期 9 项，以及 AppServices 失败保留/重试 1 项。
  Registry 新用例覆盖调用方丢弃等待、冷构建与关闭交错，以及退出失败保留同一 quarantine；原生完整拖放的已知失败未改变。
- browser platform boundary、生成合同 check 和 diff whitespace 检查通过。扩展 startup_smoke 首次并行运行 3 passed / 2 failed，
  两项失败位于服务启动前的共享 Temp 父目录迁移协调锁；不删除或绕过锁，按 `--test-threads=1` 复核后 5 passed。
  该测试夹具的并行隔离问题仍如实保留，本切片未改迁移设计或给并行运行标为通过。

第五十七个切片：工作区限定的原生文件上传（2026-09-14）：

- 为 `browser.upload` 接入正式 Nomi Browser Tool 和 Windows Native Runtime。新操作只有独立能力与宿主工作区授权
  都存在时才进入 schema；`browser.act` 不能隐含调用上传，模型不能提供或替换 filesystem root。工作台文案说明文件
  会发送给网页，以及当前支持的标准文件输入控件边界，没有新建浏览器设置页或测试 UI。
- 文件读取通过已授权目录句柄、逐层 no-follow 打开；拒绝相对越界、绝对路径、NTFS stream/device 特殊名字、目录、
  symlink/junction，且先检查整个路径列表。真实 Windows junction 越界拒绝通过；Unix 的 no-follow/nonblocking 分支
  尚未在 Mac/Linux 执行，不写成跨平台验收已完成。
- 宿主流式准备不可被源文件后续修改影响的文件副本，保留文件名和修改时间，读取期间检查变更；单次 16 文件/64 MiB，
  Runtime 保留 128 文件/256 MiB。副本由 native Runtime 保留至控制器关闭证明，不因 Tool 返回、Agent 停止或网页清空
  input 而消失。正常关闭验证了临时副本清理；非正常退出与跨 Runtime/File worker 分享的完整资源矩阵仍待补齐。
- 上传使用所属 Frame 协议路由的 `DOM.setFileInputFiles`，不合成浏览器事件，fidelity 明确为 browser_protocol。
  标准控件定位、run 锁、观察代际和 native Tab 身份与既有输入共用执行边界。跨 frame 实现路径已接公共路由，但本切片
  的真实上传验证覆盖主 Frame，不把既有其他动作的 frame 用例冒充文件上传证据。
- 扩展正式 Agent API 原生用例：选择 browser.upload 后实际选择两个文件，网页触发可信 input/change，HTTP 服务端
  收到原始字节；修改源文件后，Agent 停止时尚未读取的第二个文件仍返回原副本内容。网页用自身重置按钮清空后，
  保留的 File 对象仍可读；网页在 change handler 内立即消费并清空 input 的场景也通过，不把这种正常应用行为误报失败。
- 清空扩展最初失败，定位到 [Chromium SetFilesFromPaths](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/core/html/forms/file_input_type.cc)
  对空列表直接返回。撤回未经证实的“空列表可清空”提示并在执行前拒绝该参数；真实清空走网页自己的 reset/remove
  控件，不加入 DOM setter/合成事件兜底。上传的协议 ACK 表示已交给网页，不要求 change handler 之后 input 保持不变；
  结果由实际网页/网络验证，Agent 后续 fresh observe。
- 加强后的真实 `--agent-only` 26 次模型协议交互通过，保留之前的停止、迟到响应、原生在途导航与活动 Agent 退出检查，
  含后端目录与 native profile/上传副本清理。仍使用本地确定性模型协议，不宣称真实外部模型自主开发闭环完成。
- 已过定向验证：上传目录/副本 3 项、Browser Tool 权限/schema 8 项、整个 browser-platform 库 304 项（含待退役 Hub
  回归，不代表 Hub 已删除）、UI 文案模型 9 项，类型检查、desktop UI boundary、browser platform boundary、生成合同
  check 与 diff whitespace。自定义按钮/隐藏 input 的 file chooser、用户 picker、下载和其余文件矩阵继续实施。

第五十八个切片：自定义上传按钮与 chooser 生命周期（2026-09-14）：

- `upload` 可以使用同一文档中触发 HTML 文件选择的可见按钮。宿主先真实 mouse down/up/click，再接收所属
  WebView 的 Page.fileChooserOpened；事件本身不含任何文件路径或上传权限。按原观察世界校验 Session/Frame，
  将 backend node 解析回该执行上下文，并校验 input 的 ownerDocument/type/multiple/directory 属性后才提供文件。
  不借助全局 target 枚举、不接受模型提供 node/session id，也不合成点击或文件事件。
- 操作级 receiver 在点击前才 armed；重复事件使已收到的选择失效，原生关闭或取消会撤销 receiver。安装等待期间
  调用方退出也保留已排队卸载，不留下另一份可复用的文件授权。待 picker 的取消使用既有 run token，不新建接管状态。
- Agent 输入门拦截新 HTML 文件选择，已拥有 iframe 的协议路由同步开启/关闭。导航恢复按实时 input lock 重放策略，
  读取发生在 native dispatch 时；解锁失败先恢复原子锁状态再回滚窗口，避免用创建时的陈旧状态恢复策略。
- 初次崩溃回归超时，定位为渲染进程已确认退出后新提交的 chooser 协议命令无法返回。补充 native ProcessFailed
  证明的快速拒绝：旧文档命令立即失败，只保留明确导航恢复入口；对不存在的旧文档无需施加 chooser 策略，重载后
  对新文档重放。真实 crash-only 再次通过，并增加了“崩溃后新 Runtime 命令 500ms 内拒绝”的验证。
- 正式 Agent 原生链路 28 次模型协议交互通过：自定义按钮真实点击、动态创建隐藏 input、实际文件传输、网页删除
  临时 input，以及等待 picker 时 Stop 不传文件、不继续 turn。仍是本地确定性模型协议，不宣称真实外部模型验收。
- managed-popup-only 与 frame-input-only 真实回归通过；后者是既有鼠标/键盘/select 的 frame 回归，不冒充跨 frame
  文件上传验证。chooser 解析 1 项、frame 路由 6 项、UI 文案 9 项，以及类型、desktop UI boundary、browser boundary、
  生成合同 check、diff whitespace 均通过。全量默认 smoke 的跨进程 dragend 失败仍未解决。
- 用户已打开的 OS 文件对话框、完整用户 picker broker、File System Access API、目录与跨 frame 文件用例仍待继续。
  本切片只完善 HTML 文件 chooser 的 Agent 路径，不把新请求拦截说成上述所有系统对话框均已覆盖。

第五十九个切片：跨 frame 文件数据与文档边界（2026-09-15）：

- 新增内部 `--upload-frames-only` 原生 conformance 用例，复用带 site-per-process 的真实 child WebView。
  同进程与 OOPIF 分别验证可见标准 file input，以及点击按钮后临时创建、不挂入 DOM 的 input；所有文件事件可信，
  自定义按钮点击可信。经原生 chooser 的非空 child Session 证明 OOPIF 路由，未把普通 iframe 当作跨进程证据。
- 四条路径都读回正确的中文文件名和 UTF-8 内容；源文件在准备副本后已修改，页面仍取得原始副本。
  测试先销毁文件所属文档再清理副本，原生 fixture profile 清理通过。该用例验证 native driver 层，正式配置/Agent
  链路继续使用独立的 --agent-only，不把两者混称为同一完整主应用场景。
- 取消“picker 必须与触发按钮处于同一文档”的过窄限制：父页面按钮可以触发已观察 iframe 的 chooser。
  选择目标必须匹配当前页已观察的 Frame/Session，并在对应隔离上下文中解析和校验 ownerDocument；不使用模型给出的
  node/session，也不把未知文档临时补成授权对象。UI/Tool 文案同步说明当前页已观察 frame 的范围。
- 实际验证父按钮→子文档文件选择成功；随后让子文档在观察之后导航替换。旧父按钮引用仍能真实点击并确实触发了
  原生 chooser，但旧子文档上下文拒绝文件赋值，文件数据没有到达新文档。重新观察后，同一个按钮上传成功。
- 最终 --upload-frames-only 与正式 --agent-only 原生回归通过；Browser Tool 8 项、UI 文案 9 项、类型检查、desktop UI
  boundary、browser platform boundary、生成合同 check 及 diff whitespace 检查通过。
- 默认完整 smoke 的跨进程 dragend 问题仍未解决；用户 picker、File System Access、目录与异常退出回收继续实施。

第六十个切片：用户原生文件选择器的隔离与真实选择验证（2026-09-15）：

- 实现 Windows 原生 IFileOpenDialog 辅助进程。此前同进程 STA timer 调用 `Close` 虽返回成功，真实可见窗口的
  `Show` 却未结束，该实现已删除；不把 HRESULT 成功当作清理证明。
  [Mozilla 对同类取消问题的调查](https://bugzilla.mozilla.org/show_bug.cgi?id=1870660#c2)也记录了嵌套 Shell 模态窗口
  不能通用关闭的限制。因此每个选择器使用独立受管进程，取消后等待该进程树退出，避免关闭其他选择器或浏览器。
- 私有子命令在应用、数据库及单实例路由初始化前处理。stdin 请求最多 64 KiB、单条 stdout 回复最多 1 MiB；
  严格 DTO 拒绝未知字段、相对/NUL 路径、空成功结果、错误单选数量及超过 256 项的多选结果。保留 Unicode 路径，
  不做有损转换；只继承必要 Windows 环境变量，不向 Shell 扩展传递模型密钥、应用信任令牌或项目工作目录。
- 调用方最后一个句柄释放会取消选择器；后台 worker 继续等待退出。worker panic 或初始化后的早期错误也执行清理，
  失败保留原进程供 `close` 重试；不因为丢失调用方或收到 Result 消息就丢弃进程所有权。
- `--picker-only` 真实原生回归通过：可见后取消、两个选择器互不影响、最后一个调用方退出、提前取消、幂等关闭，
  均不返回选择文件，原生 fixture profile 清理通过。
- 新增仅内部交互验收 `--picker-selection-only`。使用 computer-use 操作实际 Windows 文件窗口，验证单选、双文件
  多选、中文与空格文件名、点击取消；每次结果均比对真实路径并等待 helper 退出，临时文件与 fixture profile 清理通过。
  没有向正式应用加入测试按钮、测试步骤或测试面板，也没有把假结果注入文件选择器。
- 协议/结果校验 2 项测试、桌面主程序编译检查、browser platform boundary 与生成合同 check 通过。
  Windows opusic-sys 既有缺失 PDB 链接警告仍存在，不作为测试失败或已修复处理。
- **尚未接线**：UserReady 网页文件入口、页面/标签失效时取消、Agent 入场前关闭已打开选择器和选择结果回填网页。
  本切片只验证原生选择器基础，不把它计为用户上传、完整输入锁或 Windows 整体交付。目录、File System Access、
  下载及跨进程拖放 source dragend 的已知失败仍在后续范围内。

第六十一个切片：新 OOPIF 文件窗口策略与入场竞态（2026-09-15）：

- 检查用户 chooser 接线前发现：既有 FrameSessions 只消费生命周期，直到下一次观察/操作刷新时才配置新增 iframe。
  这会让 Agent 工作期间的新 OOPIF 在尚未观察时没有文件窗口拦截策略。改为 native attachment 时短暂停住新 iframe，
  由同一事件 worker 递归配置所属子路由、Page 事件与当前文件策略，再恢复执行，不等待 Agent 调用。
  机制使用 [Target.setAutoAttach](https://chromedevtools.github.io/devtools-protocol/tot/Target/#method-setAutoAttach)
  与 [Runtime.runIfWaitingForDebugger](https://chromedevtools.github.io/devtools-protocol/tot/Runtime/#method-runIfWaitingForDebugger)，
  仍不做浏览器级目标发现，不附着其他 Tab、worker 或模型提供的 session。
- FIFO 屏障现在等待此前 attachment 的原生配置完成。配置失败失效路由，不无保护地恢复 iframe；关闭原生视图
  仍是最终清理边界。worker 已有原生回调，不再通过 abort 丢弃；撤销订阅后消费队列结束，保留在途回调的生命周期。
- Agent 入场即更新既有 iframe 与新 attachment 使用的策略，不等第一次 Browser Tool。普通标签/托管 popup 初始化时
  按当前运行状态设置策略，避免把 Agent 的拦截状态错误地留给 UserReady。
- --upload-frames-only 新增首个观察前用例：先建立原生路由再导航创建 OOPIF，仅通过 fixture 的页面坐标驱动真实鼠标
  点击，其 chooser 在非空 child Session 中被拦截；没有 observe/trees 调用代替自动初始化，也不向该请求赋文件。
  后续既有六次实际文件传输、中文内容、动态 input、委托与旧文档拒绝用例继续通过。
- frame-input-only、managed-popup-only、crash-only、runtime-locks-only、agent-only（28 次本地确定性模型协议）均通过，
  原生 fixture profile 清理通过。首次 popup 回归暴露旧测试将候选标签登记误当作已完成激活，现等待真实激活并保留
  原有标签数量、活动标签、输入锁、加载、WindowProxy 与关闭断言；没有取消这些验收要求。
- frame 路由 8 项测试（含新增配置屏障、失败失效）、主程序编译、browser boundary、生成合同 check 通过。
- UserReady 原生文件选择器回填、导航/隐藏/Agent 入场时已打开选择器的取消仍未接线。本切片修复其前置的 frame
  策略竞态，不宣称用户文件交互已完成；高 churn/配额/配置失败的完整原生恢复矩阵及跨进程 dragend 仍须继续验证。

第六十二个切片：UserReady 网页文件入口与取消边界（2026-09-15）：

- 应用用户文件选择器接到实际 HTML chooser。原生事件提供 Frame/Session/backend node，宿主从原有 FrameSessions
  取得不可序列化的所属路由，在发起文档隔离上下文中绑定具体 input。没有新增前端上传协议、模型选文件入口或任意
  session/node 参数；用户选定文件只由辅助进程回传，Agent 上传仍走原有独立能力与工作区限定。
- 已安装用户 broker 的 native view 始终拦截 HTML chooser，包含其子 session；UserReady 才打开应用选择器，
  Agent 期间只交给原有 Agent 文件路径处理。回填之前验证原 input，真正 UI dispatch 时再次检查取消、可见性、关闭
  与运行锁，避免仅在异步调用发起时做一次检查。动态、不挂 DOM 的 input 保持原文档对象身份，不用 selector 重新定位。
- 每个 view 保留当前选择请求与 helper 清理权；隐藏、原文档/祖先导航、关闭与 Agent 入场取消请求。
  使用完整 frame 祖先链区分无关 iframe，避免广告或其他无关 frame 刷新就打断用户选择。native close 与运行输入门
  等待选择请求及 helper 退出；重复请求不把旧选择结果赋给新 input。
- --user-files-only 真实回归通过：主页面打开原生文件窗口、隐藏取消、主文档导航取消、真正非空 Session 的 OOPIF
  临时 input 打开窗口后随文档替换取消、Agent 入场前等待退出，以及最终关闭。所有取消路径没有向网页交付文件；
  无关 iframe 导航保持原选择器打开。此证据覆盖真实窗口与真实生产 Runtime，不是 mock picker 返回值。
- 初次用例最终关闭失败，定位为 controller Close 后 WebView2 仍短暂占用临时 Profile（Windows error 32）。
  Runtime 现对该临时目录的 sharing/lock violation 有最多 2 秒的有界等待，其他错误不重试；仍失败时保留原清理权。
  更改后完整用例与 fixture profile 清理通过，不再依赖调用方重复发起 close。
- agent-only（28 次本地确定性协议）、crash-only、managed-popup-only、runtime-locks-only 回归通过；Windows 原生
  单元测试 28 项及主程序编译通过。开发依赖加入 smoke 日志，生成合同因依赖摘要变化重新生成并校验。
  日志同时暴露知识 MCP 管道 error 5 和退出期后台任务注册告警；未把 Browser 回归通过宣称为所有后台服务无告警。
- `--user-file-selection-only` 已准备主页面多选与 OOPIF 临时 input 的真实数据验收。computer-use 要求文件上传确认，
  已询问是否允许在本机测试页选择测试程序生成的临时文件，尚未收到确认，因此**未执行正常选择后的端到端数据验收**。
  不以之前 helper 单独选择通过、Agent 上传通过或这次取消通过，替代用户正常选择回填证据。
- 文件类型过滤、目录、原生 cancel 事件完整语义、File System Access、更多异常/切换/崩溃矩阵继续实施；没有新增测试
  产品面板，也未完成默认 conformance 的跨进程 source dragend 问题。整体 Windows 交付仍未完成。

第六十三个切片：原生文件类型筛选与真实窗口验收（2026-09-15）：

- 将发起 input 的 `accept` 接到原生 IFileOpenDialog。属性在该文档隔离对象上有界读取；扩展名与 MIME 在本机
  归一化、去重，支持 multipart/Unicode 扩展名和 image/audio/video 通配类型。使用已锁定的 mime_guess 离线表，
  不查询网络、不打开系统注册表来解释网页字符串。
- 只把经过校验的扩展名送入 helper，双方均限制数量、长度及字符；网页不能注入 Shell 路径、分号或任意通配式。
  无效/未知 token 忽略，过宽的提示不截出一个错误的部分集合，而是保留通用选择。增加 Windows Shell Common
  编译 feature，`SetFileTypes` 在 Show 前调用一次，默认使用匹配类型，另有 `*.*` 供用户选择所有文件。
  依据 [HTML accept 定义](https://html.spec.whatwg.org/multipage/input.html#attr-input-accept)和
  [Windows SetFileTypes](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifiledialog-setfiletypes)，
  该筛选不授予文件权限，也不替代服务端内容验证。
- 新增仅内部 `--user-file-filter-only` 验收。使用 computer-use 在真实网页发起的系统选择器中进入测试临时目录，
  实际观察默认 `*.csv;*.txt` 只显示 accepted.csv/accepted.txt；切换 `*.*` 后 other.png 出现。最后点击取消，未点
  文件、未提交打开、没有向网页传文件；native helper、临时文件及 fixture Profile 清理均通过。这不是正常上传验收。
- picker-only 与 user-files-only 原生回归继续通过，含独立 picker 取消、主/OOPIF 导航、无关 iframe 保持、隐藏、
  Agent 入场等待退出和关闭。Windows 单元测试 30 项、主程序编译、browser boundary、生成合同与 whitespace check 通过。
  首次 Cargo.lock 写入遇到 Windows 映射占用 error 1224，正常重试成功，没有删除或强行重写锁文件。
- 检查 Chromium 当前源码发现，空路径列表被忽略，`setInterceptFileChooserDialog` 的 cancel 参数应用于后续的打开
  探针，不是已交给宿主的请求完成接口。参考
  [FileInputType](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/html/forms/file_input_type.cc)和
  [InspectorPageAgent](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/inspector/inspector_page_agent.cc)。
  这是下一步取消语义调查的源码依据，不把它当作当前 WebView2 完整 cancel 事件验收；未加入 DOM 合成事件或重放点击。
- 正常选中文件后的网页回填仍待此前的用户确认；目录、完整 cancel 事件、File System Access、下载和其他交付清单
  继续推进，整体未完成。没有新增 Browser 设置、测试步骤或用户测试工作流。

第六十四个切片：退役失效的知识浏览器适配器（2026-09-15）：

- 全仓调用核对确认：知识库已使用 `BrowserRenderContentPort`，旧 `BrowserFetcher` 不再有生产构造/注入调用，
  但 Agent crate 仍编译并导出其单独的 Hub 租约、排队、续租和 Drop 清理实现。物理删除 browser_fetcher.rs 与
  browser_fetcher_tests.rs 共 1,716 行及模块/类型导出，没有 deprecated wrapper、别名或回退构造器。
- 保留仍适用的行为验证，转到 Knowledge 当前的 `rendered_content_to_page`：标题/正文提取、不混入 script 或
  LLM 包装、原始 HTML 截断标识、UTF-8 Markdown 上限及无标题页面。旧独立实现和专属假 Hub 生命周期测试不保留。
- 更正 Knowledge API 的过期注释：rendered 请求走 canonical `browser.render_content`，Provider 不可用时明确失败，
  并不会回退普通 HTTP。未更改字段、数据库或迁移。更新 Wave 1 删除清单的现存源引用到 typed port 与转换函数，
  保持该清单的总体未闭合状态，未伪造 canonical Provider 已组合的证据。
- browser boundary 增加旧源文件、旧测试文件及跨文件重新导出的防回归约束；scanner self-test 与全仓检查通过。
  7 项 rendered 路径/转换测试及 1 项 unavailable port 测试通过；`cargo check -p nomifun-app --features browser-use`
  通过，生成合同更新后 check 与 diff whitespace 检查通过。
- 本切片只退役已失效适配器。仍有生产调用的 Hub 核心、Agent Lane Provider、managed tool/driver 和 vault 路径未
  因名字旧而直接删除；后续必须连同真正隔离的 Headless owner 替换。没有恢复旧知识渲染接线或新增浏览器测试产品。

第六十五个切片：切断旧登录仓库与配置密钥接线（2026-09-15）：

- 核对实际启动路径发现，剩余后台 Hub 仍从旧共享 vault 读取 Cookie 并播种到进程身份状态，managed adapter 也会
  写回旧仓库。删除这些读取、播种、写回及其专属测试，不以空 wrapper 或兼容别名保留旧入口。
- 后台配置统一使用 `browser-v2/headless/`，显式 profile root 为其 `profiles/` 子目录；恢复扫描复用同一目录定义，
  只处理这些新根下的已知 profile 家族。旧 browser-data/platform-profiles 不扫描、不搬迁、不导入、不删除。
  此路径属于仍待替换的后台适配器，不把它当作最终 Headless owner 已交付。
- 背景配置关闭旧持久登录开关且 storage_state 为空。删除 `with_identity_vault`、仅用于注入密钥的策略装饰钩子与
  persister 字段；当前运行内部身份快照仍保持原有代际校验，不再因捕获而持久化到旧仓库。
- 删除 NomiResolvedConfig 的旧浏览器登录密钥字段、factory 赋值及 bootstrap 转发。继续核对后确认 AgentFactoryDeps
  中的应用级加密密钥也已无读取者，连同两个构造点一并删除。其他服务所需的应用数据加密密钥保持不变，数据库未修改。
- 新增 2 项配置/恢复测试：确认新目录、无旧登录状态、非交互默认；放入旧目录的损坏 marker 完全不参与扫描，而新
  目录中的损坏 marker 仍被检查且保留。测试不启动真实 Chrome，不据此宣称完成全部跨 Profile 浏览器隔离验收。
- 29 项 managed adapter 测试、4 项 Agent 类型/生命周期集成、5 项工厂 Provider 集成、5 项 serial startup_smoke
  通过，桌面主程序编译检查通过。首次 Agent 集成命令缺少 test-support feature，补齐所声明 feature 后通过。
- browser boundary 增加旧仓库读取/写回、旧恢复根、已删 managed 钩子、旧浏览器密钥字段及无用工厂密钥的防回归
  检查；self-test、全仓扫描、生成合同 check 和 whitespace check 通过。
- 仍须替换 Hub/Lane 核心、低层 standalone BrowserTool/vault 与其余后台消费路径；本切片没有把它们改名后宣布退役。
  正常用户上传验收仍待原确认，其余 Windows 原生交互及交付清单继续有效，整体未完成。

第六十六个切片：删除低层共享浏览器 vault（2026-09-15）：

- 继续核对低层消费者，删除 BrowserTool 的共享保存协调器、节流/代际写入实现、启动时旧状态导入、导航完成后的
  自动捕获写回，以及持久化数据根/密钥 builder。构造函数移除密钥参数，AgentBootstrap 同步删除字段和入口；
  没有保留兼容重载。standalone 默认目录切到 browser-v2/standalone，显式宿主内存快照注入不受影响。
- 物理删除引擎 vault.rs（715 行）与旧磁盘共享身份 integration_w4d.rs（277 行），移除全部公开 vault 读写、路径
  和错误类型导出，删除 BrowserTool 中只服务于该机制的测试。原有磁盘状态文件未扫描、迁移或删除，代码可从 Git 恢复。
- 保留 StorageState 的有界内存捕获/恢复及 cookie/localStorage/IndexedDB 字段转换，更新过期的“磁盘 vault”注释。
  新 integration_storage_snapshot.rs 验证显式内存快照注入与 None 无注入；它保留需要本机 Chrome 的 ignore 标记，
  本切片只执行 --no-run 编译，没有把两个集成用例计作真实浏览器验证。
- 引擎的身份/evaluate 互斥安全开关仍保留，不因为删除持久化功能而放松脚本执行保护；它不再表示共享仓库读写。
  Hub/Lane、其他 facade/config 路径继续按 v2 替换，未借此宣称全部旧浏览器后端退役。
- 浏览器库测试 182 项通过、4 项按原标记忽略；内存快照测试 19 项通过；替代集成测试编译与桌面主程序检查通过。
  初次编译定位并修复了一个遗漏的旧构造函数参数及失效 Path import。
- browser boundary 扩展到低层工具、bootstrap 和引擎，拒绝旧 vault 文件/API/密钥协调器恢复；self-test、全仓扫描、
  生成合同 check 与 whitespace check 通过。正常用户上传验收仍待此前确认，整体交付尚未完成。

第六十七个切片：真实 Chrome 内存快照与拒绝路径（2026-09-15）：

- 将上个切片只编译的快照集成测试改为本机 HTTP fixture 和每个实例独占的 TempDir，不再访问 example.com 或使用
  固定测试 profile。服务器只记录有界请求头，关闭时取消并等待连接任务；测试数据只包含自己生成的 Cookie/存储值。
- Launched 提供只读的精确进程树退出回执，观察者不能终止进程或取走 profile 清理权。测试先等待该回执，再清理
  对应临时 profile；操作超时在清理外层处理，不把隐藏页面或丢弃 backend 当作物理退出证明。
- 使用本机 Google Chrome 152.0.7977.76 实际执行 2 个 ignore 标记的集成测试，均通过：来源实例捕获后退出；
  显式快照在副本首次 HTTP 请求之前带上 Cookie；同源 localStorage 恢复，切到不同 host 不越界；随后全新 profile
  的无快照实例不继承身份。每个 Chrome 进程树及 profile 清理通过，额外检查未发现本轮 profile 对应的残留 Chrome。
- 发现原构造路径会在解析/恢复失败后仅 warn 并继续，可能把显式恢复请求静默变成匿名状态。现由 Host 在建连接前
  有界解析为 StorageState，拒绝错误类型和未知顶层字段，Lane 恢复错误向上传递；错误消息不包含原快照内容。
  原生拒绝用例验证两类异常输入、非就绪返回、错误文本不带测试 token，以及已经启动的进程/profile 的清理。
- 21 项状态/恢复边界测试、29 项 managed adapter 测试与桌面主程序检查通过；browser boundary、生成合同和
  whitespace check 通过。实际 Chrome 测试允许 loopback 的配置仅存在于 fixture，未放宽生产出口策略。
- 这些证据属于共享 CDP/后台引擎，不替代 WebView2 用户上传、完整多 Lane/IndexedDB/故障矩阵或跨平台验收。
  正常用户文件选择回填仍待此前确认，其他交付清单继续有效，整体未完成。

第六十八个切片：共享浏览器启动 Cookie 不重复恢复（2026-09-15）：

- 新增真实 Chrome 回归，先捕获初始 Cookie，再启动共享 Host，让第一个 Lane 的网页更新 Cookie，随后创建第二个
  Lane。修复前实际失败：新 Lane 把已更新值覆盖成启动快照旧值。用例先完成 Host/profile 清理再报告失败，未留下浏览器。
- 将 Cookie 恢复从每个 Lane 的构造移到 Host 连接建立后、页面发布前，只执行一次；清空已消费的 Cookie 数据，
  不额外引入状态锁或重复初始化标志。后续 Lane 保留运行中的浏览器 Cookie，不再重复读取初始 Cookie 快照。
- 修复后的 3 个真实 Chrome 快照用例全部通过，覆盖新增/重建 Lane 不覆盖更新、首请求 Cookie 注入、来源限定的
  localStorage、无快照新 profile 保持干净及异常输入拒绝。新增了结构有效但被 Chromium 拒绝的 Cookie：Host 构造
  明确失败，错误不包含测试载荷，进程与 profile 清理完成，不发布部分恢复状态。
- 27 项 Host 生命周期/容量测试、21 项状态/恢复边界测试及桌面主程序检查通过；browser boundary、生成合同 check
  和 whitespace check 通过。复现与验证仅使用本机测试页面和独立 profile，没有修改用户账号或产品 UI。
- 此处修复的是共享 CDP/后台引擎，不等同于 Native Surface、完整多页面存储/IndexedDB/崩溃矩阵或 Headless owner
  替换已完成。正常用户上传验收仍待此前确认，其余交付清单保持不变。

第六十九个切片：真实布局四边形与投影 iframe 输入（2026-09-15）：

- 删除手工组合 CSS rotate/scale/translate/zoom 与 bounding rect 的旧映射，改取原生 DOM.getBoxModel content quad。
  使用统一投影/逆投影处理平面内容，避免为每种 CSS 变换再写一个解释器。坐标来源对应 Chromium 的
  [InspectorDOMAgent](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/inspector/inspector_dom_agent.cc)
  与 [InspectorHighlight](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/inspector/inspector_highlight.cc)
  布局查询路径，实际效果以本机 WebView2 验证为准。
- 保持每层 iframe 可见/稳定与命中检查。同进程嵌套的 quad 位于 session 根坐标，先逆投影回父文档 viewport 再
  hit-test；跨进程保持所属 session。输入点确定后复查原 quad 和 viewport，退化、非有限、非凸及变化的几何拒绝，
  不用猜测坐标继续输入。原生鼠标键盘路径未换成 DOM 合成事件。
- frame-input-only 真实回归通过：已有缩放/旋转/镜像、焦点、遮挡、导航失效仍成立；新增 transform perspective、
  父元素 perspective、独立 Y rotate/Z translate/Z scale、静态 motion path、同进程嵌套透视的可信点击与中文输入。
  变换后的父文档遮挡仍阻止点击。首次可信 input 断言读取过早，现等待两层 postMessage 传回实际事件，未放宽 isTrusted。
- 投影/逆投影、非双线性中点、镜像、退化与变化几何等连同 Frame plan 共 5 项单元测试通过。html-drag-only、
  frame-drag-cancel-only、upload-frames-only、agent-only（本地确定性模型协议 28 次）和 runtime-locks-only 回归通过，
  fixture Profile 清理通过；桌面编译、browser boundary、生成合同和 whitespace 检查通过。
- 重新执行 frame-drag-only 仍失败：目标收到 trusted drop，但跨进程源没有正确的成功 dragend 生命周期；清理路径
  的 dropEffect=none 不冒充完成。该失败未通过更改断言或合成事件掩盖，仍是整体交付的未完成项。
- 本切片不新增用户测试面板。动态动画、极端投影/裁剪、缩放及完整输入矩阵仍需继续，不能把已验证静态场景当作
  所有 3D 页面均已验收；正常用户上传仍待此前确认，其他架构与交付清单保持有效。

第七十个切片：原生页面缩放与滚动坐标验收（2026-09-15）：

- 扩展 frame-input-only，使用测试 WebView 的原生 SetZoomFactor/ZoomFactor，而非 CSS zoom 或设备仿真。
  分别验证 80%、125%、150%，每档同时核对读回值并等待实际 CSS 视口匹配，避免仍在上一档缩放时误报成功。
- 本机基准宽度 900 CSS px 对应实际读回 1125、720、600 CSS px；每档都有远离原点的主页面按钮可信点击、
  同进程嵌套透视 iframe 点击与可信中文输入，输入值在实际子文档核对，不只检查命令返回。
- 在 150% 下把测试 iframe 放到长页面较深处并滚动，再验证点击仍命中且根视口保持明显滚动，防止将文档坐标
  混作视口坐标。用例最后恢复原生缩放，原生 fixture Profile 清理通过。
- 最终 frame-input-only、browser boundary、生成合同 check 与 whitespace check 通过。结果表明当前原生 quad
  映射在这些缩放场景正确，本切片没有为了“修复”一个未复现问题而加入生产坐标补偿。
- 改动仅为内部验收，不新增用户测试模式/控制面板，不涉及手机目标或共享 renderer 布局。其他缩放、动态变换和
  跨进程拖放完整矩阵仍需继续；整体交付清单及正常用户上传待确认状态保持有效。

第七十一个切片：原生脚本对话框与输入回执探针（2026-09-15）：

- 检查现状确认，生产 Native Surface 尚无 ScriptDialogOpening 的专用响应接线。新增仅内部的 dialog-probe-only，
  直接使用 WebView2 原生 deferral，不用 DOM 模态模拟或普通网页表单替代浏览器脚本对话框。
- 第一次安装在已加载文档上没有收到事件。依据
  [WebView2 Settings](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings)
  的文档加载语义，改为设置后重新加载测试文档，真实原生事件正常到达；记录为生产首次导航/popup 初始化的约束。
- confirm 接受/取消、prompt 接受/取消及 alert 共 5 种情况通过。每次都观察 150ms：输入调用仍待完成；随后原生
  Accept/ResultText/Complete 正确传回 true、false、文本、null 或继续执行，输入才结束。测量已排除额外 run 清理，
  不将“点击之后再答复”误当作顺序可行。成功后清理 deferral、事件订阅、输入状态及 fixture Profile。
- 更新 ADR §9.6：后续需要保留在途输入所有权，明确返回等待 dialog 的状态，答复后再确认完成；Stop 必须先取消
  deferral 再等输入锁。不能强制猜测回答、重放点击、合成事件或额外发起嵌套模型请求来绕过回执。
- 本切片是已执行的原生行为验证和架构约束，不是生产 dialog 功能接线。用户对话框 UI、Agent 响应通道、beforeunload、
  跨 frame 及关闭竞争仍未完成，未新增测试面板或接管模式。正常用户上传仍待此前确认，整体交付清单保持有效。

第七十二个切片：原生脚本对话框宿主所有权基础（2026-09-15）：

- 将探针中的单例 COM 处理替换为 `browser_surface/script_dialogs.rs` 的按 WebView 注册表；COM 参数与 deferral
  仅留在 UI 线程，外部接收有界文本快照和随机请求 ID。答复复查 ID、Tab/runtime/document target 及当前元数据，
  拒绝过期、错误和重复答复；该模块不提供 IPC，也不自行授权 Agent 或用户，正式接线仍须经过 run guard。
- 对话框消息与默认值分别限制 4096 UTF-8 字节，显示截断标志；来源仅投影 origin，不暴露凭据、路径与查询。
  prompt 显式答复拒绝 NUL/超长文本；未提供文本时保留浏览器原始默认值，不把截断后的展示文本误写回网页。
- cancel 在 Complete 成功前保留所有权；`close_native_view` 在原生 controller Close 成功后释放事件与待处理对象。
  不以订阅者丢弃或超时替代结束证明。原探针的单例实现已删除，测试使用同一份宿主代码。
- 扩展为 8 个实际 WebView2 情况：confirm 接受/取消、prompt 显式接受/取消/中文默认值/长中文默认值、宿主取消和
  alert。每例均拒绝错误文档、错误请求 ID、当前文档失效及重复答复，并验证 JavaScript 实际结果；最后在另一个
  原生命令被 confirm 阻塞时关闭 controller，确认该命令以失败结束、dialog 快照清空、临时 Profile 可清理。
- 首次测试出现一次来源元数据为空的失败，复查发现 add_child 后可能仍是初始 about:blank；探针现先等待 HTTP
  fixture 加载完成再设置/重新加载文档。最终 8 情况加 pending-close 连续 5 轮通过，未通过放宽 origin 断言消除失败。
  3 项文本/来源/答复单测、desktop binary check、browser boundary 和 whitespace 检查通过。
- 当前只有内部验收调用 install/respond；正式页面尚未启用，避免在 Agent 响应通道接好之前关闭默认网站对话框。
  下一步仍是保留在途输入并返回 awaiting-dialog、同 run 响应和先取消后 settle 的 Stop/关闭路径，以及普通用户
  网站对话框显示。beforeunload、跨 frame、并发标签与完整停止竞争仍需验收；不是完整 dialog 产品或 Windows 交付。
  本切片没有新增测试产品入口、数据库迁移或 macOS 实施，其他后续清单保持有效。

第七十三个切片：Agent 对话框响应与在途输入所有权（2026-09-15）：

- BrowserActionResult 增加明确的 completed/awaiting_dialog 状态，BrowserDialog 类型进入共享 runtime contract；
  原生 COM 对象仍不离开宿主。新增 Browser Tool `dialog` 操作，归属现有 browser.act，经 NativeBrowserTurn、
  Workspace 与当前 run guard 到达 native runtime；不增加能力名称、UI 测试功能或模型可伪造的 run ID。
- 新增 pending_action 原生所有者：首次输入返回等待 dialog 后，JoinHandle、自动化锁与原 run 取消令牌仍被 Runtime
  持有；等待者被取消不丢弃任务。答复之后继续等待原任务或第二个 dialog，不重放点击。其他输入、观察和 Tab 命令
  在等待期间返回明确的 BROWSER_DIALOG_PENDING，快照读取保持可用。原任务真正返回后才给 completed。
- Stop 先 drain 当前及同一 JavaScript 回调后续 dialog，再等待任务与原生输入清理；worker panic 不作为已清理证据。
  Runtime 显式关闭则允许继续销毁 controller，再等待输入所有者并释放 Profile，不因 graceful dismissal 失败而永久
  拒绝尝试原生销毁。等待 dialog 也沿用会话原有自动展示逻辑，无新产品工作流。
- 扩展 dialog-probe-only 为真实 BrowserWorkspace/RunGuard/NativeRuntime 集成：一次 trusted click 依次确认 confirm
  和中文 prompt，点击计数保持 1；阻止重放、等待中的观察和用户导航；拒绝旧 dialog/旧 run 答复；Stop 取消连续
  对话框，settle 后仍锁定，finish 后恢复 UserReady；新 run 复用同一页面，再验证等待输入时关闭 Runtime。
  原有 8 项原生对话框及 pending native-close 验证保留，最终扩展矩阵连续 3 轮通过。
- 首次 runtime 测试立即 Reload 了仍在初始导航中的 about:blank。已修正测试初始化：等待 HTTP 页提交，再安装并
  reload，且要求 document_generation 增加后才同步测试元数据；未放宽 URL/文档断言。
- 4 项 dialog/权限单测、runtime-locks-only、managed-popup-only 与既有 agent-only 的 28 次脚本模型调用回归通过；
  后者仍不是公网真实模型验证。browser boundary、contract check 与 whitespace 检查通过。停止/关闭日志仍可见原有
  file chooser late restoration 和知识 MCP broker/后台注册告警，不将其描述为整个应用无告警。
- 正式页面尚未安装新 handler，生产用户网站对话框显示、初始导航/popup/beforeunload、异步无在途输入的 dialog、
  跨 frame 与异常工作线程完整矩阵仍待实施；此次测试显式安装并同步 fixture metadata，不能证明生产自动安装。
  Windows 整体交付、旧 Hub 完全退役、正常用户上传待确认和 macOS 移交等既有清单继续有效。

第七十四个切片：普通网站对话框界面与用户答复（2026-09-15）：

- BrowserTabSnapshot 增加可选 script_dialog；原生所有者发布/清空对话框时同步快照并递增 revision。确认的文档
  崩溃清除旧请求，但保留 controller 的 handler 供以后显式 reload；正常关闭在原生销毁证明后退役注册。
- 会话浏览器增加 WebsiteDialog 组件，支持 alert、confirm、prompt 与 beforeunload 的普通网站交互；内容使用
  React 纯文本，长默认值不经截断展示反写网页，后续请求不继承旧草稿。Agent 运行时仅只读显示、不抢焦点、不接受
  Escape/按钮答复。对话框局限于浏览器区域，遮挡处理沿用 native surface hide/show，不使用截图、测试面板或接管模式。
- 用户 dialog command 复用 authenticated user operation gate，在原生输入锁之前答复，避免锁住正在等待的网页；
  允许 native 页面因显示对话框而暂时被遮挡。仍要求当前活动 Tab 和 UserReady，Agent 不能通过 user command 答复。
  不确定的 HTTP 回复只提示无法确认结果，不虚构“已成功”或“未应用”。
- 原生回归补齐：Agent 停止后用户 prompt 恢复、遮挡时用户命令答复正确返回中文、快照发布/清空、重复请求拒绝、
  Agent 无法冒用用户通道，以及用户对话框未关闭时开始新 run。后者首次稳定复现超时：set_user_input_enabled 在
  drain 前等待了被 dialog 阻塞的 CDP 配置。已改为 HWND 禁用→dialog drain→CDP 配置，保留严格输入锁与断言。
- 一次关闭清理出现 Windows 145（目录非空），原样重跑通过；在既有 2 秒有界清理中加入该退出写入竞态的重试。
  仅作用于所有原生 controller 销毁后的自有临时 Profile，不扩大删除目标，不重试权限错误或丢失失败时的目录所有权。
- 最终扩展 dialog-probe-only 连续 3 轮通过；crash-only、runtime-locks-only、permissions-only 通过。31 项 Browser
  UI 测试、52 项 native example 单测及独立用户命令权限单测通过；UI typecheck、desktop UI boundary、i18n、theme、
  dead CSS、browser boundary、contract 与 whitespace 检查通过。正常关闭仍可能记录既有 chooser late restoration 告警。
- 本次为 renderer 组件与 native 命令的分层验证，没有实际主应用视觉验收；fixture 仍显式安装 handler，并提供测试
  metadata/revision，不代表正式 native Tab 的自动安装接线已完成。初始脚本、导航、popup、beforeunload、异步
  dialog 与其余跨 frame/竞态矩阵仍须完成后才默认启用。Windows 整体交付、旧 Hub 清空、用户上传待确认和 macOS
  移交等后续清单均保留，不新增数据库迁移。

第七十五个切片：正式标签页启用网站对话框与通用在途任务（2026-09-15）：

- 普通 Tab 在首次导航前、popup 在绑定前自动安装 ScriptDialogOpening handler，使用真实 Tab metadata/revision。
  删除 runtime 对话框验收中的手动安装、复制元数据和额外 reload；现直接断言正式宿主的自动安装与实际快照。
- 将 pending_action 物理替换为 pending_work，不保留别名；输入、导航、观察、截图使用同一个有所有权的任务注册表。
  返回等待 dialog 不丢弃原始任务/锁，不重放操作；正常非模态多标签工作可并行。已等待的操作限制后续命令，取消与
  关闭统一收束任务；异步网站 dialog 无在途输入时同样可答复。
- 模型导航快照遇到 dialog 明确标记 awaiting_dialog；被打断的观察返回 script_dialog、空元素与未观察覆盖提示；
  截图受阻时 Browser Tool 返回对话框元数据，不生成假图片。答复完成后仍要求 fresh observe。
- 新增真实初始文档与 popup fixture，正式宿主验证用户初始 prompt、Agent 导航后的新文档 prompt、beforeunload
  取消保留原页/接受真实导航、异步 dialog 打断观察/截图及答复收束、popup 首屏确认和原生 opener/trusted click。
  原有 8 项原生对话框和用户/Agent run 生命周期矩阵继续通过，未再通过手动安装来替代产品接线。
- managed-popup-only 暴露绑定前过早解锁输入的回归。修正为绑定前保持 HWND 锁，并拆出无 CDP 等待的原生输入
  切换，绑定后按实时状态应用。PopupRequest 在失败时保留到候选清理结束，避免隐式 Drop 抢先销毁；统一销毁后
  清理处理器，未确认原生关闭时不直接移除 Tauri 注册。相关拒绝/Drop/取消断言保留，显式 Drop 触发验收清理。
- dialog-probe-only 扩展矩阵、popup-only、managed-popup-only、runtime-locks-only 及 agent-only 的 28 次脚本模型
  调用均通过。52 项 native example 单测、desktop binary check、browser boundary、desktop UI boundary、contract
  与 whitespace 检查通过。既有知识 MCP pipe、late task registration 和 chooser late restoration 告警仍存在，未宣称
  整个应用无告警或真实公网模型闭环已验证。
- 仍需实际主应用视觉验收。单标签关闭在 dialog 等待阶段可能被当前准入拒绝，需补齐舒适的关闭行为；完整跨 frame
  dialog 生命周期、用户 popup 创建与 run 启停交错、异常关闭/清理及其余矩阵未完成。默认完整拖放 smoke 的既有失败、
  用户上传待确认、旧 Hub 完全退役、运行时发行与 macOS 移交等总清单不变；本切片不是 Windows 全量交付。

第七十六个切片：对话框等待中的精确标签关闭（2026-09-15）：

- Close command 不再被 dialog 等待准入拒绝，且不会把关闭本身再次投影成等待同一 dialog；仍先验证准确的
  runtime/Tab/document target，用户和 Agent 各自使用既有权限通道，没有接管或运行中用户操作。
- 每个在途任务改用 run 的子取消令牌，并记录实际标签作用域；新建页分配 ID 时即登记。关闭只取消/收束该已销毁
  页面所属的操作，不取消父 run，也不对其他标签的 dialog 作决定。并行关闭任务彼此不等待。
- 工具栏关闭、页面自身关闭、初始化失败与 Workspace 销毁统一走带单标签关闭锁的 retire_native_tab，先确认
  原生销毁再等待输入任务，不先等待被 modal 阻塞的 driver 锁；移除重复清理分支，失败时仍保留原生所有权。
  初始化失败保留此前活动页或后来用户选择的页，不覆写后续激活操作。
- 原生新增验证：用户关闭首屏 prompt、Agent 关闭被 confirm 阻塞的输入、其他标签 dialog 不变、原 run 继续
  操作剩余页面、拒绝过期 close、用户关闭 beforeunload 阻塞的导航、关闭后新建页，以及两次并行关闭不会死锁。
  扩展 dialog-probe-only 连续 3 轮通过，均确认实际 native view 被移除及临时 Profile 清理。
- UI 回归确认关闭含 dialog 的标签后回到空浏览器，不进入 unavailable。32 项 Browser UI 测试、52 项 native
  example 单测、desktop binary check、runtime-locks-only、managed-popup-only、popup-only、agent-only 的 28 次
  脚本模型调用、desktop UI boundary、browser boundary、contract 与 whitespace 检查通过。既有后台注册、知识
  MCP pipe 和 chooser late restoration 告警不在本切片中冒充已修复。
- 主应用视觉验收仍未执行；完整跨 frame dialog、popup 创建/取消/关闭交错、异常原生失败及文件并发矩阵还需
  推进。完整拖放的既有失败、用户上传待确认、旧 Hub 退役、发行级验收和 macOS 移交等后续清单不变，整体未完成。

第七十七个切片：主应用视觉验收与全局插件脚本隔离（2026-09-15）：

- 使用 computer-use 技能在真实 Windows 主应用中验收；确认并重启的都是临时数据目录内既有 Browser Workspace QA
  实例，不使用正式用户数据。旧运行中 exe 阻止 Cargo 替换，验证 PID/QA owner 后停止该旧实例并完成新二进制构建。
- 新增仅监听 127.0.0.1 的 browser_ui_fixture_server.ts，固定提供三份 HTML fixture，不暴露任意文件路径。给 prompt
  与 confirm fixture 增加纯文本结果回显，用于核对 UI→后端→原生网页的真实值，不新增产品测试入口。
- 主应用实际验证会话浏览器展开、首屏 prompt、中文输入及网页回显、对话框遮挡原生页后恢复。收窄窗口时进入
  既有 Focus 布局，控件保持可见；截图尺寸与 renderer client 尺寸不能混同，本轮不把捕获尺寸当作精确 880x600 证明。
- 视觉验收发现之前 smoke 未覆盖的真实问题：主应用 tauri-plugin-dialog 2.7.1 的全局初始化把 window.confirm 改为
  async 函数。页面直接继续执行，并回显 [object Promise]；这不是原生 dialog 自动取消，而是网页 API 被平台污染。
  原生示例未加载该插件，因而此前局部通过未证明主应用语义正确。
- 新增 native_api_plugins 适配层，保留原插件 setup、生命周期及显式 API handler，不注入 dialog 的 alert/confirm
  全局替换。不 fork SDK、不向浏览器页面补“恢复原函数”的脚本、不放宽 IPC 权限。主应用复测确认窗口保持等待，
  取消后网页实际回显 false；等待中的标签关闭正常回到空浏览器，没有 unavailable 页面。
- 同类检查发现 notification 插件也覆盖标准 Notification。官方 JS 通知 API 依赖它，因此只在 main/companion 的
  第一方顶层文档运行该初始化；外部浏览器页和子框架保留原生 Notification。现有 IPC ACL 仍是授权边界，JS 条件
  不是权限证明；未改动系统通知权限，也未在 Computer Use 中操作权限请求。
- 原生示例现在装配同样的 dialog/notification 适配层，并检查原生服务已注册、浏览器 Notification 为原生 API。
  扩展 dialog-probe-only 与 managed-popup-only 通过；54 项 native example 单测、desktop check、边界扫描及
  新增全局 shim 防回归 self-test 通过。保留应用显式文件/通知 API，未把真实 OS 通知发送或文件选择描述为已验收。
- 视觉范围仍是 UserReady 和本机 fixture，没有验证真实模型运行中的全套主应用交互、精确 client 最小尺寸、系统
  IME 全矩阵及所有 frame/关闭竞态。本次发现与修复不能当作 Windows 整体交付；既有总清单和用户上传待确认保持有效。

第七十八个切片：共用 Headless 页面执行器与渲染基础（2026-09-15）：

- 检查确认 Knowledge 的 typed BrowserRenderContentPort 仍未接 canonical Provider；不绕过 Provider lock 直接注入
  HTTP 或浏览器实现。先将现有 search_runtime 物理整理为 headless_page，并更新应用/Provider/探针引用，不保留旧
  模块别名；本地搜索实际使用此共用实现，Tool 名 nomi_local_websearch 与工作台可选性不变。
- 新增 RenderContent purpose 和 typed render_content 引擎入口。检索保持精确 HTTPS 来源集合和固定域名公网 DNS；
  渲染允许公共跨来源 GET，但每次实际请求仍由 SafeHttpClient 做系统 DNS、全地址公网校验与连接 pinning，不能
  接触会话浏览器、用户 Profile 或旧 vault。所有生产页面请求要求 pinned browser product，拒绝模糊 purpose/来源组合。
- 渲染使用固定隔离世界脚本读取实际 DOM，等待已在途网络请求及有界 DOM quiet，最多返回 256 KiB UTF-8 HTML；
  截断不切断多字节字符，明确 html_truncated，并复查 final_url 与原生 frame tree 一致。跨来源请求保留浏览器计算的
  Origin/Referer；共享生命周期继续等待浏览器进程树与自有临时 Profile 清理后返回。
- 真实 Chrome 的 5 项本机网络/隔离验证通过，包括延迟 700ms 的跨来源 JS 中文内容、CORS Origin 转发、生产策略
  私网请求未发出、来源隔离、版本绑定及取消/调用方 abort 后清理。3 项引擎策略单测、3 项 JS 快照/UTF-8 单测、
  8 项本地检索单测、desktop check、browser boundary 与 whitespace 检查通过；本地检索公网回归返回 3 条来源。
- 新增公网 render_content 验收但本机未通过：example.com 被系统 DNS 解析为 198.18.0.225，严格 egress 因而返回
  Blocked。保留该失败和需公网环境的 ignored 测试；没有允许 Fake-IP、改用不校验连接或自动绕过已拒绝的 DNS 结果。
  该环境差异与使用固定域名公网 DNS 的搜索路径区分记录，不把本机 fixture 通过当作公网渲染完成。
- Knowledge 正式 Provider/非 Agent operation 接线、后台 admission/governor、旧 Hub 完全淘汰仍未完成，当前
  rendered Knowledge 调用仍明确 unavailable。本切片不宣称知识库渲染可用，也不缩减既有 Windows、文件、跨 frame、
  发行和 macOS 移交等后续交付清单。

第七十九个切片：Knowledge 非 Agent Provider 接线与有界渲染生命周期（2026-09-15）：

- Knowledge 的 typed BrowserRenderContentPort 已接 NomiCore Kernel invoke_role_tool，再经 Wave2 operation host
  调用应用拥有的 HeadlessRenderRuntime。使用真实 Knowledge service principal，不伪造 AgentSession、Snapshot、
  Workspace 资源绑定，不恢复 Hub 或 HTTP fallback；没有新增浏览器产品面板或测试流程。
- 装配冻结 exact Browser Provider/source，每次调用只刷新 registry generation/digest 再准入；源身份包含浏览器
  product、binary digest、引擎与宿主实现摘要。Provider 变动拒绝旧绑定，不静默重选。运行前复核安装文件和实际 product。
- Windows 的运行时供应模块物理更名为 headless_browser_runtime，同一已验证发行版本供检索与 Knowledge 使用；
  公共 setter 更名为 set_browser_release，不保留旧别名。发现安装只读元数据，不在启动时执行浏览器或发送检索。
  canonical render 输入仅 URL，输出固定 final_url/html/html_truncated；HTML→Markdown 仍由 Knowledge 负责。
- 启动恢复抓取从 AppServices 构造移至 Kernel/Provider 成功装配后，避免渲染来源提前使用默认 unavailable port。
  HeadlessRenderRuntime 最多同时执行 2 个任务，总在途/排队上限 16，排队最多 30 秒，既有四来源批量抓取可以等待，
  不再因仅有两个并发槽而立刻失败。调用方取消保留 job handle，明确 join 后才退休；shutdown 取消并等待全部任务。
- 本轮复查补齐失败队列保护：清理失败或引擎 panic 在释放并发槽前关闭准入并取消其他任务，不让等待任务继续启动。
  失败记录保留，shutdown 继续报 Cleanup，不将未知进程清理状态当成功。新增同时覆盖错误与 panic 的回归测试。
- 5 项渲染 runtime 测试、4 项 Knowledge Kernel/应用组合测试、16 项 Wave2 契约测试和 5 项 startup smoke 通过。
  实际 Knowledge 快照落库使用 recording engine，证明应用接线而非真实公网渲染。真实 Chrome 另外验证私网来源拒绝且
  本机 HTTP trap 无请求，以及桌面供应进入真实能力目录，两项通过。desktop binary check、contract check、浏览器
  边界 self-test/扫描与 whitespace 检查通过；既有原生探针 dead-code 和 opusic-sys PDB 告警未作为已修复项。
- NomiCore 的正式渲染调用链已装配；缺少验证运行时时仍不可用。公网渲染正向验收继续受本机 Fake-IP 系统 DNS 阻断，
  未放宽网络策略；Fresh-v4 Browser ports、其他后台消费者及旧 Hub 退役仍未完成。原生拖放、主应用完整 Agent 闭环、
  文件交互、发行验收和 macOS 移交等后续任务保留，不把本切片当整体交付。

第八十个切片：移除旧 Headless 的应用接线与生命周期设施（2026-09-15）：

- 用户再次明确旧 Headless 已过时，不允许继续用作后台过渡。撤销本轮最初保留后台 Hub 的处理：旧 Headless
  Provider 已从 Nomi 工厂删除。普通会话必须使用 Native Workspace；频道/cron/execution 的 Browser 自动化在新
  owner 尚未实现时明确不可用，不恢复旧引擎。不影响独立的新 nomi_local_websearch 与 Knowledge 渲染入口。
- BrowserRuntimeResolver 始终根据持久 Conversation 的 owner/source/cron/execution 进行分类，不再把缺少原生
  Workspace 等同于 Headless。无原生宿主且未选择 Browser 时普通聊天可继续；选择 Browser 时构建失败，不启动
  私人 Chromium。新请求/返回类型取代原来的 Option resolver，无旧类型别名。
- 物理删除 App 的 browser_lane_provider.rs（含独立租约清理 executor 与专属测试）、工厂 issuer/slot/context
  契约和导出。AppServices 删除 Hub 构造/注入、旧 Profile 恢复入口、机器容量策略、资源采样与库存转发、生命周期
  supervisor/关闭协调器和专属测试。删除旧 Lane conversation cascade，保留 Native Workspace 的 before-delete
  清理屏障。启动不再创建或扫描旧 Hub Profile 根，已有磁盘数据不删除、不搬迁。
- 保留通用后台任务 registry、Gateway 单飞关闭、Agent 停止及数据库关闭保护。14 项应用服务测试、2 项 Browser
  HTTP/装配测试通过。2 项 runtime target 测试明确验证 native 缺失不得降级及后台不得进入旧 Headless；9 项原生
  Agent 生命周期测试、desktop binary check 通过。此前 6 项工厂集成和 75 项工厂单测通过，最终退役后的回归继续记录。
- 浏览器边界 gate 从“仅允许一个旧 Hub 构造点”改为“禁止所有应用生产构造”，并禁止恢复已删除的租约 Provider；
  self-test、扫描与 whitespace 检查通过。旧底层 Hub/Lane facade、Nomi runtime 中尚未清除的失效 binding 清理类型
  与测试仍需物理淘汰，不能将“已断开应用接线”当作全部旧代码已清空。
- 重新编译并执行真实 child-WebView2 `--agent-only` 验收通过：28 次本机脚本模型调用，自动打开、显示后输入、
  同页观察/输入/诊断、terminal 后解锁、Stop 导航、下一 turn 同页 fresh observe、上传快照和活动退出清理均有结果。
  这不是公网真实模型验收；既有关闭后 background task registration 告警仍出现，未宣称已解决。完整跨进程拖放失败项保留。
- 最终退役后的 4 项 Knowledge Kernel/快照回归与 6 项工厂集成测试通过，确认新渲染与普通聊天不依赖旧 Hub；
  私网 Chrome ignored case 本切片未重跑，不能把普通测试中的 ignored 记为通过。
- 整体交付清单不缩减：新后台自动化的 v2 owner、原生拖放、真实模型前端闭环、完整文件/主应用交互、发行验收和
  macOS 移交等仍未完成。本轮不增加任何用户测试产品界面。

第八十一个切片：删除 Nomi 运行时旧 Lane binding 与桌面旧适配依赖（2026-09-15）：

- Nomi manager、HostWiring、TurnTerminationGuard、kill/Drop/unwind/teardown 中的 BrowserLaneBinding 字段与
  租约撤销/关闭分支已物理删除；删除对应旧租约 fixture 与测试，保留独立的 SSH、MCP、进程、后台任务清理覆盖。
  原生 Workspace 的 settle→terminal→finish 及异常时保留输入锁逻辑不变，没有用新的“接管”或租约概念替代旧代码。
- 删除 factory/browser_lane.rs 和导出，删除 NomiResolvedConfig 中已无作用的 browser_use 字段及调用方配置。
  工厂的能力选择仍由 canonical projection 与明确的 BrowserRuntimeTarget 校验，不能从配置重新启用旧后台工具。
  增加真实 .nomi.toml 解析→Nomi manager 构造的回归：先证明项目开关确实启用，再验证工具注册表没有旧 Browser。
- App 与 AI Agent crate 删除 nomi-browser 直接依赖，移除 nomi-agent/browser-use 特性传播；App 同时删除已无读取者
  的 sysinfo 和 nomi-process-runtime 直接依赖及旧 Hub 说明。桌面 normal dependency tree 检查确认没有旧 nomi-browser
  facade；新 nomi-browser-engine 的原生语义与隔离 search/render 仍保留，不把旧 facade 和新引擎混为一谈。
- 删除 binding 后 AI Agent 完整单测 544 项通过、1 项忽略。4 项 Agent 类型集成、6 项工厂集成与 desktop binary
  check 通过；新增旧模块/类型禁止恢复的 browser boundary self-test、扫描与 whitespace 检查通过。
  当前依赖配置的完整单测与真实 WebView2 回归继续在本切片末尾记录。
- 低层 nomi-agent bootstrap 的旧可选 feature、nomi-browser facade/managed adapter、平台 Hub/Lane 核心仍在仓库中，
  下一步继续物理删除，不能因为桌面不再链接就宣布“零旧代码”。新后台自动化与原生/发行的其余总清单不变。
- 最终依赖配置下完整 AI Agent 单测再次为 544 通过、1 忽略；重新编译的真实 WebView2 `--agent-only` 再次通过
  28 次本机脚本模型调用，覆盖自动打开、同页操作、Stop/terminal/解锁、下一 turn fresh observe、上传及退出清理。
  既有关闭后后台注册告警仍存在；此次不将脚本模型验收当作真实公网模型闭环或完整原生矩阵完成。

第八十二个切片：物理删除旧 facade、bootstrap 与平台 Hub/Lane 核心（2026-09-15）：

- 核对确认 nomi-browser 在仓库外部已无 Rust 调用方，删除整个 crate 的 18 个源码/测试/HTML fixture 文件、
  workspace 依赖项及仅剩的空目录；不删除任何用户 Profile 或其他磁盘数据。旧 BrowserTool、managed adapter、
  租约 facade、站点记忆、截图/SoM 定位兜底不再作为可恢复的生产实现存在，源码仍可从 Git 历史查看。
- nomi-agent bootstrap 删除 Browser 注册、Lane client setter、extract/visual model adapters 及其专属测试；
  删除 browser-use feature 和关联依赖，不保留空 feature 别名。通用系统提示词删除旧 Browser 开关/缓存/提示词段，
  更新 71 处调用（含测试），不是把一个永远 false 的兼容参数继续向下传递。MCP、Computer 和普通提示词逻辑保留。
- 删除 nomi-config BrowserConfig、默认来源、browser_data_dir API 与旧合并规则。配置合并的通用测试改用仍受支持
  的 Computer 字段，保留标量覆盖/安全布尔合并验证；负向回归确认旧 TOML 字段不进入解析配置、也不创建旧 Browser。
- 平台旧 Hub/Lane 已无外部类型依赖，删除 clock/cleanup_budget/driver/error/hub/identity/lease/lifecycle/model/
  resource/scheduler 共 11 个旧模块及内部测试。lib 只导出六个新模块，删除无用 tracing/uuid 依赖；新原生生命周期、
  Workspace 权限、上传与 URL 安全逻辑不变，32 项平台测试通过。
- 171 项配置单测、629 项 Agent engine 单测和 544 项应用 Agent 单测通过（后者另有 1 项 ignored）。Agent engine
  所有 integration targets 编译通过；desktop binary check 通过。一次 Cargo.lock 写入被 Windows 映射占用拒绝，
  确认文件完整且该命令已结束后重试成功，没有删除锁文件、覆盖其他改动或启动重复构建。
- 更新 browser gate：不再要求旧 facade/adapter 存在，而是禁止旧 crate、旧 Hub 类型、旧配置与 bootstrap 构造恢复；
  保留私有引擎、Profile 分配、原生输入、UI/IPC 等原有禁止规则。更新测试脚本与 C1 检查目标为新引擎/平台；
  自测和扫描通过。提示词集成、contract 与真实 WebView2 的最终回归在本切片末尾补记。
- 本轮完成的是旧架构实体删除，不是 Browser Workspace 整体交付。引擎中旧独立入口、剩余启动资源接线仍需核对；
  新后台自动化、完整原生拖放/文件/主应用交互、真实模型开发闭环、发行与 macOS 移交清单继续保留。
- 最终提示词相关六个 integration targets 通过。删除依赖改变了 Cargo.lock 摘要，首次 contract check 因此拒绝旧
  fixture；使用现有 write 生成器更新后 check 通过，仅 5 个摘要关联产物实际变化，并核对 runtime/platform fixture
  的 cargo_lock_digest 等于实际 SHA256，不涉及数据库迁移。真实 child-WebView2 `--agent-only` 重新编译后通过
  28 次本机脚本模型调用与临时 Profile 清理。既有关闭后后台注册告警仍出现，完整拖放失败及真实公网模型验收未解决。

第八十三个切片：退役旧资源供应与修复活动 Agent 退出后的任务注册（2026-09-15）：

- 删除无消费者的 App browser_resource.rs 及专属测试，删除桌面向 NOMIFUN_BUNDLED_CHROME_DIR 写入旧 CfT
  资源路径的启动代码和导出。新安装版本验证/本地检索/渲染供应不读此通道，没有恢复自动发现失败后的旧回退。
  边界扫描新增旧模块与环境变量恢复禁令；未删除用户浏览器、下载资源或 Profile，源码可从 Git 历史恢复。
- 重新执行 --frame-drag-only，当前 Edg/152.0.4191.66 仍明确失败：目标 trusted drop/DataTransfer 成功，源结束
  仅在取消收尾时出现 dropEffect=none；窗口式 controller 的 CompositionController QI 仍为 80004002。未放宽测试。
  核对 [Chromium 当前 InputHandler](https://raw.githubusercontent.com/chromium/chromium/main/content/browser/devtools/protocol/input_handler.cc)
  与 [WebView2 DragStarting](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller5)；
  实际 Edge revision 的公开源码请求为 404，不能据主分支代码宣称精确根因。Composition 仍未验证，不切换宿主、不合成 dragend。
- 给真正的后台注册不变量错误补充静态调用位置，真实 --agent-only 精确定位迟到注册来自 Conversation 完成通知：
  Agent 被停止后准备通知时，背景任务 registry 已关闭；原代码先 spawn 再 register，即使通知会因 shutdown 直接返回，
  仍创建了必须取消/加入收尾的任务。这是生产调用顺序问题，不是浏览器功能需要的新产品面板。
- 将 Conversation/DeliveryNotify 的宿主接口改为提交尚未启动的 Future。BackgroundTaskRegistry 在同一 admission
  mutex 内检查 Open、启动并记录任务；Closing/Closed 时直接拒绝且不启动 Future。已启动任务的 register/abort/join
  保护及真正 post-close 注册错误继续保留。没有把错误降级、吞掉日志或去掉最终数据库关闭屏障。
- 新增关闭中/关闭后拒绝且不 poll Future、128 次提交与 shutdown 并发的两个回归，连同三个原 registry 测试共
  5 项通过。desktop binary check 通过。真实 --agent-only 的 28 次模型协议调用通过，并对全部输出检查确认不再有
  关闭后后台注册错误；原生输入锁、terminal 后解锁、同页后续 turn、上传与退出清理仍通过。
- delivery_notify 名称过滤在 Conversation lib 中命中 0 项，不计为有效测试；随后完整 Conversation 324 项、App service
  16 项与 delivery observer 3 项回归均通过，边界 self-test/扫描及 whitespace 检查通过。跨进程拖放仍未修复；
  引擎旧独立入口、新后台自动化、下载/文件、真实模型前端
  闭环、完整主应用和发行验收继续推进，整体目标未完成。

第八十四个切片：原生另存为基础与文件选择器回归（2026-09-15）：

- 确认页面及 popup 当前仍直接拒绝下载。先扩展既有隔离选择器，不接入旧 Headless 或旧文件 facade；将协议改为
  显式 PickerMode::Open/Save（v2），统一在 IFileDialog 基类配置，单选/保存用 GetResult，多选仍用 GetResults。
  Save 使用 IFileSaveDialog、覆盖确认/只读保护选项及建议文件名检查，拒绝目录分隔、ADS、设备名和越界长度。
  Open 的强制已有文件、扩展名过滤、单/多选、进程树取消及必要环境变量白名单保持有效。
- 更新所有应用/fixture 调用，不保留旧请求形状别名；修正 ADR §12.4 残留的 Browser bottom bar 描述，下载不新增
  底部栏或独立面板。新增模式/文件名/协议负例；55 项 native example 单测、desktop check、browser boundary
  self-test/扫描、desktop UI boundary（880x600）与 whitespace 检查通过。
- 扩展 --picker-only：真实 Save 对话框已打开并取消，确认辅助进程退出且未创建文件；原 Open 独立实例、调用方
  Drop 和提前取消仍通过。首次 --user-files-only 在 frame_navigate 的 chooser 等待失败，原样再跑通过；没有据此
  宣称没有竞态。检查发现 fixture 未等待子框架 ready 消息和有效命中坐标，补齐输入前等待（不重试已发送点击），
  随后三轮均通过全部 hide/navigation/OOPIF/Agent admission/close 取消与零文件交付断言。
- 使用 computer-use 技能在真实系统“另存为”窗口检查临时目录与“下载 验收.txt”，点击保存后 helper 返回精确路径，
  确认进程已退出。验收程序仅以 create_new 写入该临时路径并核对 UTF-8 内容，然后清理；窗口和临时目录均已消失。
  没有覆盖既有文件、执行上传或操作用户其他目录。这是 Save 选择交互证据，不是网络下载内容的证据。
- [WebView2 下载事件](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2downloadstartingeventargs)
  支持延期及结果路径设置，设置路径可能覆盖已有文件；正式接线还必须处理真实 DownloadOperation 生命周期、取消/关闭、
  沙箱与发布。当前网页下载拒绝逻辑尚未移除，Agent download 也未实现，不把本切片当作下载能力已交付。
- 下一步继续接原生下载事件与保存 broker，取得实际下载文件/终态证据；跨进程拖放、用户上传正常选择待确认、其余
  原生/主应用/真实模型/发行和 macOS 移交任务均保持原范围。整体目标未完成。

第八十五个切片：同页 UserReady 原生下载与实际传输验收（2026-09-15）：

- 上两个目标续行被主动中断，没有运行中的构建或已启动下载任务；本轮重新核对进程与源码后继续，未重复启动未知任务。
- 新增 Windows user_downloads broker，移除 root/popup 的统一 on_download(false)，由同一 WebView2 的 DownloadStarting
  接管。事件默认 Cancel/Handled，COM args、deferral、operation 和订阅仅留在 UI 线程。可见/空闲/未关闭时才打开
  隔离 Save picker；保存选择后再复查文档 target、取消标记和运行锁，才设置真实目标路径并完成 deferral。
- 每 Tab 四个在途任务、一个待选择保存窗口；完成任务可回收，失败清理保留重试权。隐藏/导航/Agent 入场取消未完成
  选择并等待 helper 退出，已开始的用户下载不因 Agent 入场取消；关闭 Tab 取消所有传输并等待 COMPLETED 或显式
  USER_CANCELED 确认，防止把可能自动恢复的普通 INTERRUPTED 当作取消证明。Drop 仅请求取消，不提供成功证明。
- root 与 popup 均接入安装、显示、导航、输入锁和关闭钩子；默认下载浮层被隐藏，不增加运行锁外的交互窗口，也不增加
  测试产品面板。Agent 新下载仍拒绝，未用用户 Save picker 代替 Agent 下载沙箱。
- 原生 --user-downloads-only 验证取消/hide/Agent/navigation/close，全部通过。使用 computer-use 在真实保存窗口选择
  本机临时中文路径；WebView2 从本机 HTTP attachment 保存的 UTF-8 文件与响应字节一致，不是测试程序代写下载内容。
  进行中响应另验证：保存后原生状态确为 in progress，Agent 入场不打断它，之后关闭页面取消并移除 native view。
  临时文件、目录和保存窗口均已确认清理。
- 曾将两次人工选择合并验收而触发外层超时，未计为通过；确认进程已结束、临时目录已清理后拆成独立模式，并增加
  fixture 内层超时后的显式清理。UIA SetValue 因缓存属性缺失失败，重新观察后使用原生键盘输入，不复用失败前的状态。
- 57 项 native example 单测、desktop check、取消矩阵、28 次脚本模型 --agent-only、browser boundary 自测/扫描和
  whitespace 检查通过，既有关闭后后台任务注册错误未再出现。手动完整下载与慢传输取消是本机 fixture 证据，不等于
  已验证全部站点和所有失败路径。
- 用户下载的基础传输链路已实现；状态菜单、直接附件导航/popup/Blob 等完整矩阵、浏览器崩溃与网络失败/配额边界、
  Agent download 的沙箱/文件发布继续实施。跨进程拖放、真实模型前端闭环、其余 UI/发行和 macOS 移交范围不缩减。

第八十六个切片：现有浏览器菜单内的下载状态与取消（2026-09-15）：

- 仅扩展已有浏览器菜单，显示文件名、下载状态、已接收字节和取消按钮；不新增面板、设置入口、测试步骤或数据库记录。
  运行中所有下载操作受既有 Agent 输入锁约束，没有接管入口；取消失败只显示提示，不让整个浏览器变成不可用页面。
- Windows runtime 持有最多 64 条会话内记录；关闭单个 Tab 后终态记录仍可查看，浏览器 runtime 关闭后释放。
  淘汰旧终态而非在途记录，状态变化复用现有 revision 推送。文件名按纯文本显示，快照不包含下载 URL 或磁盘目标路径。
- WebView2 状态与字节事件进入同一记录；完成/取消的终态在原生清理成功后发布，清理失败保留重试权。
  用户取消验证当前 Tab/document target 和运行锁，随后等待原生清理；菜单遮挡原生表面不影响取消权限。
  Agent 不能调用用户下载取消命令。增加完成与取消并发时终态不倒退的保护。
- UI 类型检查、35 项 BrowserWorkspacePanel 交互测试、4 项 Windows 下载单测、用户/Agent 取消权限测试及 desktop binary
  编译检查通过；desktop UI boundary、browser boundary
  自测/扫描、i18n 检查通过。首次从根目录运行 UI 测试未加载 ui/bunfig.toml 的 DOM preload，失败不计为产品证据；
  使用仓库正确的 `bun test --cwd ui ...` 入口重跑全部通过。
- 原生 `--user-downloads-only` 通过取消/hide/Agent/navigation/close 矩阵；取消阶段改为从 runtime 下载快照取 id，
  调用真实用户命令并验证返回的是不可再次取消的 Cancelled 记录，保存窗口与临时 Profile 清理完成。
  本切片未重复人工保存验收，也不将待选择窗口取消当成已验证活动传输的菜单取消。
- 未扩展产品范围。Agent download 沙箱/文件发布、剩余下载矩阵与既有 Windows 交付缺口继续按原清单处理。

第八十七个切片：Agent 原生下载、任务文件发布与 Stop 收尾（2026-09-15）：

- `browser.download` 现在接入正式 Nomi Browser 工具注册与授权工作区 scope；只有同时具备冻结能力和宿主 scope 时
  才公布 download 操作，额外磁盘路径输入被严格拒绝。Agent 工作台复用原有能力项，仅补充清晰的下载边界文案。
- download 点击当前 observation 的真实元素，不注入隐藏 anchor，不用另一 HTTP 客户端，不开放普通 act 的下载权限。
  与用户共用同一 WebView2 下载事件 broker；一次明确调用最多接一个同页事件，不弹出用户保存窗口。
- 新 scope 以目录句柄锚定已授权 workspace，阻止 downloads junction/symlink 绕过。传输进入私有临时目录；
  原生 COMPLETED/清理后复用引擎扩展名和 magic bytes 规则校验，以同卷临时文件、Windows MOTW、原子无覆盖硬链接
  完成发布与计费，返回相对路径、字节、SHA-256。没有不支持硬链接时的检查后覆盖式 rename fallback。
  单文件 512 MiB、scope 总量 1 GiB/256 文件、四个在途任务；失败残留保留计费与清理责任，并发发布只允许一个成功。
- 下载沿既有 pending-work/网站对话框与 run guard 生命周期执行。Stop 取消并等待原生传输；释放输入前再次清理 Agent
  下载和保留的临时文件。清理不确定不会删除仍在写入的目录，也不会解锁；不影响已授权且进行中的用户下载。
- 实际 Windows `--agent-downloads-only` 已通过 production Tool→run→native click→本机 HTTP→文件发布：连续五次
  下载不覆盖，内容与返回证明吻合；可执行扩展名、MZ 伪装文本和超大 Content-Length 被拒绝；慢传输 Stop 后无新增
  产物/部分文件、无在途任务，保存弹窗从未出现。fixture workspace 和 Profile 清理成功。
- 60 项 native example 单测、5 项 scope/发布测试、10 项 Agent 工作台文案/配置测试及 UI 类型检查通过；
  加强收尾后再次通过 Agent 原生下载矩阵、UserReady 下载取消矩阵和 desktop binary 编译检查。
  desktop UI boundary、browser boundary 自测/扫描与 whitespace 检查通过。
- 本切片没有验证所有站点、popup/Blob/直接附件导航、文件系统故障注入、下载中崩溃及真实模型完整开发闭环；
  这些仍按原交付清单继续。macOS 仍留待用户转交，不将 Windows 的 MOTW 或原生事件实现冒充 Mac 验收。

第八十八个切片：截断响应的原生失败收尾与同页恢复（2026-09-15）：

- 增加真实 HTTP 截断响应：声明 1 MiB，只发送 64 KiB 后断开连接。初次验收超时，后续原生探针明确观察到
  State=IN_PROGRESS、InterruptReason=SERVER_CONTENT_LENGTH_MISMATCH、CanResume=false、BytesReceived=65536；
  Cancel 返回后仍保持这个组合，旧逻辑无法取得终态，因而阻止解锁/关闭。失败轮次未计为通过。
- 修复原生状态判定：正常状态路径不变，只对上述已复现的不可恢复长度不匹配组合触发失败收尾；必须有成功的
  Cancel 回执，且不可恢复，才确认失败而非完成。其余不可恢复中断原因使用明确已知集合，未知值不获得清理证明。
  已收到的终态不再被后续进度事件抹掉；开始明确取消时才清除旧的中断证据。
- [Microsoft 的 CanResume 文档](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2downloadoperation#get_canresume)
  明确列出三类可能自动恢复的中断。本实现继续排除 SERVER_NO_RANGE、FILE_HASH_MISMATCH、FILE_TOO_SHORT，
  不以 CanResume=false、超时、部分文件大小或任意错误码单独证明取消。
- 测试补充精确失败状态、15 秒诊断超时后的取消/等待、失败后在同一 Tab 再次成功下载，以及已有 Stop 清理。
  原生矩阵通过六次独立文件发布（包含网络失败后的恢复）、危险内容/超大响应拒绝、失败无部分产物和最终解锁。
  UserReady 下载取消矩阵、五项下载状态单测、desktop binary 编译及 browser boundary/whitespace 检查通过。
- 只读失败探针不暴露到 renderer/Agent IPC；COM 查询前释放 registry 借用。最初失败进程已结束，遗留的唯一
  `nomifun-agent-download-py1pgw` 空暂存目录经检查后非递归删除；最终通过轮次的 workspace/Profile 正常清理。
  其余 Blob/popup/直接附件、崩溃与文件系统故障矩阵继续保留，未据此声明完整浏览器交付。

第八十九个切片：当前网页交给系统浏览器（2026-09-15）：

- 现有 Browser 菜单增加“在系统浏览器打开”。请求只携带当前 target，不接受地址栏草稿或任意 URL 参数；宿主从
  原生 WebView Source 读取地址，保留 query/fragment，拒绝非 HTTP(S)、凭证 URL、控制字符与过长输入。
- 操作只属于用户，复用 run guard，并在原生线程复核 target、取消和输入锁；非当前 Tab、过期 target、Agent
  命令均拒绝。菜单临时遮挡原生页面不会使合法操作失效。Windows 使用原生 ShellExecuteExW，不调用命令 shell，
  不复制 Cookie/Profile、不关闭或替换内嵌页面；只报告 OS 已接收打开请求，不承诺外部页面加载成功。
- 正常失败只产生菜单提示，无自动重试或全页不可用状态。38 项 BrowserWorkspacePanel 测试、UI 类型检查、i18n、
  desktop UI boundary 与 browser boundary 通过；两项原生 URL 校验单测结果在下一续行收取，确认通过。
  i18n 生成首次遇到 Windows 临时文件占用，未删除原文件，重试生成成功。
- Windows `--external-browser-only` 已实际通过：默认 Microsoft Edge 请求了本机临时页面，内嵌页面保持原 target/URL；
  运行中、非当前标签与过期 target 的请求均未触发打开。首次 fixture 用 about:blank 创建第二页，被正常 URL 策略拒绝；
  修正 fixture 使用本机 HTTP 页面后通过，没有放宽产品 URL 策略。
- computer-use 技能在核对外部浏览器 URL 时触发安全停止，未执行关闭或其他界面操作。验收标签
  `NomiFun external browser check` 尚未由本任务关闭；不通过其他接口绕过该停止。Rust 测试已正常结束，未重启未知构建。
  完整原生浏览器、其余菜单能力及跨平台交付仍未宣称完成。

第九十个切片：前端 Blob 导出、重定向与下载弹窗的原生生命周期（2026-09-15）：

- 补充真实 Blob CSV、空 Blob、HTTP redirect 和 target=_blank attachment 验收；下载字节与 SHA-256 必须吻合，
  伪装成文本的可执行 Blob 必须拒绝，失败记录必须属于本次新任务。前端不新增导出/测试产品流程。
- target=_blank 暴露了两个真实缺口：原下载授权没有随这次弹窗传递；Wry 0.55.1 自行 DestroyWindow，导致下载
  COM operation 尚未结束就失效。失败轮次保留为诊断证据，没有降级成 HTTP 侧下载或放宽产物校验。
- 原生 NewWindowRequested 同步捕获这一次 AgentRequest；异步创建完成后只向它创建的子页转交授权，重新校验
  opener/child target、锁和取消。父子共享一次原子领取，旧 target 不消耗额度，完成/取消后清除所有相关待领取入口。
  清理重试根据实际下载所属 Tab 查找宿主，不误用 opener。新增两项一次性领取/过期授权单测。
- 校验原 crates.io 包 SHA-256 后，将 wry 0.55.1 固定在 vendor/wry，保留上游许可证与源码来源说明。
  唯一上游源码改动是 Windows child view 不注册默认 DestroyWindow 回调；顶层窗口及其他平台源码不变。
  NomiFun 原有 WindowCloseRequested 订阅接管关闭：页面自行关闭先等待已授权 Agent 下载终态，显式关闭和 Stop
  仍取消并等待。源码范围及退出该补丁的条件见 vendor/wry/NOMIFUN-PATCH.md，无事件 token 猜测、API 拦截或旧 Headless。
- 补齐关闭与新操作交错：已进入关闭阶段的子页不再接收排队的布局更新或对话框策略恢复；下载弹窗自动退役后，
  opener 可以立即继续操作。原生矩阵验证十次独立发布，并明确检查下载-only popup 自动消失、原页保留。
- 65 项 native example 单测、shared run authority 测试通过；实际 Agent download、managed popup、raw popup、
  website dialog/导航/关闭矩阵通过。UserReady 取消矩阵曾在并行构建期间未观察到 Save picker；构建结束后原样重跑
  通过，未将这次瞬态失败归因为已修复，也未放宽产品策略或反复点击。
- Cargo.lock 因固定 wry 来源变化；通过已有 agent-v2-contract write 更新派生证明，再运行 check 通过，不修改数据库。
  desktop binary 编译、desktop UI boundary、browser boundary 与 whitespace 检查通过。核对 vendored 源码差异，
  仅上述 Windows child 生命周期两处修改。先前外部浏览器验收标签未再操作。
  仍须完成其余故障/跨 frame/真实模型/发行与平台验收；本切片不是整个重构完成声明。

第九十一个切片：真实模型的原生前端修复与复测（2026-09-15）：

- 正式 product API→AgentPreset→资源选择→Nomi loop→同会话 WebView2 的真实模型验收已通过。
  使用现有专用 StepFun Coding Plan 验收凭据与 step-3.7-flash，不使用脚本模型响应。
  临时应用的计数函数故意每次加 2；模型先在真实浏览器复现，再用工作区文件工具修复 app.js，刷新后点击并验证 1、2、3。
  页面独立记录 isTrusted 点击及文档 generation，服务器记录实际重新提供的源码；最终检查真实 DOM 与原生 HWND
  可见/输入锁状态，不以模型口头“完成”作为证据。前后使用不同文档 generation，源码确实改变并重新加载。
- 不创建测试产品面板。验收代码和数据在独立临时 workspace/backend/Profile 中，完成后关闭后端和原生页面并清理；
  不读取或修改用户现有项目文件和会话。该证据覆盖这个小应用及当前模型，不扩展为所有项目、所有模型或正式安装包已验证。
- 初次本地创建 Session 被 422 拒绝，原因是启用文件能力却未显式选择 workspace。改为使用产品 resource_selections
  选择服务器提供的 default-workspace，未构造假 TypedResourceBinding 或放宽权限校验。
- 真实模型随后多次在 Browser 参数阶段失败，没有产生有效点击。Browser 工具现在同时提供对象层级的操作枚举与
  动作字段提示，元素参数明确要求复制完整 reference；原始 oneOf 仍作为严格校验保留。没有新增宽松命令、旧参数别名
  或绕过过期引用。单测通过实际 ToolRegistry 校验正确输入，并拒绝缺字段、额外字段和用户专属 open_external。
- 真实原生验收首次成功退出后，外层脚本误从 stderr 查找 stdout 成功记录，未计为外层通过；修正通道并添加解析自测，
  再次实际运行完整流程通过：browser_live_frontend_status=pass，native_click/workspace_fix/retest/terminal_unlock 均有证据。
  先前失败尝试不计为通过。
- 复用现有凭据隔离运行器，新增 --browser / PowerShell -Browser 入口；默认普通 live smoke 行为不变。
  凭据仅由可信启动器临时接收，从后续环境移除，编译进程与工具子进程不继承；验收进程只经 stdin 接收。
  失败诊断只保留有界数字、布尔值和枚举工具名，双重过滤后才输出，不打印模型正文、参数、文件内容或凭据。
- 11 项 Browser Tool 单测（含严格 registry 校验）、两种运行器自测、28 次本地脚本模型正式接线回归、browser boundary
  和 whitespace 检查通过。新增测试用 zeroize 依赖后，通过既有生成器更新派生证明并 check；不迁移数据库。
  真实模型基本闭环已取得证据；完整主应用 UI、跨进程拖放、网络策略、其余故障/发行与 macOS 移交仍待完成。

验收入口（Windows，已配置专用凭据时）：

```powershell
powershell.exe -NoLogo -NoProfile -File scripts/validation/run-nomi-core-live-provider-from-windows-credential-manager.ps1 -Browser
```

第九十二个切片：按文件边界并发推进与统一集成（2026-09-15）：

- 用户明确允许无编辑冲突的并发。分工为前端独占编辑、旧引擎只读审计后独占抽离、网络边界只读研究；主线保留
  原生运行时、Cargo 和原生验收的修改/执行权。各任务无交叉写入；Rust 编辑停止后由主线统一编译，不并发争抢原生窗口。
- Browser 菜单“复制页面地址”完成：复用既有剪贴板工具，只复制 active Tab URL，不复制地址栏草稿；运行锁、输入锁
  失败、busy 和草稿页禁用，不重放；失败只显示提示，旧会话迟到结果不污染新会话。45 项 UI 测试（新增 7 项）、i18n、
  desktop UI boundary 通过，主线复核代码并运行 UI 类型检查通过。未增加后端入口或产品面板。
- 旧引擎审计明确新 search/render 仍需要 process cleanup authority；将 HostCleanupLease 抽到独立 cleanup.rs，
  更新 host/CDP/launch/headless_page 与公开导出，删除旧 host 路径定义，不保留兼容别名。两项生命周期/Debug 脱敏单测通过，
  引擎集成测试目标编译通过但未执行其中被过滤的浏览器用例；主线统一运行桌面 bin 的 no-default-features cargo check
  通过（仍有可见性与未使用项警告），git diff --check 通过；这不是宣称整个旧引擎已退役。
- 网络审计核实共享 UDF/environment 不能按会话配置不同代理，proxy auth cache/连接复用也不能充当逐会话隔离。
  已向用户提出“项目共享登录＋项目级网络边界”和“会话独立登录＋会话网络隔离”两种选择；尚未得到选择前不改 Profile
  语义、不编造逐请求/run 网络归属，也不擅自放宽设计要求。
- 主线重新验证跨进程拖放仍失败：目标 trusted drop 成功，源仅在取消收尾获得 dropEffect=none。公开 Chromium
  InputHandler 当前把 drop 与 source-ended 发给目标 widget，与现象一致，但不将其当成实际 Edge revision 的源码证明。
  未合成 dragend、未放宽测试，Composition 输入路径仍需实际原生证明。

第九十三个切片：删除旧浏览器自动供应与补齐 popup 清理责任（2026-09-15）：

- 按互不重叠的文件边界继续并发：引擎退役由独立任务修改，主线修改原生 popup，第二个独立任务只读审查。
  Rust 编辑全部停稳后统一编译，未并发操作原生验收窗口。
- 物理删除旧 acquire.rs（1,944 行）、ChromeSource、bundled_dir、引擎自动发现/下载兜底和 zip 直接依赖。
  现存低层 CDP 构造必须显式供应绝对 executable；默认配置、相对路径、目录、缺失文件在启动前拒绝。
  仅 opt-in conformance helper 读取 NOMIFUN_CHROME_BINARY；新版 desktop verified release 和 search/render 供应不变。
  CDP/Host 的其余退役与新后台消费者仍待完成，不将这一切口称为旧引擎全部清空。
- 修正两个被新 executable 前置校验遮蔽的历史脱敏用例：分别确认真实触达 profile 准备失败和 OS spawn 失败，
  不以 admission 拒绝替代其原验证目标。引擎 lib 测试 745 通过、12 个显式浏览器用例未运行；全部测试目标 cargo check 通过。
  新增退役扫描器 gate 和反例自测，均通过，防止重新引入旧 downloader/来源设置。
- 原生 popup 的拒绝清理不再只在 Drop 中尝试后丢失失败：未确认的 child Close 与 deferral Complete 保留准确对象，
  run settle/close barrier 重试，失败不解锁、不释放 Profile。创建期间的重入拒绝保留请求且禁止后续绑定。
- 独立审查找出并修正候选双重关闭责任：注册到 Runtime 前显式 claim child；随后旧请求仅持有 deferral，
  Stop/expiry/导航不能销毁 Runtime-owned candidate。Runtime 保留并负责统一关闭；不是把 missing WebView 当关闭成功。
- 两项清理责任单测及已有 URI gate 共 3 项通过。新增原生回归覆盖创建前取消、满足输入锁后的创建后取消，
  以及 claim 后取消不能销毁 Runtime-owned candidate；Windows --popup-only 实际通过。
  --managed-popup-only 的会话注册、Agent 输入锁/终态解锁、真实 opener、命名复用、self.close 和配额回归通过；
  --agent-downloads-only 的 redirect/Blob/空文件/popup 附件、网络失败与恢复、Stop 收尾及 10 个唯一发布文件回归通过。
  这些用例均确认其临时 Profile 清理，不等同于完整 popup 创建/关闭故障矩阵已经穷尽。
- 使用显式安装的 Chrome 执行 5 个新版 Headless 本地原生用例通过：绑定验证、跨域动态内容、
  非 adapter 网络拒绝、loopback 拒绝，以及取消/调用者 abort 后准确清理；未重跑公网搜索用例。
  zip 依赖删除导致 Cargo.lock 变化，已通过既有 generator write/check 更新派生合同，不做数据库搬迁。
- 桌面 bin 的 no-default-features cargo check、最终 browser boundary 与 git diff --check 通过；
  仍有已有可见性/未使用项及原生链接 PDB 警告。未进行安装包验收，跨进程拖放仍未解决，网络/Profile 选择仍待用户确认。

第九十四个切片：旧默认 Host 退役与 iframe 诊断归属（2026-09-15）：

- 引擎独占编辑、原生诊断由主线编辑、独立只读审查；全部源码停稳后统一 Cargo，没有交叉编辑。
- 物理删除 create_engine、EngineConfig、默认共享 Profile 解析/冷启信号量、ManagedBrowserHost、StandaloneResourceScope、
  旧动态调额/关闭协调器和 display 探测降级，不通过兼容别名或 cfg 隐藏。CDP 显式构造改为必须接收调用方 task 资源权限；
  测试权限只存在于 conformance helper，不重新引入生产默认签发。删除旧 Host 专属用例，生命周期/调试/互斥与
  精确关闭响应验证改接仍在使用的低层路径；Windows/macOS display 专属依赖移除。708 项引擎 lib 测试通过，
  全部集成目标编译并运行其非 ignored 用例通过；另外显式执行 2 项引擎启动/退出和 5 项新版 Headless 本地原生用例通过。
- 原生诊断沿用当前 WebView 的 iframe auto-attach 事件，不新增 target discovery 或控制入口。
  私有投影按 session、default execution context、context uniqueId、frame loader 和根 document_generation 归属，
  request ID 不跨 session 混用；worker/unowned session 不进入结果。异步 frame tree seed 只补初始缺失元数据，
  不覆盖真实新导航或复活已脱离的子树。上下文/请求/会话/队列均有界。
- 生命周期 COM 读取、事件队列或解析失败使诊断采集 fail-closed，并显式报告 unavailable；不会阻止页面/iframe
  初始化、文件选择器安全配置或恢复执行。清页不把已失败的订阅重新标成可用。不做 DOM getter 求值、请求体或 header 导出。
- 协议字段依据：[Chrome DevTools Runtime 定义](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/js_protocol.pdl)
  与 [Target 定义](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Target.pdl)。
  新增单测及 --diagnostics-only 原生 fixture，后者核对真实同进程/嵌套 OOPIF、逐文档 URL 代数、逐 scope HTTP 503、
  console/exception 和无 getter 执行。10 项诊断单测通过，最终 --diagnostics-only 连续 3 次原生运行通过。
- 原生回归暴露并修正两项时序问题：WebView2 的 Page.frameNavigated 可能先于 native load generation 通知到达；
  使用每 Tab 单个、有界合并请求的 metadata worker，在 native load finished 后读取同一页面 frame tree，补齐缺失 loader，
  不覆盖已观测导航。此前会话回归中的 network 缺失不计为通过，修正后 --managed-popup-only 恢复通过。
  fixture 在主动替换 iframe 时可能遇到旧 context/route 失效；只对有截止时间的只读 readiness 观察重试，
  不重放点击，仍要求新文档 URL 代数和全部真实错误证据。
- --frame-input-only 与正式 Agent 接线 --agent-only（28 次本地脚本模型调用）通过。
  Tool 明示诊断覆盖 root 与已附着 iframe，worker 不包含，dropped/unavailable 不是页面无错误的证明。
  临时排查打印已移除；没有新增终端、控制台、问题或测试步骤产品面板。
- 最终 native smoke 单测 74 项全部通过；桌面 bin no-default-features cargo check、派生合同 check、
  退役边界扫描与 diff --check 通过。仍有已有原生验收 helper/可见性和 PDB 警告；不等同于完整默认 native smoke
  或发行验收通过。跨进程拖放、网络/Profile 决策、其他 Browser Role/后台消费者和主应用/安装包工作仍未全部完成。

第九十五个切片：Composition 原生拖放替代路线的可复现验证（2026-09-15）：

- 聚焦完整验收仍失败的跨进程 HTML drag，没有修改生产 Browser host。独立任务只创建隔离 probe 文件，主线负责
  ABI 核对、集成和原生运行；同一时刻只有一个原生验收窗口。
- 新增测试专用 browser_composition_probe.rs：沿用 fixture 的真实 WebView2 Environment，创建 DComposition
  child HWND、CompositionController 和 native visual；无 JPEG/截图表面、DOM 合成输入或全局系统鼠标操作。
  --composition-only 实际验证可信 SendMouseInput 点击、HTTP iframe 页面导航、明确 controller Close 和临时 Profile 清理。
  一次过早点击失败后，增加输入前真实动画帧完成等待，未重试点击；最终基础 probe 通过。
- 从微软官方 Microsoft.Web.WebView2 1.0.3719.77 NuGet 的 WebView2.h（仅内存读取）取得精确 GUID/vtable，
  补充 0.38.2 尚未声明的 CompositionController5/DragStartingEventArgs/handler。补充模块只用于 example，未升级
  生产 Wry 或 WebView2 绑定。IUnknown QI/引用计数测试通过；独立审查未发现 ABI 次序错误，并修正创建失败时
  先返回错误再 best-effort 清理的问题：错误回执前明确清理，未确认关闭的对象保留供重试。
- 按[微软 CompositionController3 文档](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller3)
  注册本 probe HWND 的 OLE DropTarget，转发客户端/屏幕坐标；显式 AllowExternalDrop=true。使用
  [DragStarting 的异步顺序](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller5)：
  获取 deferral 和原始 IDataObject，执行原生逻辑，再 Handled=true 与 Complete，不进入默认全局 DoDragDrop。
- --composition-drag-only 与 --composition-drag-target-entry-only 保持真实失败：源 OOPIF 由该 WebView 的
  iframe attach 事件确认，源/目标都可达且有可信 pointer 输入，DragStarting 实际触发、allowed=3；
  当前 native forwarding 的 Drop 返回 0，目标未获得 drop，源 dragend 为 none。保持非零退出和完整证据，
  没有合成 dragend、重建 DataTransfer、关闭站点隔离或把失败当成 supported。
- 结论仅限当前实验路径：尚不足以替换生产宿主，跨进程拖放没有解决。保留最小复现以继续核对原生 SDK 行为；
  这不是完成整个重构，也不是断言所有 Composition/OLE 实现均不可行。新增 DirectComposition/windows-core
  依赖仅在 Windows dev-dependencies，macOS 和生产产品模式没有被本实验更改。
- 当前实测 Runtime 为 Edg/152.0.4191.66，revision @cc2931e6363af1d70882ad63ee33b0e8cd524de0。
  另核对官方 SDK 1.0.4191.47：相关接口仍为 CompositionController5/DragStartingEventArgs，未发现更新的同名
  拖放完成接口。补齐 probe 的父窗口位置通知后，直线路径仍失败。基础 probe、ABI 引用计数单测、
  派生合同 write/check、边界扫描与 diff --check 已通过；拖放失败状态未改写。

第九十六个切片：追加独立系统浏览器能力并撤回误向清理（2026-09-15）：

- 用户连续确认：要操作正在使用、已登录的真实系统浏览器，而且属于另一套独立能力，不是内嵌浏览器的模式。
  主 ADR 已修订原生 Profile/输入锁/消费者路由的适用边界，并新增 `2026-09-15-system-browser.zh.md`。
  系统浏览器能力仍未实现，不能把旧 headful Chrome 或 URL 外部打开当成交付。
- 立即中断外部可见窗口接口退役。独立任务只用 apply_patch 精确撤回该轮七文件删除，十二处原块与保存内容一致，
  `integration_takeover.rs` 原 blob 哈希一致；没有 checkout/reset、全文件回滚，也没有恢复此前已删除的
  ManagedBrowserHost、自动供应、vault 等旧代码。恢复后 `cargo check -p nomi-browser-engine --tests` 通过。
- 核实 Chrome 官方现状：144+ 支持在正在运行的个人会话中启用并授权 auto-connect，无需新 Profile/重启；
  浏览器侧授权范围是选中 Profile 的所有窗口，NomiFun 必须另行限定 Agent 标签目标与输出。首选验证此官方连接
  机制和固定版本适配器，不先建设自有扩展平台。尚未启动任何个人浏览器连接、授权请求或读取个人标签内容。
- 明确新宿主只拥有调试连接，不拥有用户浏览器进程/Profile，退出必须 detach/disconnect；不清理、不迁移、不复制
  用户登录数据，不自动 fallback 为独立 Chrome。系统浏览器无法照搬原生 WebView 的物理输入锁，外部干预/撤销的
  中断边界在补充设计中显式说明，不能伪称可强制锁住浏览器地址栏。
- 已询问用户优先连接 Chrome 还是 Edge；这是确定首个真实连接验收目标，不新增测试模式或控制台产品。
  Windows 先行、macOS 移交、Linux 可后置与原重构全部未完成交付项继续保留；追加能力不能悄悄从最终范围删除。
- 恢复后的桌面 bin no-default-features cargo check 与 diff --check 通过；未运行个人浏览器连接验收，
  未安装扩展、修改用户浏览器配置或读取个人页面。旧 headful helper 的恢复不能作为新能力已交付的证据。

第九十七个切片：关闭全部内嵌标签与关闭后协议屏障（2026-09-15）：

- 完成 §4.4 已有菜单项“关闭全部标签页”：前端发送 close_all + 当前 runtime_generation，运行锁、输入异常、
  busy 或无页面时禁用；成功清除地址草稿，保留 Work Surface 和现有空状态。无确认向导、测试面板或自动重试。
  UI 独占编辑与后端主线分工无冲突，52 项 UI 测试、类型检查、i18n 与 desktop UI boundary 通过。
- 命令仅对用户开放，不进入 Agent schema，agent_command 明确拒绝；旧 generation 和无 Runtime 请求不创建新
  Runtime。保留 Runtime/Profile/下载历史，仅复用现有原生退役流程关闭本 Workspace 页面；失败保留准确对象供重试。
  关闭前取消未完成页面工作、处理网站对话框并等待创建 guard，全部 opener 先失效，避免迟到 popup 重新建页。
- 发现并修正用户命令排队越过 Agent run 的时序缺口：入口即拒绝运行中用户请求，执行前再核对 run revision；
  不能等 Agent 结束后自动执行旧关闭/导航意图。平台 40 项测试通过，包含即时拒绝、跨 revision 拒绝与部分关闭失败重试。
- --close-all-only 真实 WebView2 场景通过：多 Tab、已注册 popup、网站 confirm、创建中的慢导航，旧 generation/
  Agent 路径拒绝，另一会话实例保持，关闭后在同 Runtime 中新建页，验证持久 Cookie/localStorage 仍在。
  验证的是持久 Profile 数据，不把已关闭页面的表单、history 或 sessionStorage 宣称为保留。
  尚未用原生故障注入穷尽 popup 正在创建及部分 Close 失败的矩阵。
- 扩展回归发现旧单页关闭会在另一页保留 dialog 时卡于已关闭页的 pending input join。隔离 TRACE 显示 native
  retirement 已结束，随后清理 CDP 可能在 Close 与 Tauri 注销之间提交且永无回调。新增只覆盖该间隙的 CLOSED_VIEWS
  屏障，协议入口及 UI 实际提交前均检查存活性；注销确认后移除 marker，注销失败保留 Close 证明供重试。
  unclaimed popup 同样移除已确认注销的 marker，不累计正常关闭历史。重试仍结算子 popup，不跳过其清理。
- 修正后 --dialog-close-only 和完整 --dialog-probe-only 通过，另一页 dialog 保持且旧句柄协议请求立即拒绝；
  --popup-only、--close-all-only、--agent-only（28 次本地脚本模型调用）与 --agent-downloads-only 回归通过。
  临时 CLOSE_TRACE 已删除。独立只读审查未发现新增屏障的具体所有权/无限历史问题。
- 本轮没有再次执行旧 headful 接口删除，保持第九十六切片的恢复状态；独立系统浏览器追加范围仍保留且未实现，
  未连接个人浏览器。跨进程拖放、网络/Profile 决策、剩余能力/主应用/发行验收继续保留，不能以本切片宣布整体完成。
- 最终原生宿主单测 76 项通过；桌面 bin no-default-features cargo check、browser platform boundary、
  派生合同 check 与 diff --check 通过。保留已有 helper/可见性/PDB 警告，不把这些验证扩大为完整默认 smoke 或发行验收通过。

第九十八个切片：系统浏览器独立 Rust 连接基础（2026-09-15）：

- 上一轮仅重述“当前已登录浏览器”的定义，属于 no progress；本轮已重新读取当前工作树与设计，再落实连接层代码。
- 固定核验 Chrome DevTools MCP 1.9.0：默认工具会附加其他标签信息，浏览器内部可自动重连，部分输入存在 DOM
  合成路径。因此不直接接入整套 MCP。进一步核实 Puppeteer 25.10.0 的 channel connect 为只读 discovery 文件后
  WebSocket 连接，采用现有 Rust CDP 传输更简单，不引入 Node/MCP 供应或自有扩展平台。
- 新增 `nomi-browser-engine::attached_browser`：默认 Windows Chrome `DevToolsActivePort` 有界只读、严格 loopback
  解析，仅 Browser.getVersion 握手。独立 owner 不含进程/Profile/目标清理权，不调用旧 CdpBackend/Launched/global
  auto-attach，不暴露 raw handle，不自动重连。macOS/Linux discovery 显式 unavailable，Edge 尚未验收。
- 只读审查发现旧 Connection::shutdown 保留 sink 的风险，已改为断开时取出并丢弃独占连接；单个清理任务持有连接，
  并发/取消后的再次断开等待同一结果，不因旧调用取消提前返回。peer 不响应 Close 时也验证本地 TCP 被释放。
  本地 9 项连接测试通过，最终全部引擎 lib 串行复核 717 通过、10 项真实浏览器
  环境测试 ignored；不把 mock peer/版本字符串或保留 Cookie fixture 扩大为个人 Chrome 授权/登录实测。
- 本轮先验证的 MCP single-session 试验已精确撤回，`nomi-mcp` 无净改动，不保留未使用实现。未下载或安装 MCP 包，
  未连接个人浏览器，未改其设置、读取标签或登录数据库。新增 boundary 规则禁止 attach owner 引入进程/全局目标清理。
- 本切片仅是连接基础，工作台入口、用户标签授权、Agent 工具/run 绑定和真实个人浏览器批准后的验收仍未完成；
  主重构原有跨进程拖放、Profile/egress、剩余 UI/能力/发行验收与 macOS 移交范围均保留。
- 最终 browser platform boundary 的自测与实际扫描、`git diff --check` 通过。没有 renderer 改动，未运行 UI
  测试或桌面安装包验收；没有把新增连接层单测当作整个重构交付证明。
- 完整 lib 并发复跑时，既有 `orphan_recovery_logs_do_not_echo_profile_paths_or_marker_payload` 一次捕获到空日志而
  失败；该测试独立重跑与最终全套串行复核通过。保留这项并发稳定性差异，未修改无关 Profile 测试，未宣称并发套件
  已稳定通过。独立复审确认取消等待后共享关闭回执的修复没有新的具体问题。

第九十九个切片：系统浏览器标签授权与会话应用宿主（2026-09-15）：

- 上一轮已增加独立连接 owner，属于 progress；本轮重新核对工作树后向应用层推进，而非继续重述方案。
- 新增用户-only 标签 inventory 与随机 choice token，授权前核对原 target/URL；过期、关闭、跨连接与 raw target/URL/
  序号请求拒绝，授权元数据不追加其他标签。17 项 engine 连接/标签测试通过，包括在途 inventory 的同步断开屏障。
- 新建独立 `SystemBrowserService`，user/conversation/incarnation 隔离；Snapshot 纯内存，连接关闭投影立即撤下授权。
  两层 grants 有界并按 opaque target 去重。取消请求不丢弃连接清理，迟到连接不发布；失败保留准确 owner 供重试。
  9 项宿主测试通过。并发审查指出并修复了刚连接即断开时，同一请求可能重复尝试失败清理的竞态。
- 接入 DesktopHost/AppServices、会话 `system-browser` API、删除前清理及应用 shutdown；只有 Windows 主应用显式
  注入服务，默认构造不访问个人浏览器。API 使用 instance-owner/local-trust/owned-conversation 校验，修改连接/授权
  通过现有闲置重配置，不能绕过 Agent 运行状态。用户库存不是 Agent Tool，也没有接管或测试面板。
- 发现重配置 helper 的保留任务会吞掉原 HTTP future 取消：connect/grant 增加 caller 取消护卫，并在前置检查和
  work 都验证；DELETE 可取消尚在等待的连接，再取得 preparation gate 做物理结算。底层先 fence I/O 再等待 operation。
  完整 HTTP fixture 已通过：身份/会话隔离、无连接 GET、拒绝 raw endpoint、运行中拒绝配置、正常连接/选择/授权、
  HTTP 取消 POST、连接中 DELETE、删除前失败保留会话/精确重试。fixture 使用 mock browser 与 mock running runtime，
  不写绕过 Running admission 的 SQL，不连接实际 Chrome，不把 mock 当作真实个人浏览器验收。
- 并行定位发现 Wave1 六个单 package registration 仍使用因 local-websearch 插入而错位的数字索引；改为按 package ID
  查找，新增 package/source/mount/capability 一致性回归。Wave1 全部 10 项单测和派生合同 check 通过，不改生成 manifest。
- 本轮未做工作台 UI/Agent Tool 注册，没有向用户宣称系统浏览器已可用；跨连接真实 Tab 的 run 互斥、真实观察/输入、
  用户明确批准的登录页面验收仍待完成。主 ADR 原有嵌入式浏览器、跨进程拖放、Profile/egress、发行/平台交付范围全部保留。
- 最终 Windows desktop bin no-default-features 检查通过；既有 browser_workspace HTTP 集成 2 项回归通过，
  browser platform boundary 自测/扫描与 diff --check 通过。保留既有 native helper/private_interfaces/PDB 警告。
  本轮没有 renderer 改动，未运行 UI 验收、个人 Chrome 原生授权或签名安装包测试。

第一百个切片：可选系统浏览器与 Agent 主文档输入纵向接线（2026-09-15）：

- 上一轮完成连接与用户授权宿主，属于 progress。本轮冲突隔离并行：UI、独立 Catalog、Agent adapter/运行层分工，
  主线实现 engine 驱动、持久 owner verifier 与真实 Chrome 验证；没有并行 Cargo 或交叉覆盖相同文件。
- 新增独立 platform SystemBrowserCommand/Binding/Host/Workspace/Turn，注册 `nomifun.system-browser`、
  `nomi_system_browser@1.0.0` 和同名 Tool。工作台独立选择不会授予 browser.*、nomi_local_websearch 或厂商 search；
  冻结 manifest/config annotation、exact contribution/schema/runtime digest 全部验证，通用 Wave1 host 不可绕过 run。
- 会话 UI 根据实际 Snapshot ceiling 显示入口，运行中锁变更、平台不可用不请求、读状态不连接/枚举；支持明确连接、
  选择授权、断开与取消自己 pending connect。取消先读取精确 incarnation，再仅 DELETE 一次，未知结果只 GET 恢复；
  迟到回包/跨会话/卸载不自动重连或断开用户浏览器。29 项相关 UI 测试通过，含与 native BrowserPanel 同挂载的遮挡验证。
- Runtime 每个 owner 唯一 run，begin 冻结授权和连接代际；首次使用 Tab 原子申请跨连接稳定 target claim。
  claim 到 settle 完成且 terminal 后 finish 才释放。保留在途 invoke/detach job 与 Drop custodian，不因调用方取消丢失清理。
  持久 owner verifier 在 workspace 与 begin 重验，拒绝外用户、非 nomifun source、cron 与 delegated 会话。
- 新 engine 仅对已授权 Tab 使用 Target.attachToTarget；主文档观察复用传输无关 semantic core，不开放 raw evaluate。
  点击/输入/按键/滚动走浏览器 Input；导航或新 observe 使旧引用失效，协议层与隔离世界都拒绝 privileged URL。
  Hover 后重新检查元素，Ctrl+A 后重新检查焦点；failed mouseUp/keyUp 保留义务并在 settle 重试，不把清理异常当成功。
  原生浏览器 UI 快捷键被拒绝；密码/可编辑值脱敏，模型 Tab URL 安全投影。当前 iframe 只计入明确未覆盖范围。
- 独立可见 Chrome 152.0.7977.76 临时 Profile 测试通过：可信鼠标/中文输入/按键/wheel、hover retarget 与 focus trap 拒绝、
  密码遮蔽、导航/观察失效、跨连接同 Tab key 相同、detach/disconnect 不关闭浏览器。测试自己最终回收其进程；
  没有读取用户默认 Chrome Profile、个人标签或登录页面，也没有修改用户浏览器设置。临时 TRACE 全部移除。
- 验证：engine attach 自动测试 20 项通过（真实可见 Chrome 另显式执行通过）；Agent system_browser 8 项与既有
  native_browser 生命周期 8 项通过；App system_browser/binding/host 测试 21 项通过；Wave1 11 项、domain-support 8 项通过；
  system_browser HTTP 1 项综合场景及 browser_workspace HTTP 2 项通过。派生合同使用生成器 write/check，没有手改 generated。
- Windows desktop bin no-default-features 检查、完整 UI typecheck、i18n parity/types、desktop UI boundary、browser boundary
  自测/扫描与 diff 检查通过；保留既有 PDB/helper/可见性警告，以及现有 Arco Transition 的 act 测试警告。
- 本轮是主文档纵向实现，不是整体交付：真实个人 Chrome 原生授权、完整主应用模型到系统浏览器闭环、iframe/文件/
  对话框/拖放矩阵仍待完成，macOS 仍仅移交计划、Linux/Edge 未声明支持。主 ADR 的内嵌 OOPIF drag、Profile/egress、
  后台剩余消费者、主应用与签名安装包验收不因这个新增能力完成部分而缩减。

第一百零一个切片：主应用后端 → 模型 → 真实系统浏览器闭环（2026-09-15）：

- 上一轮已完成独立能力/UI/运行层与真实主文档输入，属于 progress；本轮把这些分层证据连成同一条生产后端链路。
- 新增显式 conformance 构建特性与借用浏览器发现接缝；测试仍使用正式 connection/grant/driver，不 mock 执行。
  默认 Desktop normal/build feature tree 的实际 Cargo 查询确认不含 conformance；它不是用户可配置的 Profile/endpoint，
  也不是生产连接失败 fallback。未增加数据库迁移、个人数据导入、扩展或额外产品 UI。
- `system_browser_agent` 通过真实 DesktopServer HTTP API 创建 provider/Agent/Preset revision/session，选择唯一独立
  nomi_system_browser，连接并授权 fixture Tab，然后走 Nomi Factory/Tool/Run 的 18 次本地脚本模型调用。
  真实 Chrome 先取得测试站点 HttpOnly Cookie；ready/witness 要求已有 Cookie，模型不获该值，证明不是重建无登录页。
- 首轮正常输入/提交、Stop 掉未交付的 click、同 nonce 页面续跑新输入/提交、应用退出中取消未交付 click 均通过。
  用户 chooser 必须实际含未授权 sentinel 页，授权与 Tool Tabs 只含选定页，所有模型请求拒绝出现 sentinel 或 Cookie。
  第二轮仅用新增事件证明本轮 trusted input/click，退出后读取 fixture DOM count=2 与原 nonce/input，防止迟到第三次提交。
- 审查补强了测试本身：取消/退出使用不同 release gate，避免残留 permit 放过迟到响应；保留最初 pipeline 错误而非被
  末尾 witness 断言遮蔽；对用户 chooser 等待标题到达但不重放 mutation。所有 backend/server/browser 清理及 root.close
  成功后才输出 `SYSTEM_BROWSER_MAIN_APP_PASS ... temporary_cleanup=true`。
- 该测试是可见、独立、临时 Profile 的真实 Chrome，不是用户个人浏览器。生产 attach 逻辑与输入执行未换成模拟实现；
  profile/进程仅由外部 fixture owner 在最后关闭，NomiFun shutdown 后仍验证原浏览器和 Tab 存活。
- 尚不能扩大为全部完成：个人浏览器许可/真实登录网站、完整 Tauri GUI 与在途原子输入的主应用取消仍未验收；
  iframe、文件、对话框、拖放、Mac/Edge/Linux 与主 ADR 全部原有交付缺口继续保留。

第一百零二个切片：系统浏览器输入提交边界与真实在途停止（2026-09-15）：

- 上一轮收取桌面编译与边界扫描的最终通过结果，属于 progress。本轮从当前代码重新确认在途鼠标取消仍缺主应用证据，
  以此推进，不扩大产品功能。并行只读审查发现生产取消窗口，随后按文件隔离分工修复驱动与编写主应用测试；Cargo 串行。
- 生产驱动在最后一次 LOCATE、focus 与 viewport/move 等待后、真正发送新效果前检查 cancellation。已按下鼠标的释放
  保持无条件，不把 Stop 当作可以丢弃 mouseUp 的理由。新增 4 项确定性协议回归，全部 attach 自动测试 24 项通过，
  既有可见 Chrome 真实输入回归通过。
- 增加测试专用透明 CDP relay，只延迟真实 Chrome 的 down/up ACK；真实后端、模型协议、Tool/Run 与生产输入未替换。
  独立测试 wrapper 只观察正在 invoke 的 token；Stop 与 shutdown 必须确实取消该 token 后才放行 down ACK。
  同时核对 fixture trusted pointerdown/up/click 与精确 ACK/释放/detach 顺序，证明 terminal/退出不会先于输入结算。
- 在途 Stop 后同页续跑与在途应用退出均通过，原 Tab/浏览器仍由外部 fixture owner 持有；不是终止用户浏览器。
  每项主应用场景 18 次模型请求，隔离运行与最终串行整组 2 项通过，所有自有 fixture/server/browser/临时根清理后才报 PASS。
- 首次整组的第二项在创建 Preset revision 时发生 SQLite database locked；保留这次失败。没有重试或重放该 mutation，
  后续重新创建完整隔离 fixture 后通过，不把通过扩大为数据库并发问题已修复。
- 不触碰用户个人 Chrome/默认 Profile，不增加 GUI、测试面板、接管、DB migration。当前只闭合主文档鼠标在途边界，
  个人授权/登录网站、完整 GUI、键盘/文件/iframe/对话框/拖放与主 ADR 原有交付清单均继续保留。

第一百零三个切片：系统浏览器跨文档观察与原生输入（2026-09-15）：

- 上一轮修复输入取消窗口并取得在途 Stop/shutdown 证据，属于 progress。本轮推进真实网站内部的 iframe，不增加
  frame 选择器、Tool 操作、测试面板或产品模式。按文件隔离并行完成 scoped routes、共享几何迁移、真实 fixture 和审查；
  所有 Cargo 验证在源码停稳后串行执行。
- 既有七操作透明覆盖授权页的同进程嵌套与 OOPIF；语义世界以 frame/session/loader/父链绑定，元素 ref 在观察内隔离。
  固定脚本用于观察/定位，真实 click/type/key/wheel 仍送入根页面 Input。父层遮挡、移动后重定位及 open Shadow DOM
  iframe 深层焦点均检查。数据/特权文档不读取，模型不获得 raw frame/session 或未授权页元数据。
- 共享唯一 ContentQuad/父命中算法，原桌面副本物理移走。frame routes 只配置已 grant 的 page 与其 iframe 后代，
  既不全局 auto-attach、暂停文档，也不继承其他 page/worker。路由有界、清理失败可精确重试，不新建 Profile 或搬数据。
- 审查修复重复 core 的永久 listener/MutationObserver 保留：仅语义专用初始化覆写两个未用 upstream hook；通用 injection
  与真实输入拦截器不变。子文档/父 owner 使用独立可释放 group；未取得 root objectId 的初始化失败也保留并释放精确 group。
- 第一轮真实输入已完成但 release 失败；捕获实际 Chrome 错误定位为关闭 auto-attach 时不允许 filter，修正并补协议断言，
  没有把 fake 协议端通过当作真实关闭成功。最终 38 项 attach 测试包含主文档与新 iframe 真实场景，全部通过。
- 新真实 fixture 包含 open shadow root 内同源 iframe、其同源嵌套与跨站 OOPIF；中文输入、Press、Click、Scroll、
  父遮挡、子导航旧 ref 失效、独立私密 sentinel 不进入观察均验证。4 个 context/Window listener 在重复观察后不增长，
  独立 core WeakRef 在释放 group 与 GC 后清空。release/disconnect 后浏览器及两页仍在，最后由 fixture owner 清理。
- 此轮不宣称个人授权/登录站点或完整 GUI 已验收；系统浏览器 frame 的完整模型链路、其余动态/变换/文件/对话框/拖放、
  主 ADR 原有内嵌 OOPIF drag、网络/Profile、消费者退役与发行范围继续保留。
- 共享核心回归：真实 WebView2 `--frame-input-only` 通过，覆盖输入锁、原生 click/type/key/wheel/select、同源/OOPIF/
  嵌套与变换/透视/缩放，最终 native fixture Profile 清理通过。原生宿主 78 项单测、共享几何 3 项单测与主应用真实 Chrome
  两项场景通过（各 18 次模型请求）。修复原生 smoke 漏引系统浏览器 lifecycle 模块及相应测试期类型依赖，不加生产兜底。
- browser platform boundary 自测与扫描通过；没有 renderer/布局变更，没有以这些证据替代完整默认 native smoke、
  个人 Chrome 授权或签名安装包验收。原有 helper/PDB 警告仍存在。
- 最终 Windows desktop bin no-default-features 检查通过，normal/build feature tree 不含 browser conformance；
  `git diff --check` 通过。保留既有 native helper/private_interfaces/PDB 警告，不宣称无警告或整体交付完成。

第一百零四个切片：最终范围收敛与遗留物理删除（2026-09-16）：

- 按用户最终裁决，把 Windows MVP 收敛为真实会话内嵌 WebView2、顺序式用户/Agent 输入、可选
  `nomi_local_websearch` 与独立 `nomi_system_browser`。不再把 popout/归位、跨 OOPIF HTML drag 成功、
  DevTools、测试工作台、完整系统浏览器文件/拖放矩阵或 Linux/macOS 实现列为 Windows 阻塞项。
- 同一 native frame session 的真实 HTML drag 保留成功合同；不同 session 在任何 mouseDown 前返回
  `UnsupportedAction`，并验证页面没有 pointer/drag/drop 事件。删除失败的 Composition/OLE 实验宿主、
  专用 SDK 绑定与 DirectComposition dev-dependency。
- 物理删除 renderer 的隐藏 DevTools 连点入口、空 `openDevTools`/F12 log bridge、无 producer 的
  `chrome-devtools` MCP 专用兼容分支，以及无消费者的 iframe HTMLViewer/Inspector。
- 删除无消费者的 `browser.inventory.changed` 兼容广播，只保留统一 `sync.resync-required`；Companion
  Browser narration 改为 v2 `operation` + 嵌套 action，并补直接回归。
- 通用 Chromium launcher/raw transport 只在显式 `conformance` feature 对外公开；生产只公开 Native Surface
  所需语义原语、隔离 Headless page 与 attach-only system browser。
- UI 名称收敛为“会话浏览器 / 已登录 Chrome”，本地网页搜索文案不再暴露 Bing、DNS 或浏览器版本实现细节。
  多文件 UI 测试的 Arco portal 污染已修复，组合验证 96/96 通过。
- 站点数据真实 WebView2 smoke 与同页 HTML drag 已在最终逻辑上复跑通过；旧预览 EXE 早于最后的跨 session
  fail-fast 和本轮清理，不能作为最终构建证据，必须在源码冻结后重建。

第一百零五个切片：最终源码统一验证与 Windows 预览（2026-09-16）：

- 默认 native smoke 恢复为稳定单 frame-owner 矩阵，约 18.5s 通过；真实基础输入、跨 session
  drag mouseDown 前拒绝且 `events=[]/drops=[]`、临时 Profile 清理均成功。独立 `--html-drag-only`
  保留 trusted drop/dragend(move)、原始 DataTransfer、取消与目标替换回归。
- 相关 UI 六文件组合 98/98；App realtime lag 3/3；MCP 245/245；engine default/conformance all-targets check、
  desktop no-default-features check、typecheck、i18n 7684 keys、完整 `bun run check`、格式与 diff check 均通过。
  既有 opusic PDB 链接警告与部分 conformance helper dead-code 警告保留，不影响结果。
- 最终嵌入式预览 `dist/browser-preview-20260916-final/NomiFun-Browser-Preview.exe`：
  316025856 bytes，SHA-256 `1FA580E815687ADA97842DF479E24AA74D963A98963A9DC6894CC5EE30EBCDDE`，
  frontend `6a7f95e3-43e3-4bf9-984a-e466fbef10e7`，API 29，channel/identifier 已从 EXE 内核对，NotSigned。
- 首轮 GUI fixture 漏选 Browser capabilities，三次消息由 fixture 以 `Selected Browser tool missing`
  拒绝；这不是模型网络故障。修复后准备阶段使用正式 Preset revision 编译链选中
  `browser.observe/navigate/act`，preparatory factory 只声明 Provider 且永不执行页面，真实动作仍只由桌面
  WebView2 host 执行。新数据集创建成功，不再需要用户手动配置能力。

第一百零六个切片：Windows 自主验收收官（2026-09-16）：

- 最终 EXE 主界面已由用户启动，只做了最初的可见确认；后续验收全部由自动链完成。
  可见页面证据包含真实 native page、F12 无 DevTools 窗口，页面 witness 记录用户点击
  `trusted=true`、计数 1 与中文备注。
- Windows 凭据管理器中的 StepFun 凭据经一次性 stdin 传入，未进入 argv/环境/日志/子工具。
  真实 `step-3.7-flash` 闭环 PASS：原生点击复现错误、修改临时 `app.js`、刷新并用真实点击
  复测 `1,2,3`，terminal 后才解锁。
- 确定性 `--agent-only` PASS：28 次模型调用覆盖自动打开、原生输入/诊断、terminal→unlock、
  同页下一 turn、取消迟到模型回复、取消原生导航、Stop 后用户导航、上传与应用退出清理。
- 独立 `nomi_system_browser` 两项主应用真实临时 Chrome 验收通过，各 25 次模型调用：
  trusted input、已登录 fixture、未授权标签隔离、dialog reply、Stop 拒绝迟到 dialog、续跑、应用退出不关闭
  浏览器及临时资源清理均 PASS。没有读取用户个人 Chrome/Profile/标签。
- build-info 已记录 `gui_retested_after_this_build=true`、`windows_mvp_delivery_complete=true`、
  `not_final_delivery=false`。预览仍 NotSigned，不冒充签名安装包。

## 当前交付清单（2026-09-16 冻结）

已完成的 Windows 代码范围：

1. Conversation 内真实 child WebView2；用户与 Agent 操作同一个页面，不使用 JPEG/screencast/iframe。
2. AgentRunning/UserReady 单一输入权威；Agent 工作时原生用户输入锁定，Stop 仅在原子动作 settle 后解锁。
3. Browser observe/click/type/press/wheel/select、同 session drag、iframe/OOPIF 观察与基础输入、dialog、
   permission、popup、上传、下载、crash/close/shutdown 均接入真实宿主路径。
4. `nomi_local_websearch@1.0.0` 是 Agent 工作台独立可选能力，使用隔离 Headless Search owner，与厂商
   `web.search` 不冲突、不读取会话登录态。
5. `nomi_system_browser@1.0.0` 是独立可选能力；Windows attach-only 连接已运行 Chrome，用户按会话授权标签，
   Agent 只操作授权标签；不启动、不迁移、不清理用户浏览器/Profile。
6. Interactive Browser 使用系统/WebView2 原生网络栈，允许 localhost/LAN/WebSocket/HMR；仅限制顶层无凭据
   HTTP(S) 与特权 scheme。严格公网 DNS/IP 边界只属于后台 Search/Render。
7. 旧 Browser 页面、设置、管理/登录 API、Viewer、Hub/Lane、vault、旧 Headless 交互式所有权和 DevTools
   产品面均已退出生产路径；不做数据库迁移或兼容 fallback。

Windows MVP 代码、构建与验收已完成。macOS v2 移交清单已写入
`docs/continuity/2026-09-16-browser-workspace-v2-macos-handoff.zh.md`；macOS 实现与签名验收由用户转交
Mac 环境继续，Linux 后置。个人 Chrome 未被访问是隐私选择，不是 Windows 代码缺口。

明确能力边界（不是 TODO）：

- 不提供 popout/归位、DevTools/F12、测试步骤/问题/控制台/终端 Browser 面板或 takeover。
- 不同 native frame session 的 HTML drag 明确 unsupported；不合成 dragend、不关闭站点隔离。
- 系统浏览器首版仅 Windows Chrome；Edge、macOS、Linux 与完整文件/拖放矩阵不在本次 Windows MVP。
- 签名发行包需要正式签名凭据；无签名预览可用于功能验收，但不能冒充正式发布包。
